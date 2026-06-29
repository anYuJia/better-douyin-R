use crate::api::DouyinClient;
use crate::api::types::VideoInfo;
use crate::downloader::batch::download_single_video;
use crate::downloader::downloader::Downloader;
use crate::downloader::events::{emit_event, estimate_batch_eta};
use crate::downloader::filename::truncate_chars;
use crate::downloader::quality::{DownloadQuality, ordered_video_urls};
use anyhow::Result;
use futures::future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use crate::api::types::MediaType;

fn should_refresh_streaming_video(video: &VideoInfo, config: &crate::config::AppConfig) -> bool {
    if matches!(
        video.media_type,
        MediaType::Image | MediaType::LivePhoto | MediaType::Mixed
    ) {
        return false;
    }

    let candidates = ordered_video_urls(
        video,
        DownloadQuality::from_config(&config.download_quality),
    );
    candidates.is_empty() || candidates.iter().all(|url| looks_like_audio_url(url))
}

fn looks_like_audio_url(url: &str) -> bool {
    let normalized = url.trim().to_ascii_lowercase();
    normalized.ends_with(".mp3")
        || normalized.ends_with(".m4a")
        || normalized.ends_with(".aac")
        || normalized.contains("/ies-music/")
        || normalized.contains("/music/")
}

impl Downloader {
    /// 边获取边下载（流式下载）
    pub async fn start_streaming_download(
        &self,
        client: DouyinClient,
        sec_uid: String,
        batch_task_id: String,
        _nickname: String,
        estimated_total: usize,
    ) -> Result<()> {
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
        let fetch_client = client.clone();

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
                    match fetch_client
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
            let api_client = client.clone();

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
                    let api_client = api_client.clone();

                    let mut video = video;
                    let aweme_id = video.aweme_id.clone();
                    let _display_name = truncate_chars(&video.desc, 8);
                    let start_time = Instant::now();

                    // 启动下载任务
                    let handle = tokio::spawn(async move {
                        if should_refresh_streaming_video(&video, &config) {
                            match api_client.get_video_detail(&video.aweme_id).await {
                                Ok(refreshed_video) => {
                                    log::info!(
                                        "streaming download refreshed video detail before download: aweme_id={}",
                                        video.aweme_id
                                    );
                                    video = refreshed_video;
                                }
                                Err(error) => {
                                    log::warn!(
                                        "streaming download failed to refresh video detail before download: aweme_id={} error={}",
                                        video.aweme_id,
                                        error
                                    );
                                }
                            }
                        }

                        let result = download_single_video(
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
                future::join_all(download_handles).await;
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
}
