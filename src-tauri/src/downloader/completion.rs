use crate::api::types::{DownloadStatus, DownloadTask, MediaType};
use crate::api::DownloadHistory;
use crate::config::AppConfig;
use crate::downloader::downloaded_cache::{
    add_to_downloaded_cache, ensure_downloaded_cache, record_downloaded,
};
use crate::downloader::downloader::{Downloader, DownloaderEvent};
use crate::downloader::events::emit_event;
use crate::downloader::filename::{
    create_unique_output_file, media_extension, media_type_name, truncate_chars,
};
use crate::downloader::http::build_download_headers;
use crate::downloader::media_request::request_media_with_fallback;
use crate::history::HistoryManager;
use anyhow::{anyhow, Result};
use chrono::Local;
use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};

pub(super) async fn record_completed_download(
    aweme_id: &str,
    title: &str,
    author: &str,
    author_id: &str,
    cover: &str,
    media_type: &MediaType,
    save_dir: &Path,
    downloaded_files: &[PathBuf],
    total_size: u64,
    history: &Arc<Mutex<HistoryManager>>,
    downloaded_cache: &Arc<RwLock<HashSet<String>>>,
    record_write_lock: &Arc<Mutex<()>>,
) -> Result<()> {
    // 保存到历史
    if let Some(first_file) = downloaded_files.first() {
        let mut history_lock = history.lock().await;
        let _ = history_lock.add(DownloadHistory {
            aweme_id: aweme_id.to_string(),
            title: title.to_string(),
            author: author.to_string(),
            author_id: author_id.to_string(),
            cover: cover.to_string(),
            file_path: first_file.to_string_lossy().to_string(),
            media_type: media_type_name(media_type).to_string(),
            file_size: total_size,
            create_time: Local::now().timestamp(),
        });
    }

    // 写入本地隐藏去重记录
    if record_downloaded(save_dir, aweme_id, record_write_lock)
        .await
        .is_ok()
    {
        add_to_downloaded_cache(downloaded_cache, aweme_id);
    }

    Ok(())
}

impl Downloader {
    pub(crate) async fn ensure_downloaded_cache(&self) {
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

    /// 单个视频下载（带批量进度）
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn download_single_with_progress(
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

        // 统一收尾记录
        record_completed_download(
            &task.aweme_id,
            &task.title,
            &task.author,
            &task.author_id,
            &task.cover,
            &task.media_type,
            &save_dir,
            &downloaded_files,
            total_downloaded_size,
            &history,
            &downloaded_cache,
            &record_write_lock,
        )
        .await?;

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
