//! 下载器实现

use crate::api::types::{DownloadMediaItem, DownloadStatus, DownloadTask, MediaType, VideoInfo};
use crate::api::DouyinClient;
use crate::config::AppConfig;
use crate::history::HistoryManager;
use crate::media_utils::is_dash_video_only_url;
use anyhow::{anyhow, Result};
use chrono::Local;
use futures::StreamExt;
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};

use super::downloaded_cache::{add_to_downloaded_cache, ensure_downloaded_cache, record_downloaded};
use super::events::{emit_event, estimate_batch_eta, wait_if_paused};
use super::filename::{build_output_dir, create_unique_output_file, generate_filename_with_config, media_extension, media_type_display, media_type_name, truncate_chars};
use super::http::{build_download_client, build_download_headers};
use super::media_group::{collect_media_items, download_media_group};
use super::media_request::request_media_with_fallback;
use super::quality::{ordered_video_urls, select_video_url, DownloadQuality};




#[derive(Debug, Clone)]
pub struct DownloaderEvent {
    pub name: &'static str,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct DownloadRuntime {
    pub(crate) client: reqwest::Client,
    pub(crate) config: AppConfig,
    pub(crate) tasks: Arc<Mutex<Vec<DownloadTask>>>,
    pub(crate) progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    pub(crate) cancel_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) pause_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) history: Arc<Mutex<HistoryManager>>,
    pub(crate) downloaded_cache: Arc<RwLock<HashSet<String>>>,
    pub(crate) record_write_lock: Arc<Mutex<()>>,
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
    downloaded_cache: Arc<RwLock<HashSet<String>>>,
    downloaded_cache_loaded: Arc<AtomicBool>,
    record_write_lock: Arc<Mutex<()>>,
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
            downloaded_cache: Arc::new(RwLock::new(HashSet::new())),
            downloaded_cache_loaded: Arc::new(AtomicBool::new(false)),
            record_write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn update_config(&mut self, config: AppConfig) -> Result<()> {
        let download_path_changed = self.config.download_path != config.download_path;
        if self.config.proxy != config.proxy {
            self.client = build_download_client(&config)?;
        }
        self.config = config;
        if download_path_changed {
            if let Ok(mut cache) = self.downloaded_cache.write() {
                cache.clear();
            }
            self.downloaded_cache_loaded.store(false, Ordering::Release);
        }
        Ok(())
    }

    async fn ensure_downloaded_cache(&self) {
        ensure_downloaded_cache(
            self.config.download_path.clone(),
            &self.downloaded_cache,
            &self.downloaded_cache_loaded,
        )
        .await;
    }

    /// 检查是否已下载（用于去重）
    pub async fn is_downloaded(&self, aweme_id: &str) -> bool {
        if self
            .history
            .lock()
            .await
            .get(aweme_id)
            .map(|record| Path::new(&record.file_path).is_file())
            .unwrap_or(false)
        {
            return true;
        }

        self.ensure_downloaded_cache().await;
        self.downloaded_cache
            .read()
            .map(|cache| cache.contains(aweme_id))
            .unwrap_or(false)
    }

    /// 添加视频下载任务
    pub async fn add_task(&self, video: &VideoInfo, save_path: Option<PathBuf>) -> Result<String> {
        let base_path = save_path.unwrap_or_else(|| PathBuf::from(&self.config.download_path));

        let media_urls = self.collect_download_media_items(video);
        self.add_media_task(
            video.aweme_id.clone(),
            video.desc.clone(),
            video.author.nickname.clone(),
            video.author.uid.clone(),
            video.video.cover.clone(),
            video.media_type.clone(),
            media_urls,
            video.create_time,
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
        published_at: i64,
        save_path: Option<PathBuf>,
    ) -> Result<String> {
        if media_urls.is_empty() {
            return Err(anyhow!("No media URLs"));
        }

        let task_id = uuid::Uuid::new_v4().to_string();
        let base_path = save_path.unwrap_or_else(|| PathBuf::from(&self.config.download_path));
        let author_dir = build_output_dir(
            &self.config,
            &base_path,
            &author,
            media_type_name(&media_type),
            published_at,
        );
        let filename = generate_filename_with_config(
            &self.config,
            &title,
            &aweme_id,
            &author,
            media_type_name(&media_type),
            published_at,
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
            downloaded_cache: self.downloaded_cache.clone(),
            record_write_lock: self.record_write_lock.clone(),
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
                download_media_group(runtime.clone(), task_id_owned.clone()).await
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
        let downloaded_cache = self.downloaded_cache.clone();
        let record_write_lock = self.record_write_lock.clone();
        let config = self.config.clone();
        let http_client = self.client.clone();

        self.ensure_downloaded_cache().await;

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
            let downloaded_cache = downloaded_cache.clone();
            let record_write_lock = record_write_lock.clone();
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

                    // 检查是否已下载
                    {
                        let is_in_history = history
                            .lock()
                            .await
                            .get(&video.aweme_id)
                            .map(|record| Path::new(&record.file_path).is_file())
                            .unwrap_or(false);
                        let is_in_cache = downloaded_cache
                            .read()
                            .map(|cache| cache.contains(&video.aweme_id))
                            .unwrap_or(false);
                        if is_in_history || is_in_cache {
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
                            continue;
                        }
                    }

                    // 获取信号量许可。已下载视频已经在上方跳过，不占用并发额度。
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };

                    // 克隆变量
                    let history = history.clone();
                    let downloaded_cache = downloaded_cache.clone();
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
                    let record_write_lock = record_write_lock.clone();

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
                            downloaded_cache,
                            record_write_lock,
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
        downloaded_cache: Arc<RwLock<HashSet<String>>>,
        record_write_lock: Arc<Mutex<()>>,
        cancel_tokens: Arc<Mutex<std::collections::HashMap<String, bool>>>,
        pause_tokens: Arc<Mutex<std::collections::HashMap<String, bool>>>,
        batch_task_id: String,
        progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    ) -> Result<()> {
        // 收集媒体URL
        let media_urls = collect_media_items(&video, &config);
        if media_urls.is_empty() {
            return Err(anyhow!("No media URLs"));
        }

        let base_path = PathBuf::from(&config.download_path);
        let author_dir = build_output_dir(
            &config,
            &base_path,
            &video.author.nickname,
            media_type_name(&video.media_type),
            video.create_time,
        );
        let filename = generate_filename_with_config(
            &config,
            &video.desc,
            &video.aweme_id,
            &video.author.nickname,
            media_type_name(&video.media_type),
            video.create_time,
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

            let (response, response_url) =
                request_media_with_fallback(&client, &config, &video.aweme_id, media, &headers)
                    .await?;

            let content_length = response.content_length().unwrap_or(0);
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let extension = media_extension(media.r#type.as_str(), &response_url, content_type);
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
        if record_downloaded(&author_dir, &video.aweme_id, &record_write_lock)
            .await
            .is_ok()
        {
            add_to_downloaded_cache(&downloaded_cache, &video.aweme_id);
        }

        Ok(())
    }

    /// 收集媒体项
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
        self.ensure_downloaded_cache().await;

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

            // 检查是否已下载（去重）
            if self.is_downloaded(&video.aweme_id).await {
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

                continue;
            }

            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };

            // 克隆必要的数据
            let client = self.client.clone();
            let config = self.config.clone();
            let tasks = self.tasks.clone();
            let history = self.history.clone();
            let downloaded_cache = self.downloaded_cache.clone();
            let record_write_lock = self.record_write_lock.clone();
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
                video.create_time,
            );
            let filename = generate_filename_with_config(
                &self.config,
                &video.desc,
                &video.aweme_id,
                &video.author.nickname,
                media_type_name(&video.media_type),
                video.create_time,
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
                    downloaded_cache,
                    record_write_lock,
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
        downloaded_cache: Arc<RwLock<HashSet<String>>>,
        record_write_lock: Arc<Mutex<()>>,
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

            let (response, response_url) =
                request_media_with_fallback(&client, &config, &task.aweme_id, media, &headers)
                    .await?;

            let response_size = response.content_length().unwrap_or(0);
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let extension = media_extension(media.r#type.as_str(), &response_url, content_type);
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
        if record_downloaded(&save_dir, &task.aweme_id, &record_write_lock)
            .await
            .is_ok()
        {
            add_to_downloaded_cache(&downloaded_cache, &task.aweme_id);
        }

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



/// 选择视频下载URL

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::BitRateInfo;
    use chrono::TimeZone;
    use super::super::downloaded_cache::{
        extract_downloaded_aweme_id, is_complete_download_file, parse_downloaded_set,
    };
    use super::super::filename::MAX_FILENAME_BYTES;

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

    fn video_with_resolution_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "play-default".to_string();
        video.video.height = 1080;
        video.video.play_addr_h264 = Some("top-h264".to_string());
        video.video.play_addr_lowbr = Some("lowbr".to_string());
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                height: 480,
                play_addr: Some("p480".to_string()),
                play_addr_h264: Some("p480-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                height: 720,
                play_addr: Some("p720".to_string()),
                play_addr_h264: Some("p720-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                gear_name: "normal_1080_0".to_string(),
                height: 1920,
                play_addr: Some("p1080".to_string()),
                play_addr_h264: Some("p1080-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 800,
                bit_rate: 800,
                gear_name: "normal_1080_1".to_string(),
                height: 1920,
                is_h265: true,
                play_addr: Some("p1080-h265".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 700,
                bit_rate: 700,
                gear_name: "adapt_2k_1440p".to_string(),
                play_addr: Some("p1440".to_string()),
                play_addr_h264: Some("p1440-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 900,
                bit_rate: 900,
                gear_name: "adapt_4k".to_string(),
                play_addr: Some("p2160".to_string()),
                play_addr_h264: Some("p2160-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_sparse_resolution_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "play-default".to_string();
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                height: 480,
                play_addr: Some("sparse-480".to_string()),
                play_addr_h264: Some("sparse-480-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                gear_name: "normal_1080_0".to_string(),
                height: 1920,
                play_addr: Some("sparse-1080".to_string()),
                play_addr_h264: Some("sparse-1080-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_top_level_low_resolution_and_quality_candidates(include_1080: bool) -> VideoInfo {
        let mut bit_rates = vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                gear_name: "normal_540_0".to_string(),
                height: 580,
                play_addr: Some("toplow-540".to_string()),
                play_addr_h264: Some("toplow-540-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                gear_name: "normal_720_0".to_string(),
                height: 580,
                play_addr: Some("toplow-720".to_string()),
                play_addr_h264: Some("toplow-720-h264".to_string()),
                ..Default::default()
            },
        ];
        if include_1080 {
            bit_rates.push(BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                gear_name: "normal_1080_0".to_string(),
                height: 580,
                play_addr: Some("toplow-1080".to_string()),
                play_addr_h264: Some("toplow-1080-h264".to_string()),
                ..Default::default()
            });
        }

        let mut video = VideoInfo::default();
        video.video.play_addr = "toplow-default".to_string();
        video.video.height = 580;
        video.video.play_addr_h264 = Some("toplow-top-h264".to_string());
        video.video.bit_rate = Some(bit_rates);
        video
    }

    fn video_with_portrait_dimension_quality_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "portrait-default".to_string();
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                width: 720,
                height: 1280,
                play_addr: Some("portrait-720".to_string()),
                play_addr_h264: Some("portrait-720-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 700,
                bit_rate: 700,
                width: 1440,
                height: 2560,
                play_addr: Some("portrait-2k".to_string()),
                play_addr_h264: Some("portrait-2k-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_narrow_portrait_1080_candidate() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "narrow-default".to_string();
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                width: 404,
                height: 720,
                play_addr: Some("narrow-720".to_string()),
                play_addr_h264: Some("narrow-720-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                width: 608,
                height: 1080,
                play_addr: Some("narrow-1080".to_string()),
                play_addr_h264: Some("narrow-1080-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    #[test]
    fn auto_prefers_best_h264_candidate() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::Auto).as_deref(),
            Some("bitrate-high-h264")
        );
    }

    #[test]
    fn selection_skips_watermark_candidates() {
        let mut video = VideoInfo::default();
        video.video.play_addr = "https://example.com/aweme/v1/playwm/?watermark=1".to_string();
        video.video.download_addr = Some("https://example.com/clean.mp4".to_string());

        assert_eq!(
            select_video_url(&video, DownloadQuality::Auto).as_deref(),
            Some("https://example.com/clean.mp4")
        );
    }

    #[test]
    fn selection_skips_dash_video_only_candidates() {
        let mut video = VideoInfo::default();
        video.video.play_addr = "https://example.com/progressive.mp4".to_string();
        video.video.bit_rate = Some(vec![BitRateInfo {
            data_size: 900,
            bit_rate: 900,
            gear_name: "adapt_4k".to_string(),
            play_addr: Some("https://example.com/media-video-avc1".to_string()),
            play_addr_h264: Some("https://example.com/media_video_h264".to_string()),
            ..Default::default()
        }]);

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("https://example.com/progressive.mp4")
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
    fn target_quality_prefers_closest_resolution_not_above_target() {
        let video = video_with_resolution_candidates();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(480)).as_deref(),
            Some("p480-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1080)).as_deref(),
            Some("p1080-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1440)).as_deref(),
            Some("p1440-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("p2160-h264")
        );
    }

    #[test]
    fn target_quality_is_a_maximum_height_with_downward_fallback() {
        let video = video_with_sparse_resolution_candidates();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("sparse-1080-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1080)).as_deref(),
            Some("sparse-1080-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(720)).as_deref(),
            Some("sparse-480-h264")
        );
    }

    #[test]
    fn target_quality_prefers_explicit_bitrate_quality_over_top_level_url() {
        let video = video_with_top_level_low_resolution_and_quality_candidates(false);

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("toplow-720-h264")
        );

        let video = video_with_top_level_low_resolution_and_quality_candidates(true);
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("toplow-1080-h264")
        );
    }

    #[test]
    fn target_quality_uses_short_side_for_portrait_2k_candidates() {
        let video = video_with_portrait_dimension_quality_candidates();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("portrait-2k-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1080)).as_deref(),
            Some("portrait-720-h264")
        );
    }

    #[test]
    fn target_quality_uses_short_side_for_top_level_portrait_candidate() {
        let mut video = VideoInfo::default();
        video.video.play_addr = "top-portrait-2k".to_string();
        video.video.play_addr_h264 = Some("top-portrait-2k-h264".to_string());
        video.video.play_addr_lowbr = Some("top-portrait-low".to_string());
        video.video.width = 1440;
        video.video.height = 2560;

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("top-portrait-2k-h264")
        );
    }

    #[test]
    fn target_quality_keeps_portrait_1080_candidate_above_720() {
        let video = video_with_narrow_portrait_1080_candidate();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("narrow-1080-h264")
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
            Some("bitrate-high-h264")
        );

        config.download_quality = "smallest".to_string();
        downloader
            .update_config(config.clone())
            .expect("update config");

        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("lowbr")
        );

        config.download_quality = "1080p".to_string();
        downloader.update_config(config).expect("update config");

        let video = video_with_resolution_candidates();
        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("p1080-h264")
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
    fn extracts_only_protected_aweme_id_suffix() {
        assert_eq!(
            extract_downloaded_aweme_id("标题123456789012_7380011223344556677.mp4").as_deref(),
            Some("7380011223344556677")
        );
        assert_eq!(
            extract_downloaded_aweme_id("标题123456789012_7380011223344556677_02.jpg").as_deref(),
            Some("7380011223344556677")
        );
        assert!(extract_downloaded_aweme_id("标题123456789012.mp4").is_none());
    }

    #[test]
    fn ignores_partial_download_files_case_insensitively() {
        let path = PathBuf::from("标题_7380011223344556677.TMP");
        assert!(!is_complete_download_file(
            &path,
            path.file_name().and_then(|name| name.to_str()).unwrap()
        ));
    }

    #[test]
    fn filename_template_can_omit_aweme_id_suffix() {
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
            0,
        );

        assert_eq!(filename, "这是 一个 完整 标题 第二段 文案");
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
            0,
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
            0,
        );

        assert!(filename.starts_with(&"abcdefghijklmnopqrstuvwxyz".repeat(6)));
        assert!(filename.ends_with(aweme_id));
        assert!(filename.len() <= MAX_FILENAME_BYTES);
    }

    #[test]
    fn filename_template_uses_work_create_time_for_date_tokens() {
        let config = AppConfig {
            filename_template: "{date}_{time}_{title}_{aweme_id}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let create_time = 1_704_067_205;
        let expected_prefix = Local
            .timestamp_opt(create_time, 0)
            .single()
            .unwrap()
            .format("%Y%m%d_%Y%m%d_%H%M%S")
            .to_string();
        let filename = generate_filename_with_config(
            &config,
            "跨年作品",
            aweme_id,
            "作者",
            "video",
            create_time,
        );

        assert_eq!(
            filename,
            format!("{}_跨年作品_{}", expected_prefix, aweme_id)
        );
    }

    #[test]
    fn filename_template_leaves_date_tokens_empty_without_create_time() {
        let config = AppConfig {
            filename_template: "{date}_{time}_{title}_{aweme_id}".to_string(),
            ..Default::default()
        };

        let filename = generate_filename_with_config(
            &config,
            "无发布时间作品",
            "7380011223344556677",
            "作者",
            "video",
            0,
        );

        assert_eq!(filename, "无发布时间作品_7380011223344556677");
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
            build_output_dir(&config, &base, "作者/名", "video", 0),
            base.join("作者_名_video")
        );

        config.auto_create_folder = false;
        assert_eq!(build_output_dir(&config, &base, "作者", "video", 0), base);
    }

    #[test]
    fn author_name_with_asterisk_is_sanitized_for_output_dir() {
        let base = PathBuf::from("/tmp/douyin");
        let config = AppConfig {
            folder_name_template: "{author}".to_string(),
            auto_create_folder: true,
            ..Default::default()
        };

        assert_eq!(
            build_output_dir(&config, &base, "作者*星号", "video", 0),
            base.join("作者_星号")
        );
    }

    #[tokio::test]
    async fn unique_output_file_does_not_overwrite_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("better-douyin-r-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let existing = dir.join("clip.mp4");
        std::fs::write(&existing, b"existing").expect("write existing file");

        let (created_path, created_file) = create_unique_output_file(&dir, "clip", 0, 1, "mp4")
            .await
            .expect("create unique file");
        drop(created_file);

        assert_ne!(created_path, existing);
        assert_eq!(
            created_path.file_name().and_then(|name| name.to_str()),
            Some("clip_2.mp4")
        );
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
