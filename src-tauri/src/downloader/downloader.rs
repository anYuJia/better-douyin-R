//! 下载器实现

use crate::api::types::{DownloadMediaItem, DownloadStatus, DownloadTask, MediaType, VideoInfo};
use crate::api::DouyinClient;
use crate::config::{get_user_agent, AppConfig};
use crate::history::HistoryManager;
use anyhow::{anyhow, Result};
use chrono::Local;
use futures::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, COOKIE, RANGE, REFERER,
    USER_AGENT,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};
use url::Url;

const MAX_FILENAME_CHARS: usize = 180;
const MAX_FILENAME_BYTES: usize = 230;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadQuality {
    Auto,
    Highest,
    H264,
    Smallest,
}

impl DownloadQuality {
    fn from_config(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "highest" => Self::Highest,
            "h264" => Self::H264,
            "smallest" => Self::Smallest,
            _ => Self::Auto,
        }
    }
}

fn bit_rate_metric(bit_rate: &crate::api::types::BitRateInfo) -> i64 {
    if bit_rate.data_size > 0 {
        return bit_rate.data_size;
    }
    if bit_rate.bit_rate > 0 {
        return bit_rate.bit_rate;
    }
    if bit_rate.quality_type > 0 {
        return bit_rate.quality_type as i64;
    }
    if bit_rate.width > 0 && bit_rate.height > 0 {
        return bit_rate.width as i64 * bit_rate.height as i64;
    }
    0
}

#[derive(Debug, Clone)]
struct VideoCandidate {
    url: String,
    metric: i64,
    is_h264: bool,
    is_download_addr: bool,
    is_lowbr: bool,
}

#[derive(Debug, Clone)]
pub struct DownloaderEvent {
    pub name: &'static str,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
struct DownloadRuntime {
    client: reqwest::Client,
    config: AppConfig,
    tasks: Arc<Mutex<Vec<DownloadTask>>>,
    progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    cancel_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pause_tokens: Arc<Mutex<HashMap<String, bool>>>,
    history: Arc<Mutex<HistoryManager>>,
}

/// 下载器
#[derive(Clone)]
pub struct Downloader {
    client: reqwest::Client,
    config: AppConfig,
    tasks: Arc<Mutex<Vec<DownloadTask>>>,
    progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    cancel_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pause_tokens: Arc<Mutex<HashMap<String, bool>>>,
    history: Arc<Mutex<HistoryManager>>,
}

fn build_download_client(config: &AppConfig) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .danger_accept_invalid_certs(false);

    if let Some(proxy) = &config.proxy {
        let proxy = proxy.trim();
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
    }

    Ok(builder.build()?)
}

impl Downloader {
    pub fn new(
        config: AppConfig,
        progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    ) -> Result<Self> {
        let client = build_download_client(&config)?;

        Ok(Self {
            client,
            config,
            tasks: Arc::new(Mutex::new(Vec::new())),
            progress_tx,
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
            pause_tokens: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(HistoryManager::load())),
        })
    }

    pub fn update_config(&mut self, config: AppConfig) -> Result<()> {
        if self.config.proxy != config.proxy {
            self.client = build_download_client(&config)?;
        }
        self.config = config;
        Ok(())
    }

    /// 检查是否已下载（用于去重）
    pub async fn is_downloaded(&self, aweme_id: &str) -> bool {
        let history = self.history.lock().await;
        history.is_downloaded(aweme_id)
            || load_all_downloaded_set(&PathBuf::from(&self.config.download_path))
                .contains(aweme_id)
    }

    /// 添加视频下载任务
    pub async fn add_task(&self, video: &VideoInfo, save_path: Option<PathBuf>) -> Result<String> {
        let base_path = save_path.unwrap_or_else(|| PathBuf::from(&self.config.download_path));

        // 检查是否已下载（去重）
        if self.is_downloaded(&video.aweme_id).await {
            return Err(anyhow!("视频已下载，跳过: {}", video.aweme_id));
        }

        let media_urls = self.collect_download_media_items(video);
        self.add_media_task(
            video.aweme_id.clone(),
            video.desc.clone(),
            video.author.nickname.clone(),
            video.author.uid.clone(),
            video.video.cover.clone(),
            video.media_type.clone(),
            media_urls,
            Some(base_path),
        )
        .await
    }

    /// 添加媒体组下载任务
    #[allow(clippy::too_many_arguments)]
    pub async fn add_media_task(
        &self,
        aweme_id: String,
        title: String,
        author: String,
        author_id: String,
        cover: String,
        media_type: MediaType,
        media_urls: Vec<DownloadMediaItem>,
        save_path: Option<PathBuf>,
    ) -> Result<String> {
        if media_urls.is_empty() {
            return Err(anyhow!("No media URLs"));
        }

        let task_id = uuid::Uuid::new_v4().to_string();
        let base_path = save_path.unwrap_or_else(|| PathBuf::from(&self.config.download_path));
        if self.is_downloaded(&aweme_id).await {
            return Err(anyhow!("视频已下载，跳过: {}", aweme_id));
        }
        let author_dir = build_output_dir(
            &self.config,
            &base_path,
            &author,
            media_type_name(&media_type),
        );
        let filename = generate_filename_with_config(
            &self.config,
            &title,
            &aweme_id,
            &author,
            media_type_name(&media_type),
        );

        let task = DownloadTask {
            id: task_id.clone(),
            aweme_id,
            url: media_urls
                .first()
                .map(|item| item.url.clone())
                .unwrap_or_default(),
            media_urls: media_urls.clone(),
            title,
            author,
            author_id,
            cover,
            save_path: author_dir.to_string_lossy().to_string(),
            filename,
            media_type,
            total_files: media_urls.len() as u32,
            completed_files: 0,
            status: DownloadStatus::Pending,
            progress: 0.0,
            total_size: 0,
            downloaded_size: 0,
            error_msg: None,
            create_time: Local::now().timestamp(),
            complete_time: None,
            image_urls: None,
        };

        self.tasks.lock().await.push(task);
        Ok(task_id)
    }

    fn select_video_download_url(&self, video: &VideoInfo) -> Option<String> {
        select_video_url(
            video,
            DownloadQuality::from_config(&self.config.download_quality),
        )
    }

    fn collect_download_media_items(&self, video: &VideoInfo) -> Vec<DownloadMediaItem> {
        let mut items = Vec::new();

        if let Some(urls) = &video.live_photo_urls {
            for url in urls {
                if !url.trim().is_empty() {
                    items.push(DownloadMediaItem {
                        r#type: "live_photo".to_string(),
                        url: url.clone(),
                    });
                }
            }
        }

        if let Some(urls) = &video.image_urls {
            for url in urls {
                if !url.trim().is_empty() {
                    items.push(DownloadMediaItem {
                        r#type: "image".to_string(),
                        url: url.clone(),
                    });
                }
            }
        }

        if items.is_empty() {
            if let Some(url) = self.select_video_download_url(video) {
                items.push(DownloadMediaItem {
                    r#type: "video".to_string(),
                    url,
                });
            } else if let Some(url) = DouyinClient::get_no_watermark_url(video) {
                items.push(DownloadMediaItem {
                    r#type: "video".to_string(),
                    url,
                });
            }
        }

        items
    }

    /// 开始下载
    pub async fn start_download(&self, task_id: &str) -> Result<()> {
        let task_id_owned = task_id.to_string();
        let runtime = DownloadRuntime {
            client: self.client.clone(),
            config: self.config.clone(),
            tasks: self.tasks.clone(),
            progress_tx: self.progress_tx.clone(),
            cancel_tokens: self.cancel_tokens.clone(),
            pause_tokens: self.pause_tokens.clone(),
            history: self.history.clone(),
        };

        runtime
            .cancel_tokens
            .lock()
            .await
            .insert(task_id_owned.clone(), false);
        runtime
            .pause_tokens
            .lock()
            .await
            .insert(task_id_owned.clone(), false);

        {
            let mut tasks_lock = runtime.tasks.lock().await;
            if let Some(task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                task.status = DownloadStatus::Downloading;
            }
        }

        tokio::spawn(async move {
            if let Err(error) =
                Self::download_media_group(runtime.clone(), task_id_owned.clone()).await
            {
                let is_cancelled = error.to_string().to_lowercase().contains("cancelled")
                    || error.to_string().to_lowercase().contains("canceled")
                    || *runtime
                        .cancel_tokens
                        .lock()
                        .await
                        .get(&task_id_owned)
                        .unwrap_or(&false);
                if !is_cancelled {
                    log::error!("Download error: {}", error);
                }

                {
                    let mut tasks_lock = runtime.tasks.lock().await;
                    if let Some(task) = tasks_lock.iter_mut().find(|t| t.id == task_id_owned) {
                        task.status = if is_cancelled {
                            DownloadStatus::Cancelled
                        } else {
                            DownloadStatus::Failed
                        };
                        task.error_msg = Some(error.to_string());
                    }
                }

                if is_cancelled {
                    emit_event(
                        &runtime.progress_tx,
                        "download-cancelled",
                        serde_json::json!({
                            "task_id": task_id_owned,
                            "message": "下载已取消"
                        }),
                    )
                    .await;
                } else {
                    emit_event(
                        &runtime.progress_tx,
                        "download-failed",
                        serde_json::json!({
                            "task_id": task_id_owned,
                            "error": format!("下载失败: {}", error)
                        }),
                    )
                    .await;

                    emit_event(
                        &runtime.progress_tx,
                        "download-error",
                        serde_json::json!({
                            "task_id": task_id_owned,
                            "message": format!("下载失败: {}", error)
                        }),
                    )
                    .await;
                }
            }
        });

        Ok(())
    }

    async fn download_media_group(runtime: DownloadRuntime, task_id: String) -> Result<()> {
        let task = {
            let tasks_lock = runtime.tasks.lock().await;
            tasks_lock
                .iter()
                .find(|t| t.id == task_id)
                .cloned()
                .ok_or_else(|| anyhow!("Task not found"))?
        };

        let media_count = task.media_urls.len() as u32;
        if media_count == 0 {
            return Err(anyhow!("No media URLs"));
        }

        let display_name = truncate_chars(&task.title, 8);
        let save_dir = PathBuf::from(&task.save_path);
        let headers = build_download_headers(&runtime.config);

        tokio::fs::create_dir_all(&save_dir).await?;

        emit_event(
            &runtime.progress_tx,
            "download-started",
            serde_json::json!({
                "task_id": task.id,
                "desc": task.title,
                "display_name": display_name,
                "type": "single_video",
                "aweme_id": task.aweme_id,
                "media_type": media_type_name(&task.media_type),
                "media_count": media_count,
                "save_path": task.save_path
            }),
        )
        .await;

        emit_event(
            &runtime.progress_tx,
            "download-progress",
            serde_json::json!({
                "task_id": task.id,
                "progress": 0,
                "completed": 0,
                "total": media_count,
                "status": "starting",
                "desc": task.title,
                "display_name": display_name,
                "save_path": task.save_path,
                "media_type": media_type_name(&task.media_type)
            }),
        )
        .await;

        let mut downloaded_files = Vec::new();
        let mut total_downloaded_size = 0u64;

        for (index, media) in task.media_urls.iter().enumerate() {
            if *runtime
                .cancel_tokens
                .lock()
                .await
                .get(&task_id)
                .unwrap_or(&false)
            {
                return Err(anyhow!("Download cancelled"));
            }

            wait_if_paused(&runtime.pause_tokens, &runtime.cancel_tokens, &task_id).await?;

            let file_type_display = media_type_display(media.r#type.as_str());
            emit_event(
                &runtime.progress_tx,
                "download-log",
                serde_json::json!({
                    "task_id": task.id,
                    "display_name": display_name,
                    "message": format!("正在下载第 {}/{} 个文件 ({})", index + 1, media_count, file_type_display),
                    "timestamp": Local::now().format("%H:%M:%S").to_string()
                }),
            )
            .await;

            let response = runtime
                .client
                .get(&media.url)
                .headers(headers.clone())
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(anyhow!("HTTP error: {}", response.status()));
            }

            let response_size = response.content_length().unwrap_or(0);
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let extension = media_extension(media.r#type.as_str(), &media.url, content_type);
            let (file_path, mut file) = create_unique_output_file(
                &save_dir,
                &task.filename,
                index,
                task.media_urls.len(),
                &extension,
            )
            .await?;
            let mut file_downloaded_size = 0u64;
            let mut stream = response.bytes_stream();
            let file_started_at = Instant::now();
            let mut last_emit_at = Instant::now();
            let mut last_emit_progress = (index as f32 / media_count as f32) * 100.0;

            downloaded_files.push(file_path.clone());

            while let Some(chunk_result) = stream.next().await {
                if *runtime
                    .cancel_tokens
                    .lock()
                    .await
                    .get(&task_id)
                    .unwrap_or(&false)
                {
                    let _ = tokio::fs::remove_file(&file_path).await;
                    for downloaded_file in &downloaded_files {
                        let _ = tokio::fs::remove_file(downloaded_file).await;
                    }
                    return Err(anyhow!("Download cancelled"));
                }

                wait_if_paused(&runtime.pause_tokens, &runtime.cancel_tokens, &task_id).await?;

                let chunk = chunk_result?;
                file.write_all(&chunk).await?;
                file_downloaded_size += chunk.len() as u64;
                total_downloaded_size += chunk.len() as u64;

                let elapsed = file_started_at.elapsed().as_secs_f64().max(0.001);
                let file_progress = if response_size > 0 {
                    ((file_downloaded_size as f64 / response_size as f64) * 100.0) as f32
                } else {
                    0.0
                }
                .clamp(0.0, 100.0);
                let overall_progress =
                    ((index as f32 + file_progress / 100.0) / media_count as f32) * 100.0;
                let speed_bps = (file_downloaded_size as f64 / elapsed) as u64;
                let eta_seconds = if response_size > 0 && speed_bps > 0 {
                    Some((response_size.saturating_sub(file_downloaded_size) / speed_bps) as u64)
                } else {
                    None
                };

                {
                    let mut tasks_lock = runtime.tasks.lock().await;
                    if let Some(current_task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                        current_task.progress = overall_progress;
                        current_task.downloaded_size = total_downloaded_size;
                        current_task.total_size = current_task.total_size.max(response_size);
                    }
                }

                let should_emit = last_emit_at.elapsed().as_millis() >= 500
                    || (overall_progress - last_emit_progress).abs() >= 1.0
                    || (response_size > 0 && file_downloaded_size >= response_size);

                if should_emit {
                    emit_event(
                        &runtime.progress_tx,
                        "download-progress",
                        serde_json::json!({
                            "task_id": task.id,
                            "progress": overall_progress,
                            "completed": index,
                            "total": media_count,
                            "status": "downloading",
                            "desc": task.title,
                            "display_name": display_name,
                            "file_index": index + 1,
                            "file_total": media_count,
                            "file_progress": file_progress,
                            "bytes_downloaded": file_downloaded_size,
                            "bytes_total": response_size,
                            "speed_bps": speed_bps,
                            "eta_seconds": eta_seconds,
                            "file_type": media.r#type,
                            "file_type_display": file_type_display,
                            "save_path": task.save_path,
                            "file_path": file_path.to_string_lossy().to_string(),
                            "media_type": media_type_name(&task.media_type)
                        }),
                    )
                    .await;
                    last_emit_at = Instant::now();
                    last_emit_progress = overall_progress;
                }
            }

            {
                let mut tasks_lock = runtime.tasks.lock().await;
                if let Some(current_task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                    current_task.completed_files = (index + 1) as u32;
                    current_task.progress = (((index + 1) as f32) / media_count as f32) * 100.0;
                    current_task.downloaded_size = total_downloaded_size;
                }
            }

            emit_event(
                &runtime.progress_tx,
                "download-progress",
                serde_json::json!({
                    "task_id": task.id,
                    "progress": (((index + 1) as f32) / media_count as f32) * 100.0,
                    "completed": index + 1,
                    "total": media_count,
                    "status": "downloading",
                    "desc": task.title,
                    "display_name": display_name,
                    "file_index": index + 1,
                    "file_total": media_count,
                    "file_progress": 100,
                    "bytes_downloaded": file_downloaded_size,
                    "bytes_total": response_size,
                    "speed_bps": 0,
                    "eta_seconds": 0,
                    "file_type": media.r#type,
                    "file_type_display": file_type_display,
                    "save_path": task.save_path,
                    "file_path": file_path.to_string_lossy().to_string(),
                    "media_type": media_type_name(&task.media_type)
                }),
            )
            .await;

            emit_event(
                &runtime.progress_tx,
                "download-log",
                serde_json::json!({
                    "task_id": task.id,
                    "display_name": display_name,
                    "message": format!(
                        "✅ 第 {}/{} 个文件下载成功 ({})",
                        index + 1,
                        media_count,
                        file_path.file_name().and_then(|value| value.to_str()).unwrap_or_default()
                    ),
                    "timestamp": Local::now().format("%H:%M:%S").to_string()
                }),
            )
            .await;
        }

        {
            let mut tasks_lock = runtime.tasks.lock().await;
            if let Some(current_task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                current_task.status = DownloadStatus::Completed;
                current_task.progress = 100.0;
                current_task.complete_time = Some(Local::now().timestamp());
                current_task.completed_files = current_task.total_files;
                current_task.downloaded_size = total_downloaded_size;
                current_task.total_size = total_downloaded_size;
            }
        }

        if let Some(first_file) = downloaded_files.first() {
            let mut history_lock = runtime.history.lock().await;
            let _ = history_lock.add(crate::api::DownloadHistory {
                aweme_id: task.aweme_id.clone(),
                title: task.title.clone(),
                author: task.author.clone(),
                author_id: task.author_id.clone(),
                cover: task.cover.clone(),
                file_path: first_file.to_string_lossy().to_string(),
                media_type: media_type_name(&task.media_type).to_string(),
                file_size: total_downloaded_size,
                create_time: Local::now().timestamp(),
            });
        }

        // 写入本地隐藏去重记录
        let _ = record_downloaded(&save_dir, &task.aweme_id).await;

        emit_event(
            &runtime.progress_tx,
            "download-completed",
            serde_json::json!({
                "task_id": task.id,
                "message": format!("下载成功: {}", task.title),
                "aweme_id": task.aweme_id,
                "media_type": media_type_name(&task.media_type),
                "file_count": media_count,
                "display_name": display_name,
                "save_path": task.save_path,
                "file_path": downloaded_files.first().map(|p| p.to_string_lossy().to_string()),
                "total_size": total_downloaded_size
            }),
        )
        .await;

        Ok(())
    }

    pub async fn get_tasks(&self) -> Vec<DownloadTask> {
        self.tasks.lock().await.clone()
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<()> {
        let mut tokens = self.cancel_tokens.lock().await;
        tokens.insert(task_id.to_string(), true);

        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = DownloadStatus::Cancelled;
        }

        Ok(())
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<()> {
        let mut tokens = self.pause_tokens.lock().await;
        tokens.insert(task_id.to_string(), true);

        let mut progress = 0.0;
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = DownloadStatus::Paused;
            progress = task.progress;
        }
        drop(tasks);

        emit_event(
            &self.progress_tx,
            "download-progress",
            serde_json::json!({
                "task_id": task_id,
                "progress": progress,
                "status": "paused",
                "speed_bps": 0
            }),
        )
        .await;

        Ok(())
    }

    pub async fn resume_task(&self, task_id: &str) -> Result<()> {
        let mut tokens = self.pause_tokens.lock().await;
        tokens.insert(task_id.to_string(), false);

        let mut progress = 0.0;
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            if task.status == DownloadStatus::Paused {
                task.status = DownloadStatus::Downloading;
            }
            progress = task.progress;
        }
        drop(tasks);

        emit_event(
            &self.progress_tx,
            "download-progress",
            serde_json::json!({
                "task_id": task_id,
                "progress": progress,
                "status": "downloading"
            }),
        )
        .await;

        Ok(())
    }

    pub async fn remove_task(&self, task_id: &str) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|t| t.id != task_id);

        let mut tokens = self.cancel_tokens.lock().await;
        tokens.remove(task_id);

        let mut pause_tokens = self.pause_tokens.lock().await;
        pause_tokens.remove(task_id);

        Ok(())
    }

    /// 发送批量下载开始事件
    pub async fn emit_batch_started(&self, task_id: &str, nickname: &str, total_videos: usize) {
        emit_event(
            &self.progress_tx,
            "batch-download-started",
            serde_json::json!({
                "task_id": task_id,
                "nickname": nickname,
                "total_videos": total_videos,
                "message": format!("开始下载 {} 个视频", total_videos)
            }),
        )
        .await;
    }

    /// 边获取边下载（流式下载）
    pub async fn start_streaming_download(
        &self,
        client: DouyinClient,
        sec_uid: String,
        batch_task_id: String,
        _nickname: String,
        estimated_total: usize,
    ) -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        // 初始化取消和暂停标记
        self.cancel_tokens
            .lock()
            .await
            .insert(batch_task_id.clone(), false);
        self.pause_tokens
            .lock()
            .await
            .insert(batch_task_id.clone(), false);

        // 创建视频队列
        let (video_tx, video_rx) = tokio::sync::mpsc::channel::<VideoInfo>(32);

        // 状态跟踪
        let batch_started_at = Instant::now();
        let total_discovered = Arc::new(AtomicUsize::new(0));
        let completed_count = Arc::new(AtomicUsize::new(0));
        let skipped_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));

        // 克隆变量
        let cancel_tokens = self.cancel_tokens.clone();
        let pause_tokens = self.pause_tokens.clone();
        let progress_tx = self.progress_tx.clone();
        let history = self.history.clone();
        let config = self.config.clone();
        let http_client = self.client.clone();

        let batch_id_fetch = batch_task_id.clone();
        let sec_uid_clone = sec_uid.clone();

        // === 获取任务：分页获取视频并发送到队列 ===
        let fetch_handle = {
            let video_tx = video_tx;
            let total_discovered = total_discovered.clone();
            let cancel_tokens = cancel_tokens.clone();
            let pause_tokens = pause_tokens.clone();
            let batch_id = batch_id_fetch;

            tokio::spawn(async move {
                let mut cursor: i64 = 0;
                let mut has_more = true;
                let page_size = 20u32;

                while has_more {
                    // 检查取消
                    if *cancel_tokens.lock().await.get(&batch_id).unwrap_or(&false) {
                        log::info!("Fetch task cancelled");
                        break;
                    }

                    // 检查暂停
                    loop {
                        let is_paused = *pause_tokens.lock().await.get(&batch_id).unwrap_or(&false);
                        let is_cancelled =
                            *cancel_tokens.lock().await.get(&batch_id).unwrap_or(&false);
                        if is_cancelled || !is_paused {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }

                    // 获取一页视频
                    match client
                        .get_user_videos(&sec_uid_clone, cursor, page_size)
                        .await
                    {
                        Ok((videos, next_cursor, more)) => {
                            has_more = more;
                            cursor = next_cursor;

                            let video_count = videos.len();

                            for video in videos {
                                // 检查取消
                                if *cancel_tokens.lock().await.get(&batch_id).unwrap_or(&false) {
                                    break;
                                }

                                total_discovered.fetch_add(1, AtomicOrdering::SeqCst);

                                // 发送到下载队列（非阻塞）
                                if video_tx.send(video).await.is_err() {
                                    log::info!("Video channel closed");
                                    has_more = false;
                                    break;
                                }
                            }

                            if video_count == 0 && !more {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to fetch videos: {}", e);
                            break;
                        }
                    }

                    // 短暂延迟避免请求过快
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }

                // 标记获取完成
                drop(video_tx);
                log::info!(
                    "Fetch task completed, discovered {} videos",
                    total_discovered.load(AtomicOrdering::SeqCst)
                );
            })
        };

        // === 下载任务：从队列取出视频并下载 ===
        let download_handle = {
            let mut video_rx = video_rx;
            let completed = completed_count.clone();
            let skipped = skipped_count.clone();
            let failed = failed_count.clone();
            let cancel_tokens = cancel_tokens.clone();
            let pause_tokens = pause_tokens.clone();
            let progress_tx = progress_tx.clone();
            let history = history.clone();
            let config = config.clone();
            let http_client = http_client.clone();
            let total_discovered = total_discovered.clone();
            let batch_id = batch_task_id.clone();
            let estimated = estimated_total;

            tokio::spawn(async move {
                // 并发控制
                let max_concurrent = config.max_concurrent.clamp(1, 10);
                let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
                let mut download_handles = Vec::new();

                loop {
                    // 检查取消
                    if *cancel_tokens.lock().await.get(&batch_id).unwrap_or(&false) {
                        break;
                    }

                    // 检查暂停
                    loop {
                        let is_paused = *pause_tokens.lock().await.get(&batch_id).unwrap_or(&false);
                        let is_cancelled =
                            *cancel_tokens.lock().await.get(&batch_id).unwrap_or(&false);
                        if is_cancelled || !is_paused {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }

                    // 尝试从队列获取视频（带超时，避免永久阻塞）
                    let video = tokio::select! {
                        result = video_rx.recv() => {
                            match result {
                                Some(v) => v,
                                None => break, // 通道关闭
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_secs(30)) => {
                            // 超时，检查是否还在获取
                            continue;
                        }
                    };

                    // 获取信号量许可
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };

                    // 检查是否已下载
                    {
                        let is_in_history = history.lock().await.is_downloaded(&video.aweme_id);
                        let downloaded =
                            load_all_downloaded_set(&PathBuf::from(&config.download_path));
                        if is_in_history || downloaded.contains(&video.aweme_id) {
                            skipped.fetch_add(1, AtomicOrdering::SeqCst);
                            let current = completed.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                            let total =
                                total_discovered.load(AtomicOrdering::SeqCst).max(estimated);

                            // 发送进度（跳过的不显示消息）
                            emit_event(
                                &progress_tx,
                                "download-progress",
                                serde_json::json!({
                                    "task_id": batch_id,
                                    "overall_progress": (current as f32 / total as f32 * 100.0) as u32,
                                    "current_downloaded": current,
                                    "total_videos": total,
                                    "processed": current,
                                    "skipped": skipped.load(AtomicOrdering::SeqCst),
                                    "remaining": total.saturating_sub(current),
                                    "eta_seconds": estimate_batch_eta(current, total, batch_started_at),
                                    "status": "downloading"
                                }),
                            ).await;

                            drop(permit);
                            continue;
                        }
                    }

                    // 克隆变量
                    let history = history.clone();
                    let cancel_tokens = cancel_tokens.clone();
                    let pause_tokens = pause_tokens.clone();
                    let progress_tx = progress_tx.clone();
                    let batch_id = batch_id.clone();
                    let completed = completed.clone();
                    let failed = failed.clone();
                    let total_discovered = total_discovered.clone();
                    let estimated = estimated;
                    let batch_started_at = batch_started_at;
                    let config = config.clone();
                    let http_client = http_client.clone();

                    let aweme_id = video.aweme_id.clone();
                    let _display_name = truncate_chars(&video.desc, 8);
                    let start_time = Instant::now();

                    // 启动下载任务
                    let handle = tokio::spawn(async move {
                        let result = Self::download_single_video(
                            http_client,
                            config,
                            video,
                            history,
                            cancel_tokens.clone(),
                            pause_tokens.clone(),
                            batch_id.clone(),
                            progress_tx.clone(),
                        )
                        .await;

                        drop(permit);

                        let elapsed = start_time.elapsed();

                        match result {
                            Ok(_) => {
                                let current = completed.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                                let total =
                                    total_discovered.load(AtomicOrdering::SeqCst).max(estimated);

                                emit_event(
                                    &progress_tx,
                                    "download-progress",
                                    serde_json::json!({
                                        "task_id": batch_id,
                                        "overall_progress": (current as f32 / total as f32 * 100.0) as u32,
                                        "current_downloaded": current,
                                        "total_videos": total,
                                        "processed": current,
                                        "remaining": total.saturating_sub(current),
                                        "eta_seconds": estimate_batch_eta(current, total, batch_started_at),
                                        "elapsed_seconds": elapsed.as_secs(),
                                        "status": "downloading"
                                    }),
                                ).await;
                            }
                            Err(e) => {
                                failed.fetch_add(1, AtomicOrdering::SeqCst);
                                log::error!("Download error for {}: {}", aweme_id, e);

                                let current = completed.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                                let total =
                                    total_discovered.load(AtomicOrdering::SeqCst).max(estimated);

                                emit_event(
                                    &progress_tx,
                                    "download-progress",
                                    serde_json::json!({
                                        "task_id": batch_id,
                                        "overall_progress": (current as f32 / total as f32 * 100.0) as u32,
                                        "current_downloaded": current,
                                        "total_videos": total,
                                        "processed": current,
                                        "failed": failed.load(AtomicOrdering::SeqCst),
                                    "remaining": total.saturating_sub(current),
                                    "eta_seconds": estimate_batch_eta(current, total, batch_started_at),
                                    "status": "downloading",
                                        "message": format!("下载失败: {}", aweme_id)
                                    }),
                                ).await;
                            }
                        }
                    });

                    download_handles.push(handle);
                }

                // 等待所有下载完成
                futures::future::join_all(download_handles).await;
                log::info!("Download task completed");
            })
        };

        // 等待两个任务完成
        let (fetch_result, download_result) = tokio::join!(fetch_handle, download_handle);

        if let Err(e) = fetch_result {
            log::error!("Fetch handle error: {}", e);
        }
        if let Err(e) = download_result {
            log::error!("Download handle error: {}", e);
        }

        // 发送完成事件
        let was_cancelled = *self
            .cancel_tokens
            .lock()
            .await
            .get(&batch_task_id)
            .unwrap_or(&false);
        let final_completed = completed_count.load(AtomicOrdering::SeqCst);
        let final_skipped = skipped_count.load(AtomicOrdering::SeqCst);
        let final_failed = failed_count.load(AtomicOrdering::SeqCst);
        let final_total = total_discovered
            .load(AtomicOrdering::SeqCst)
            .max(estimated_total);

        if was_cancelled {
            emit_event(
                &self.progress_tx,
                "batch-download-cancelled",
                serde_json::json!({
                    "task_id": batch_task_id,
                    "total_videos": final_total,
                    "completed": final_completed,
                    "processed": final_completed,
                    "skipped": final_skipped,
                    "failed": final_failed,
                    "remaining": final_total.saturating_sub(final_completed),
                    "message": format!("下载已取消，已完成 {} 个视频", final_completed)
                }),
            )
            .await;
        } else {
            emit_event(
                &self.progress_tx,
                "batch-download-completed",
                serde_json::json!({
                    "task_id": batch_task_id,
                    "total_videos": final_total,
                    "completed": final_completed,
                    "processed": final_completed,
                    "succeeded": final_completed.saturating_sub(final_skipped + final_failed),
                    "skipped": final_skipped,
                    "failed": final_failed,
                    "message": format!("下载完成: {} 个视频, {} 个跳过", final_completed, final_skipped)
                }),
            ).await;
        }

        // 清理
        self.cancel_tokens.lock().await.remove(&batch_task_id);
        self.pause_tokens.lock().await.remove(&batch_task_id);

        Ok(())
    }

    /// 下载单个视频
    #[allow(clippy::too_many_arguments)]
    async fn download_single_video(
        client: reqwest::Client,
        config: AppConfig,
        video: VideoInfo,
        history: Arc<Mutex<HistoryManager>>,
        cancel_tokens: Arc<Mutex<std::collections::HashMap<String, bool>>>,
        pause_tokens: Arc<Mutex<std::collections::HashMap<String, bool>>>,
        batch_task_id: String,
        progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    ) -> Result<()> {
        // 收集媒体URL
        let media_urls = Self::collect_media_items(&video, &config);
        if media_urls.is_empty() {
            return Err(anyhow!("No media URLs"));
        }

        let base_path = PathBuf::from(&config.download_path);
        let author_dir = build_output_dir(
            &config,
            &base_path,
            &video.author.nickname,
            media_type_name(&video.media_type),
        );
        let filename = generate_filename_with_config(
            &config,
            &video.desc,
            &video.aweme_id,
            &video.author.nickname,
            media_type_name(&video.media_type),
        );
        let display_name = truncate_chars(&video.desc, 20);

        tokio::fs::create_dir_all(&author_dir).await?;

        let headers = build_download_headers(&config);
        let mut downloaded_files = Vec::new();
        let mut total_size = 0u64;
        let total_files = media_urls.len();

        // 发送开始下载事件
        emit_event(
            &progress_tx,
            "current-video-progress",
            serde_json::json!({
                "task_id": batch_task_id,
                "aweme_id": video.aweme_id,
                "name": display_name,
                "progress": 0,
                "speed_bps": 0
            }),
        )
        .await;

        let start_time = Instant::now();

        for (index, media) in media_urls.iter().enumerate() {
            // 检查取消
            if *cancel_tokens
                .lock()
                .await
                .get(&batch_task_id)
                .unwrap_or(&false)
            {
                for f in &downloaded_files {
                    let _ = tokio::fs::remove_file(f).await;
                }
                return Err(anyhow!("Cancelled"));
            }

            // 检查暂停
            loop {
                let is_paused = *pause_tokens
                    .lock()
                    .await
                    .get(&batch_task_id)
                    .unwrap_or(&false);
                let is_cancelled = *cancel_tokens
                    .lock()
                    .await
                    .get(&batch_task_id)
                    .unwrap_or(&false);
                if is_cancelled || !is_paused {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            let response = client
                .get(&media.url)
                .headers(headers.clone())
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(anyhow!("HTTP error: {}", response.status()));
            }

            let content_length = response.content_length().unwrap_or(0);
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let extension = media_extension(media.r#type.as_str(), &media.url, content_type);
            let (file_path, mut file) =
                create_unique_output_file(&author_dir, &filename, index, total_files, &extension)
                    .await?;
            let mut stream = response.bytes_stream();
            let mut file_size = 0u64;
            let mut last_emit = Instant::now();

            while let Some(chunk_result) = stream.next().await {
                // 检查取消
                if *cancel_tokens
                    .lock()
                    .await
                    .get(&batch_task_id)
                    .unwrap_or(&false)
                {
                    let _ = tokio::fs::remove_file(&file_path).await;
                    for f in &downloaded_files {
                        let _ = tokio::fs::remove_file(f).await;
                    }
                    return Err(anyhow!("Cancelled"));
                }

                // 检查暂停
                loop {
                    let is_paused = *pause_tokens
                        .lock()
                        .await
                        .get(&batch_task_id)
                        .unwrap_or(&false);
                    let is_cancelled = *cancel_tokens
                        .lock()
                        .await
                        .get(&batch_task_id)
                        .unwrap_or(&false);
                    if is_cancelled || !is_paused {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                let chunk = chunk_result?;
                file.write_all(&chunk).await?;
                file_size += chunk.len() as u64;
                total_size += chunk.len() as u64;

                // 每300ms发送一次进度
                if last_emit.elapsed().as_millis() >= 300 {
                    let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
                    let speed_bps = (total_size as f64 / elapsed) as u64;
                    let progress = if content_length > 0 {
                        ((file_size as f64 / content_length as f64) * 100.0) as u32
                    } else {
                        0
                    };

                    emit_event(
                        &progress_tx,
                        "current-video-progress",
                        serde_json::json!({
                            "task_id": batch_task_id,
                            "aweme_id": video.aweme_id,
                            "name": display_name,
                            "progress": progress,
                            "speed_bps": speed_bps
                        }),
                    )
                    .await;

                    last_emit = Instant::now();
                }
            }

            downloaded_files.push(file_path);
        }

        // 发送完成事件
        let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
        let speed_bps = (total_size as f64 / elapsed) as u64;

        emit_event(
            &progress_tx,
            "current-video-progress",
            serde_json::json!({
                "task_id": batch_task_id,
                "aweme_id": video.aweme_id,
                "name": display_name,
                "progress": 100,
                "speed_bps": speed_bps
            }),
        )
        .await;

        // 保存到历史
        if let Some(first_file) = downloaded_files.first() {
            let mut history_lock = history.lock().await;
            let _ = history_lock.add(crate::api::DownloadHistory {
                aweme_id: video.aweme_id.clone(),
                title: video.desc.clone(),
                author: video.author.nickname.clone(),
                author_id: video.author.uid.clone(),
                cover: video.video.cover.clone(),
                file_path: first_file.to_string_lossy().to_string(),
                media_type: media_type_name(&video.media_type).to_string(),
                file_size: total_size,
                create_time: Local::now().timestamp(),
            });
        }

        // 写入本地隐藏去重记录
        let _ = record_downloaded(&author_dir, &video.aweme_id).await;

        Ok(())
    }

    /// 收集媒体项
    fn collect_media_items(video: &VideoInfo, config: &AppConfig) -> Vec<DownloadMediaItem> {
        let mut items = Vec::new();

        // Live Photo
        if let Some(urls) = &video.live_photo_urls {
            for url in urls {
                if !url.trim().is_empty() {
                    items.push(DownloadMediaItem {
                        r#type: "live_photo".to_string(),
                        url: url.clone(),
                    });
                }
            }
        }

        // 图片
        if !items.is_empty() {
            return items;
        }

        if let Some(urls) = &video.image_urls {
            for url in urls {
                if !url.trim().is_empty() {
                    items.push(DownloadMediaItem {
                        r#type: "image".to_string(),
                        url: url.clone(),
                    });
                }
            }
        }

        if !items.is_empty() {
            return items;
        }

        // 视频
        let quality = DownloadQuality::from_config(&config.download_quality);
        if let Some(url) = select_video_url(video, quality) {
            items.push(DownloadMediaItem {
                r#type: "video".to_string(),
                url,
            });
        } else if let Some(url) = DouyinClient::get_no_watermark_url(video) {
            items.push(DownloadMediaItem {
                r#type: "video".to_string(),
                url,
            });
        }

        items
    }

    /// 批量并发下载视频列表
    pub async fn start_batch_download(
        &self,
        videos: Vec<VideoInfo>,
        batch_task_id: String,
        nickname: String,
    ) -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let total_videos = videos.len();
        if total_videos == 0 {
            return Err(anyhow!("No videos to download"));
        }

        // 初始化取消和暂停标记
        self.cancel_tokens
            .lock()
            .await
            .insert(batch_task_id.clone(), false);
        self.pause_tokens
            .lock()
            .await
            .insert(batch_task_id.clone(), false);

        // 发送批量下载开始事件
        emit_event(
            &self.progress_tx,
            "batch-download-started",
            serde_json::json!({
                "task_id": batch_task_id,
                "nickname": nickname,
                "total_videos": total_videos,
                "message": format!("开始并发下载 {} 个视频", total_videos)
            }),
        )
        .await;

        // 并发控制：使用配置的最大并发数
        let batch_started_at = Instant::now();
        let max_concurrent = self.config.max_concurrent.clamp(1, 10);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let completed_count = Arc::new(AtomicUsize::new(0));
        let skipped_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));

        let mut download_handles = Vec::new();

        for video in videos {
            // 检查取消
            if *self
                .cancel_tokens
                .lock()
                .await
                .get(&batch_task_id)
                .unwrap_or(&false)
            {
                break;
            }

            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };

            // 检查是否已下载（去重）
            let is_in_history = self.history.lock().await.is_downloaded(&video.aweme_id);
            let downloaded = load_all_downloaded_set(&PathBuf::from(&self.config.download_path));
            if is_in_history || downloaded.contains(&video.aweme_id) {
                skipped_count.fetch_add(1, AtomicOrdering::SeqCst);
                let current = completed_count.fetch_add(1, AtomicOrdering::SeqCst) + 1;

                // 发送跳过事件
                emit_event(
                    &self.progress_tx,
                    "download-progress",
                    serde_json::json!({
                                        "task_id": batch_task_id,
                                        "overall_progress": (current as f32 / total_videos as f32 * 100.0) as u32,
                                        "current_downloaded": current,
                                        "total_videos": total_videos,
                                        "processed": current,
                                        "skipped": skipped_count.load(AtomicOrdering::SeqCst),
                            "remaining": total_videos.saturating_sub(current),
                            "eta_seconds": estimate_batch_eta(current, total_videos, batch_started_at),
                            "status": "downloading",
                                        "message": format!("跳过已下载: {}", video.desc.chars().take(15).collect::<String>())
                                    }),
                ).await;

                drop(permit);
                continue;
            }

            // 克隆必要的数据
            let client = self.client.clone();
            let config = self.config.clone();
            let tasks = self.tasks.clone();
            let history = self.history.clone();
            let cancel_tokens = self.cancel_tokens.clone();
            let pause_tokens = self.pause_tokens.clone();
            let progress_tx = self.progress_tx.clone();
            let batch_id = batch_task_id.clone();
            let completed = completed_count.clone();
            let failed = failed_count.clone();
            let aweme_id = video.aweme_id.clone();

            // 收集媒体URL
            let media_urls = self.collect_download_media_items(&video);
            if media_urls.is_empty() {
                failed_count.fetch_add(1, AtomicOrdering::SeqCst);
                let current = completed_count.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                emit_event(
                    &self.progress_tx,
                    "download-progress",
                    serde_json::json!({
                                        "task_id": batch_task_id,
                                        "overall_progress": (current as f32 / total_videos as f32 * 100.0) as u32,
                                        "current_downloaded": current,
                                        "total_videos": total_videos,
                                        "processed": current,
                                        "failed": failed_count.load(AtomicOrdering::SeqCst),
                            "remaining": total_videos.saturating_sub(current),
                            "eta_seconds": estimate_batch_eta(current, total_videos, batch_started_at),
                            "status": "downloading",
                                        "message": format!("无可下载媒体: {}", video.desc.chars().take(15).collect::<String>())
                                    }),
                ).await;
                drop(permit);
                continue;
            }

            let base_path = PathBuf::from(&self.config.download_path);
            let author_dir = build_output_dir(
                &self.config,
                &base_path,
                &video.author.nickname,
                media_type_name(&video.media_type),
            );
            let filename = generate_filename_with_config(
                &self.config,
                &video.desc,
                &video.aweme_id,
                &video.author.nickname,
                media_type_name(&video.media_type),
            );

            let task = DownloadTask {
                id: uuid::Uuid::new_v4().to_string(),
                aweme_id: video.aweme_id.clone(),
                url: media_urls
                    .first()
                    .map(|item| item.url.clone())
                    .unwrap_or_default(),
                media_urls: media_urls.clone(),
                title: video.desc.clone(),
                author: video.author.nickname.clone(),
                author_id: video.author.uid.clone(),
                cover: video.video.cover.clone(),
                save_path: author_dir.to_string_lossy().to_string(),
                filename,
                media_type: video.media_type.clone(),
                total_files: media_urls.len() as u32,
                completed_files: 0,
                status: DownloadStatus::Pending,
                progress: 0.0,
                total_size: 0,
                downloaded_size: 0,
                error_msg: None,
                create_time: Local::now().timestamp(),
                complete_time: None,
                image_urls: None,
            };

            let task_id = task.id.clone();
            let display_name = truncate_chars(&task.title, 8);
            let display_name_for_log = display_name.clone();

            self.tasks.lock().await.push(task);

            // 启动并发下载任务
            let handle = tokio::spawn(async move {
                let result = Self::download_single_with_progress(
                    client,
                    config,
                    tasks,
                    task_id,
                    progress_tx.clone(),
                    history,
                    cancel_tokens.clone(),
                    pause_tokens.clone(),
                    batch_id.clone(),
                    total_videos,
                    completed.clone(),
                    display_name,
                )
                .await;

                drop(permit);

                match result {
                    Ok(_) => {
                        let current = completed.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                        emit_event(
                            &progress_tx,
                            "download-progress",
                            serde_json::json!({
                                        "task_id": batch_id,
                                        "overall_progress": (current as f32 / total_videos as f32 * 100.0) as u32,
                                        "current_downloaded": current,
                                        "total_videos": total_videos,
                                        "processed": current,
                                    "remaining": total_videos.saturating_sub(current),
                                    "eta_seconds": estimate_batch_eta(current, total_videos, batch_started_at),
                                    "status": "downloading",
                                        "message": format!("完成 {}/{}: {}", current, total_videos, display_name_for_log)
                                    }),
                        ).await;
                    }
                    Err(e) => {
                        failed.fetch_add(1, AtomicOrdering::SeqCst);
                        log::error!("Download error for {}: {}", aweme_id, e);
                        let current = completed.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                        emit_event(
                            &progress_tx,
                            "download-log",
                            serde_json::json!({
                                "task_id": batch_id,
                                "message": format!("下载失败 {}: {}", display_name_for_log, e),
                                "level": "error"
                            }),
                        )
                        .await;
                        emit_event(
                            &progress_tx,
                            "download-progress",
                            serde_json::json!({
                                        "task_id": batch_id,
                                        "overall_progress": (current as f32 / total_videos as f32 * 100.0) as u32,
                                        "current_downloaded": current,
                                        "total_videos": total_videos,
                                        "processed": current,
                                        "failed": failed.load(AtomicOrdering::SeqCst),
                                    "remaining": total_videos.saturating_sub(current),
                                    "eta_seconds": estimate_batch_eta(current, total_videos, batch_started_at),
                                    "status": "downloading",
                                        "message": format!("失败 {}/{}: {}", current, total_videos, display_name_for_log)
                                    }),
                        ).await;
                    }
                }
            });

            download_handles.push(handle);
        }

        // 等待所有下载完成
        futures::future::join_all(download_handles).await;

        // 检查是否被取消
        let was_cancelled = *self
            .cancel_tokens
            .lock()
            .await
            .get(&batch_task_id)
            .unwrap_or(&false);

        let final_completed = completed_count.load(AtomicOrdering::SeqCst);
        let final_skipped = skipped_count.load(AtomicOrdering::SeqCst);
        let final_failed = failed_count.load(AtomicOrdering::SeqCst);

        if was_cancelled {
            emit_event(
                &self.progress_tx,
                "batch-download-cancelled",
                serde_json::json!({
                    "task_id": batch_task_id,
                    "total_videos": total_videos,
                    "completed": final_completed,
                    "processed": final_completed,
                    "skipped": final_skipped,
                    "failed": final_failed,
                    "remaining": total_videos.saturating_sub(final_completed),
                    "message": format!("下载已取消，已完成 {} 个视频", final_completed)
                }),
            )
            .await;
        } else {
            emit_event(
                &self.progress_tx,
                "batch-download-completed",
                serde_json::json!({
                    "task_id": batch_task_id,
                    "total_videos": total_videos,
                    "completed": final_completed,
                    "processed": final_completed,
                    "succeeded": final_completed.saturating_sub(final_skipped + final_failed),
                    "skipped": final_skipped,
                    "failed": final_failed,
                    "message": format!("下载完成: {} 个视频, {} 个跳过", final_completed, final_skipped)
                }),
            ).await;
        }

        // 清理
        self.cancel_tokens.lock().await.remove(&batch_task_id);
        self.pause_tokens.lock().await.remove(&batch_task_id);

        Ok(())
    }

    /// 单个视频下载（带批量进度）
    #[allow(clippy::too_many_arguments)]
    async fn download_single_with_progress(
        client: reqwest::Client,
        config: AppConfig,
        tasks: Arc<Mutex<Vec<DownloadTask>>>,
        task_id: String,
        progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
        history: Arc<Mutex<HistoryManager>>,
        cancel_tokens: Arc<Mutex<std::collections::HashMap<String, bool>>>,
        pause_tokens: Arc<Mutex<std::collections::HashMap<String, bool>>>,
        batch_task_id: String,
        _total_videos: usize,
        _completed_count: Arc<std::sync::atomic::AtomicUsize>,
        _display_name: String,
    ) -> Result<()> {
        let task = {
            let tasks_lock = tasks.lock().await;
            tasks_lock
                .iter()
                .find(|t| t.id == task_id)
                .cloned()
                .ok_or_else(|| anyhow!("Task not found"))?
        };

        let media_count = task.media_urls.len() as u32;
        if media_count == 0 {
            return Err(anyhow!("No media URLs"));
        }

        let save_dir = PathBuf::from(&task.save_path);
        let headers = build_download_headers(&config);

        tokio::fs::create_dir_all(&save_dir).await?;

        let mut downloaded_files = Vec::new();
        let mut total_downloaded_size = 0u64;
        let display_name = truncate_chars(&task.title, 20);
        let start_time = Instant::now();

        // 发送开始下载事件
        emit_event(
            &progress_tx,
            "current-video-progress",
            serde_json::json!({
                "task_id": batch_task_id,
                "aweme_id": task.aweme_id,
                "name": display_name,
                "progress": 0,
                "speed_bps": 0
            }),
        )
        .await;

        for (index, media) in task.media_urls.iter().enumerate() {
            // 检查取消
            if *cancel_tokens
                .lock()
                .await
                .get(&batch_task_id)
                .unwrap_or(&false)
            {
                for f in &downloaded_files {
                    let _ = tokio::fs::remove_file(f).await;
                }
                return Err(anyhow!("Download cancelled"));
            }

            // 检查暂停
            loop {
                let is_paused = *pause_tokens
                    .lock()
                    .await
                    .get(&batch_task_id)
                    .unwrap_or(&false);
                let is_cancelled = *cancel_tokens
                    .lock()
                    .await
                    .get(&batch_task_id)
                    .unwrap_or(&false);
                if is_cancelled || !is_paused {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            let response = client
                .get(&media.url)
                .headers(headers.clone())
                .send()
                .await?;

            if !response.status().is_success() {
                return Err(anyhow!("HTTP error: {}", response.status()));
            }

            let response_size = response.content_length().unwrap_or(0);
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let extension = media_extension(media.r#type.as_str(), &media.url, content_type);
            let (file_path, mut file) = create_unique_output_file(
                &save_dir,
                &task.filename,
                index,
                task.media_urls.len(),
                &extension,
            )
            .await?;
            let mut file_downloaded_size = 0u64;
            let mut stream = response.bytes_stream();
            let mut last_emit = Instant::now();

            downloaded_files.push(file_path.clone());

            while let Some(chunk_result) = stream.next().await {
                // 检查取消
                if *cancel_tokens
                    .lock()
                    .await
                    .get(&batch_task_id)
                    .unwrap_or(&false)
                {
                    let _ = tokio::fs::remove_file(&file_path).await;
                    for f in &downloaded_files {
                        let _ = tokio::fs::remove_file(f).await;
                    }
                    return Err(anyhow!("Download cancelled"));
                }

                // 检查暂停
                loop {
                    let is_paused = *pause_tokens
                        .lock()
                        .await
                        .get(&batch_task_id)
                        .unwrap_or(&false);
                    let is_cancelled = *cancel_tokens
                        .lock()
                        .await
                        .get(&batch_task_id)
                        .unwrap_or(&false);
                    if is_cancelled || !is_paused {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                let chunk = chunk_result?;
                file.write_all(&chunk).await?;
                file_downloaded_size += chunk.len() as u64;
                total_downloaded_size += chunk.len() as u64;

                // 每300ms发送一次进度
                if last_emit.elapsed().as_millis() >= 300 {
                    let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
                    let speed_bps = (total_downloaded_size as f64 / elapsed) as u64;
                    let progress = if response_size > 0 {
                        ((file_downloaded_size as f64 / response_size as f64) * 100.0) as u32
                    } else {
                        0
                    };

                    emit_event(
                        &progress_tx,
                        "current-video-progress",
                        serde_json::json!({
                            "task_id": batch_task_id,
                            "aweme_id": task.aweme_id,
                            "name": display_name,
                            "progress": progress,
                            "speed_bps": speed_bps
                        }),
                    )
                    .await;

                    last_emit = Instant::now();
                }
            }

            {
                let mut tasks_lock = tasks.lock().await;
                if let Some(current_task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                    current_task.completed_files = (index + 1) as u32;
                    current_task.downloaded_size = total_downloaded_size;
                }
            }
        }

        // 更新任务状态
        {
            let mut tasks_lock = tasks.lock().await;
            if let Some(current_task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                current_task.status = DownloadStatus::Completed;
                current_task.progress = 100.0;
                current_task.complete_time = Some(Local::now().timestamp());
                current_task.completed_files = current_task.total_files;
            }
        }

        // 保存到历史
        if let Some(first_file) = downloaded_files.first() {
            let mut history_lock = history.lock().await;
            let _ = history_lock.add(crate::api::DownloadHistory {
                aweme_id: task.aweme_id.clone(),
                title: task.title.clone(),
                author: task.author.clone(),
                author_id: task.author_id.clone(),
                cover: task.cover.clone(),
                file_path: first_file.to_string_lossy().to_string(),
                media_type: media_type_name(&task.media_type).to_string(),
                file_size: total_downloaded_size,
                create_time: Local::now().timestamp(),
            });
        }

        // 写入本地隐藏去重记录
        let _ = record_downloaded(&save_dir, &task.aweme_id).await;

        // 发送完成事件
        let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
        let speed_bps = (total_downloaded_size as f64 / elapsed) as u64;
        emit_event(
            &progress_tx,
            "current-video-progress",
            serde_json::json!({
                "task_id": batch_task_id,
                "aweme_id": task.aweme_id,
                "name": display_name,
                "progress": 100,
                "speed_bps": speed_bps
            }),
        )
        .await;

        Ok(())
    }
}

/// 从作者目录的隐藏文件 `.downloaded` 加载已下载的 aweme_id 集合
async fn load_downloaded_set(dir: &Path) -> HashSet<String> {
    let record_path = dir.join(".downloaded");
    if !record_path.exists() {
        return HashSet::new();
    }
    match tokio::fs::read_to_string(&record_path).await {
        Ok(content) => parse_downloaded_set(&content),
        Err(_) => HashSet::new(),
    }
}

/// 将 aweme_id 写入作者目录的隐藏文件 `.downloaded`
async fn record_downloaded(dir: &Path, aweme_id: &str) -> Result<()> {
    let record_path = dir.join(".downloaded");
    tokio::fs::create_dir_all(dir).await?;

    let aweme_id = aweme_id.trim();
    if aweme_id.is_empty() {
        return Ok(());
    }

    let mut set = load_downloaded_set(dir).await;
    if set.insert(aweme_id.to_string()) {
        let needs_leading_newline = tokio::fs::metadata(&record_path)
            .await
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
        let line = if needs_leading_newline {
            format!("\n{}", aweme_id)
        } else {
            aweme_id.to_string()
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&record_path)
            .await?;
        file.write_all(line.as_bytes()).await?;
    }
    Ok(())
}

fn parse_downloaded_set(content: &str) -> HashSet<String> {
    if let Ok(set) = serde_json::from_str::<HashSet<String>>(content) {
        return set;
    }

    let mut set = HashSet::new();
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Ok(line_set) = serde_json::from_str::<HashSet<String>>(line) {
            set.extend(line_set);
        } else {
            set.insert(line.to_string());
        }
    }
    set
}

fn load_all_downloaded_set(root: &Path) -> HashSet<String> {
    let mut result = HashSet::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some(".downloaded") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    result.extend(parse_downloaded_set(&content));
                }
            }
        }
    }

    result
}

fn template_value(
    token: &str,
    title: &str,
    aweme_id: &str,
    author: &str,
    media_type: &str,
) -> String {
    match token {
        "title" => title.to_string(),
        "aweme_id" => aweme_id.to_string(),
        "author" => author.to_string(),
        "date" => Local::now().format("%Y%m%d").to_string(),
        "time" => Local::now().format("%H%M%S").to_string(),
        "media_type" => media_type.to_string(),
        _ => String::new(),
    }
}

fn render_template(
    template: &str,
    title: &str,
    aweme_id: &str,
    author: &str,
    media_type: &str,
) -> String {
    let mut output = String::new();
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }

        let mut token = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == '}' {
                closed = true;
                break;
            }
            token.push(next);
        }

        if closed {
            output.push_str(&template_value(&token, title, aweme_id, author, media_type));
        } else {
            output.push('{');
            output.push_str(&token);
        }
    }

    output
}

fn truncate_filename_text(
    value: &str,
    default: &str,
    max_chars: usize,
    max_bytes: usize,
    protected_suffix: &str,
) -> String {
    let mut text = value.trim_matches([' ', '.', '_']).to_string();
    if text.is_empty() {
        text = default.to_string();
    }

    if !protected_suffix.is_empty() && text.ends_with(protected_suffix) {
        let prefix_len = text.len().saturating_sub(protected_suffix.len());
        let mut prefix: String = text[..prefix_len]
            .chars()
            .take(max_chars.saturating_sub(protected_suffix.chars().count()))
            .collect();
        while !prefix.is_empty() && format!("{}{}", prefix, protected_suffix).len() > max_bytes {
            prefix.pop();
        }
        text = format!("{}{}", prefix, protected_suffix);
    } else {
        text = text.chars().take(max_chars).collect();
        while !text.is_empty() && text.len() > max_bytes {
            text.pop();
        }
    }

    let text = text.trim_matches([' ', '.', '_']).to_string();
    if text.is_empty() {
        default.to_string()
    } else {
        text
    }
}

fn collect_video_candidates(video: &VideoInfo) -> Vec<VideoCandidate> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut push_candidate = |url: Option<String>,
                              metric: i64,
                              is_h264: bool,
                              is_download_addr: bool,
                              is_lowbr: bool| {
        let Some(url) = url else {
            return;
        };
        if url.trim().is_empty() || !seen.insert(url.clone()) {
            return;
        }
        candidates.push(VideoCandidate {
            url,
            metric,
            is_h264,
            is_download_addr,
            is_lowbr,
        });
    };

    push_candidate(video.video.download_addr.clone(), 0, false, true, false);
    push_candidate(video.video.play_addr_h264.clone(), 0, true, false, false);
    push_candidate(video.video.play_addr_lowbr.clone(), 1, true, false, true);

    if let Some(bit_rates) = &video.video.bit_rate {
        for bit_rate in bit_rates {
            let metric = bit_rate_metric(bit_rate);
            let h264_metric = if metric > 0 { metric + 1 } else { 0 };

            push_candidate(
                bit_rate.play_addr_h264.clone(),
                h264_metric,
                true,
                false,
                false,
            );
            push_candidate(
                bit_rate.play_addr.clone(),
                metric,
                !bit_rate.is_h265,
                false,
                false,
            );
        }
    }

    push_candidate(video.video.preview_addr.clone(), 0, false, false, false);
    push_candidate(Some(video.video.play_addr.clone()), 0, false, false, false);

    candidates
}

async fn emit_event(
    sender: &Option<mpsc::Sender<DownloaderEvent>>,
    name: &'static str,
    payload: serde_json::Value,
) {
    if let Some(tx) = sender {
        let _ = tx.send(DownloaderEvent { name, payload }).await;
    }
}

async fn wait_if_paused(
    pause_tokens: &Arc<Mutex<std::collections::HashMap<String, bool>>>,
    cancel_tokens: &Arc<Mutex<std::collections::HashMap<String, bool>>>,
    task_id: &str,
) -> Result<()> {
    loop {
        if *cancel_tokens.lock().await.get(task_id).unwrap_or(&false) {
            return Err(anyhow!("Download cancelled"));
        }
        if !*pause_tokens.lock().await.get(task_id).unwrap_or(&false) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn estimate_batch_eta(
    processed_count: usize,
    total_count: usize,
    started_at: Instant,
) -> Option<u64> {
    if processed_count == 0 || total_count == 0 || processed_count >= total_count {
        return None;
    }

    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
    let remaining = total_count.saturating_sub(processed_count) as f64;
    Some(
        ((remaining * elapsed) / processed_count as f64)
            .ceil()
            .max(1.0) as u64,
    )
}

fn build_download_headers(config: &AppConfig) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        ACCEPT_ENCODING,
        HeaderValue::from_static("identity;q=1, *;q=0"),
    );
    headers.insert(RANGE, HeaderValue::from_static("bytes=0-"));
    headers.insert(REFERER, HeaderValue::from_static("https://www.douyin.com/"));
    headers.insert(USER_AGENT, HeaderValue::from_static(get_user_agent()));

    if !config.cookie.trim().is_empty() {
        if let Ok(cookie) = HeaderValue::from_str(&config.cookie) {
            headers.insert(COOKIE, cookie);
        }
    }

    headers
}

fn media_type_name(media_type: &MediaType) -> &'static str {
    match media_type {
        MediaType::Video => "video",
        MediaType::Image => "image",
        MediaType::LivePhoto => "live_photo",
        MediaType::Mixed => "mixed",
        MediaType::Audio => "audio",
    }
}

fn media_type_display(media_type: &str) -> &'static str {
    match media_type {
        "video" => "视频",
        "image" => "图片",
        "live_photo" => "Live Photo",
        "audio" => "音频",
        _ => "媒体",
    }
}

fn media_extension(media_type: &str, url: &str, content_type: Option<&str>) -> String {
    let normalized_content_type = content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let from_content_type = match normalized_content_type.as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        "image/heic" => Some("heic"),
        "image/heif" => Some("heif"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" => Some("m4a"),
        "audio/aac" => Some("aac"),
        "audio/wav" => Some("wav"),
        "audio/ogg" => Some("ogg"),
        _ => None,
    };
    if let Some(extension) = from_content_type {
        return extension.to_string();
    }

    if let Some(extension) = extension_from_url(url) {
        return extension.to_string();
    }

    match media_type {
        "video" | "live_photo" => "mp4".to_string(),
        "audio" => "mp3".to_string(),
        _ => "jpg".to_string(),
    }
}

fn extension_from_url(url: &str) -> Option<&'static str> {
    let path = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.path_segments()?.next_back().map(str::to_string))
        .or_else(|| {
            url.split('?')
                .next()
                .and_then(|path| path.rsplit('/').next())
                .map(str::to_string)
        })?;
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();

    match extension.as_str() {
        "mp4" => Some("mp4"),
        "mov" => Some("mov"),
        "m4v" => Some("m4v"),
        "webm" => Some("webm"),
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "avif" => Some("avif"),
        "heic" => Some("heic"),
        "heif" => Some("heif"),
        "mp3" => Some("mp3"),
        "m4a" => Some("m4a"),
        "aac" => Some("aac"),
        "wav" => Some("wav"),
        "ogg" => Some("ogg"),
        _ => None,
    }
}

fn unique_output_path(
    save_dir: &Path,
    base_name: &str,
    index: usize,
    total: usize,
    extension: &str,
) -> PathBuf {
    let stem = if total <= 1 {
        sanitize_filename(base_name)
    } else {
        sanitize_filename(&format!("{}_{:02}", base_name, index + 1))
    };
    let extension = sanitize_extension(extension);
    let mut path = save_dir.join(format!("{}.{}", stem, extension));

    if path.exists() {
        let timestamp = Local::now().timestamp();
        path = save_dir.join(format!("{}_{}.{}", stem, timestamp, extension));
    }

    path
}

async fn create_unique_output_file(
    save_dir: &Path,
    base_name: &str,
    index: usize,
    total: usize,
    extension: &str,
) -> Result<(PathBuf, File)> {
    let extension = sanitize_extension(extension);
    for attempt in 0..1000 {
        let candidate = if attempt == 0 {
            unique_output_path(save_dir, base_name, index, total, &extension)
        } else {
            let stem = if total <= 1 {
                sanitize_filename(base_name)
            } else {
                sanitize_filename(&format!("{}_{:02}", base_name, index + 1))
            };
            save_dir.join(format!("{}_{}.{}", stem, attempt + 1, extension))
        };

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow!("Unable to reserve unique output file name"))
}

fn sanitize_extension(extension: &str) -> String {
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        && !extension.is_empty()
        && extension.len() <= 8
    {
        extension
    } else {
        "bin".to_string()
    }
}

/// 清理文件名
fn sanitize_filename(name: &str) -> String {
    let invalid_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let mut result = name.to_string();
    for c in invalid_chars {
        result = result.replace(c, "_");
    }
    truncate_filename_text(
        &result,
        "未命名作品",
        MAX_FILENAME_CHARS,
        MAX_FILENAME_BYTES,
        "",
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn generate_filename_with_config(
    config: &AppConfig,
    desc: &str,
    aweme_id: &str,
    author: &str,
    media_type: &str,
) -> String {
    let normalized_title = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized_aweme_id = aweme_id.trim();
    let default = if normalized_aweme_id.is_empty() {
        "未命名作品".to_string()
    } else {
        format!("未命名作品_{}", normalized_aweme_id)
    };
    let template = if config.filename_template.trim().is_empty() {
        "{title}_{aweme_id}"
    } else {
        config.filename_template.trim()
    };
    let rendered = render_template(
        template,
        if normalized_title.is_empty() {
            "未命名作品"
        } else {
            &normalized_title
        },
        normalized_aweme_id,
        author.trim(),
        media_type,
    );
    let sanitized = sanitize_filename(&rendered);
    let protected_suffix = if normalized_aweme_id.is_empty() {
        String::new()
    } else if sanitized.ends_with(normalized_aweme_id) {
        normalized_aweme_id.to_string()
    } else {
        format!("_{}", normalized_aweme_id)
    };
    let candidate = if !protected_suffix.is_empty() && !sanitized.ends_with(&protected_suffix) {
        format!("{}{}", sanitized, protected_suffix)
    } else {
        sanitized
    };

    truncate_filename_text(
        &candidate,
        &default,
        MAX_FILENAME_CHARS,
        MAX_FILENAME_BYTES,
        &protected_suffix,
    )
}

fn build_output_dir(
    config: &AppConfig,
    base_path: &Path,
    author: &str,
    media_type: &str,
) -> PathBuf {
    if !config.auto_create_folder {
        return base_path.to_path_buf();
    }

    let template = if config.folder_name_template.trim().is_empty() {
        "{author}"
    } else {
        config.folder_name_template.trim()
    };
    let rendered = render_template(template, "", "", author.trim(), media_type);
    let folder = sanitize_filename(&rendered);
    if folder.is_empty() {
        base_path.join("未知作者")
    } else {
        base_path.join(folder)
    }
}

/// 选择视频下载URL
fn select_video_url(video: &VideoInfo, quality: DownloadQuality) -> Option<String> {
    let candidates = collect_video_candidates(video);

    if candidates.is_empty() {
        return None;
    }

    // 单次遍历收集所有候选特征
    let download_addr = candidates.iter().find(|c| c.is_download_addr);
    let h264_best = candidates
        .iter()
        .filter(|c| c.is_h264 && !c.is_lowbr)
        .max_by_key(|c| c.metric);
    let highest_metric = candidates
        .iter()
        .filter(|c| c.metric > 0 && !c.is_download_addr && !c.is_lowbr)
        .max_by_key(|c| c.metric);
    let lowbr = candidates.iter().find(|c| c.is_lowbr);
    let smallest_metric = candidates
        .iter()
        .filter(|c| c.metric > 0)
        .min_by_key(|c| c.metric);
    let first = candidates.first();

    let selected = match quality {
        DownloadQuality::Auto => download_addr.or(h264_best).or(highest_metric).or(first),
        DownloadQuality::Highest => highest_metric.or(download_addr).or(h264_best).or(first),
        DownloadQuality::H264 => h264_best.or(download_addr).or(highest_metric).or(first),
        DownloadQuality::Smallest => lowbr.or(smallest_metric).or(h264_best).or(first),
    };

    selected.map(|c| c.url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::BitRateInfo;

    fn video_with_quality_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "play-default".to_string();
        video.video.download_addr = Some("download-default".to_string());
        video.video.play_addr_h264 = Some("top-h264".to_string());
        video.video.play_addr_lowbr = Some("lowbr".to_string());
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                play_addr: Some("bitrate-small".to_string()),
                play_addr_h264: Some("bitrate-small-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                play_addr: Some("bitrate-high".to_string()),
                play_addr_h264: Some("bitrate-high-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    #[test]
    fn auto_keeps_download_addr_preference() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::Auto).as_deref(),
            Some("download-default")
        );
    }

    #[test]
    fn highest_prefers_measured_quality_candidate() {
        let video = video_with_quality_candidates();
        let selected = select_video_url(&video, DownloadQuality::Highest);
        assert!(matches!(
            selected.as_deref(),
            Some("bitrate-high" | "bitrate-high-h264")
        ));
    }

    #[test]
    fn h264_prefers_best_h264_candidate() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::H264).as_deref(),
            Some("bitrate-high-h264")
        );
    }

    #[test]
    fn smallest_prefers_lowbr_candidate() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::Smallest).as_deref(),
            Some("lowbr")
        );
    }

    #[test]
    fn downloader_uses_updated_quality_config() {
        let mut config = AppConfig {
            download_quality: "auto".to_string(),
            ..Default::default()
        };
        let mut downloader = Downloader::new(config.clone(), None).expect("downloader");
        let video = video_with_quality_candidates();

        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("download-default")
        );

        config.download_quality = "smallest".to_string();
        downloader.update_config(config).expect("update config");

        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("lowbr")
        );
    }

    #[test]
    fn downloaded_set_parser_accepts_json_and_append_lines() {
        let parsed = parse_downloaded_set("[\"1001\",\"1002\"]\n1003\n1004");

        assert!(parsed.contains("1001"));
        assert!(parsed.contains("1002"));
        assert!(parsed.contains("1003"));
        assert!(parsed.contains("1004"));
    }

    #[test]
    fn filename_template_keeps_full_title_and_aweme_id() {
        let config = AppConfig {
            filename_template: "{title}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let filename = generate_filename_with_config(
            &config,
            "这是 一个 完整 标题 第二段 文案",
            aweme_id,
            "作者",
            "video",
        );

        assert_eq!(
            filename,
            format!("这是 一个 完整 标题 第二段 文案_{}", aweme_id)
        );
    }

    #[test]
    fn long_filename_template_preserves_aweme_id_suffix() {
        let config = AppConfig {
            filename_template: "{title}_{aweme_id}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let filename = generate_filename_with_config(
            &config,
            &"很长标题".repeat(80),
            aweme_id,
            "作者",
            "image",
        );

        assert!(filename.ends_with(aweme_id));
        assert!(filename.len() <= MAX_FILENAME_BYTES);
    }

    #[test]
    fn long_filename_template_keeps_more_safe_title_text() {
        let config = AppConfig {
            filename_template: "{title}_{aweme_id}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let filename = generate_filename_with_config(
            &config,
            &"abcdefghijklmnopqrstuvwxyz".repeat(8),
            aweme_id,
            "作者",
            "video",
        );

        assert!(filename.starts_with(&"abcdefghijklmnopqrstuvwxyz".repeat(6)));
        assert!(filename.ends_with(aweme_id));
        assert!(filename.len() <= MAX_FILENAME_BYTES);
    }

    #[test]
    fn output_dir_respects_folder_template_toggle() {
        let base = PathBuf::from("/tmp/douyin");
        let mut config = AppConfig {
            folder_name_template: "{author}_{media_type}".to_string(),
            auto_create_folder: true,
            ..Default::default()
        };

        assert_eq!(
            build_output_dir(&config, &base, "作者/名", "video"),
            base.join("作者_名_video")
        );

        config.auto_create_folder = false;
        assert_eq!(build_output_dir(&config, &base, "作者", "video"), base);
    }

    #[tokio::test]
    async fn unique_output_file_does_not_overwrite_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("douyin-downloader-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let existing = dir.join("clip.mp4");
        std::fs::write(&existing, b"existing").expect("write existing file");

        let (created_path, created_file) = create_unique_output_file(&dir, "clip", 0, 1, "mp4")
            .await
            .expect("create unique file");
        drop(created_file);

        assert_ne!(created_path, existing);
        assert!(created_path.exists());
        assert_eq!(
            std::fs::read(&existing).expect("read existing"),
            b"existing"
        );

        std::fs::remove_dir_all(&dir).expect("cleanup test dir");
    }

    #[test]
    fn media_extension_prefers_content_type_then_url_then_media_type() {
        assert_eq!(
            media_extension("image", "https://example.test/file", Some("image/webp")),
            "webp"
        );
        assert_eq!(
            media_extension("image", "https://example.test/file.png?x=1", None),
            "png"
        );
        assert_eq!(
            media_extension("live_photo", "https://example.test/play", None),
            "mp4"
        );
    }
}
