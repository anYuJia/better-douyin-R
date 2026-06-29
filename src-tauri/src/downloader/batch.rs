//! 批量下载逻辑

use crate::api::types::{DownloadStatus, DownloadTask, VideoInfo};
use crate::config::AppConfig;
use crate::history::HistoryManager;
use anyhow::{Result, anyhow};
use chrono::Local;
use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc};

use super::completion::record_completed_download;
use super::downloader::{Downloader, DownloaderEvent};
use super::events::{emit_event, estimate_batch_eta};
use super::filename::{
    build_output_dir, create_unique_output_file, generate_filename_with_config, media_extension,
    media_download_success_action, media_type_name, truncate_chars,
};
use super::http::build_download_headers;
use super::media_group::collect_media_items;
use super::media_request::request_media_with_fallback;

pub(crate) async fn download_single_video(
    client: reqwest::Client,
    config: AppConfig,
    video: VideoInfo,
    history: Arc<Mutex<HistoryManager>>,
    downloaded_cache: Arc<RwLock<HashSet<String>>>,
    record_write_lock: Arc<Mutex<()>>,
    cancel_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pause_tokens: Arc<Mutex<HashMap<String, bool>>>,
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
            request_media_with_fallback(&client, &config, &video.aweme_id, media, &headers).await?;

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

        log::info!(
            "{} ({}/{}) 成功：{}",
            media_download_success_action(media.r#type.as_str()),
            index + 1,
            total_files,
            file_path.to_string_lossy()
        );

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

    record_completed_download(
        &video.aweme_id,
        &video.desc,
        &video.author.nickname,
        &video.author.uid,
        &video.video.cover,
        &video.media_type,
        &author_dir,
        &downloaded_files,
        total_size,
        &history,
        &downloaded_cache,
        &record_write_lock,
    )
    .await?;

    Ok(())
}

pub(crate) async fn start_batch_download_impl(
    downloader: &Downloader,
    videos: Vec<VideoInfo>,
    batch_task_id: String,
    nickname: String,
) -> Result<()> {
    let total_videos = videos.len();
    if total_videos == 0 {
        return Err(anyhow!("No videos to download"));
    }

    // 初始化取消和暂停标记
    downloader
        .cancel_tokens
        .lock()
        .await
        .insert(batch_task_id.clone(), false);
    downloader
        .pause_tokens
        .lock()
        .await
        .insert(batch_task_id.clone(), false);

    // 发送批量下载开始事件
    emit_event(
        &downloader.progress_tx,
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
    let max_concurrent = downloader.config.max_concurrent.clamp(1, 10);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    let completed_count = Arc::new(AtomicUsize::new(0));
    let skipped_count = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));

    let mut download_handles = Vec::new();
    downloader.ensure_downloaded_cache().await;

    for video in videos {
        // 检查取消
        if *downloader
            .cancel_tokens
            .lock()
            .await
            .get(&batch_task_id)
            .unwrap_or(&false)
        {
            break;
        }

        // 检查是否已下载（去重）
        if downloader.is_downloaded(&video.aweme_id).await {
            skipped_count.fetch_add(1, AtomicOrdering::SeqCst);
            let current = completed_count.fetch_add(1, AtomicOrdering::SeqCst) + 1;

            // 发送跳过事件
            emit_event(
                &downloader.progress_tx,
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
        let client = downloader.client.clone();
        let config = downloader.config.clone();
        let tasks = downloader.tasks.clone();
        let history = downloader.history.clone();
        let downloaded_cache = downloader.downloaded_cache.clone();
        let record_write_lock = downloader.record_write_lock.clone();
        let cancel_tokens = downloader.cancel_tokens.clone();
        let pause_tokens = downloader.pause_tokens.clone();
        let progress_tx = downloader.progress_tx.clone();
        let batch_id = batch_task_id.clone();
        let completed = completed_count.clone();
        let failed = failed_count.clone();
        let aweme_id = video.aweme_id.clone();

        // 收集媒体URL
        let media_urls = downloader.collect_download_media_items(&video);
        if media_urls.is_empty() {
            failed_count.fetch_add(1, AtomicOrdering::SeqCst);
            let current = completed_count.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            emit_event(
                &downloader.progress_tx,
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

        let base_path = PathBuf::from(&downloader.config.download_path);
        let author_dir = build_output_dir(
            &downloader.config,
            &base_path,
            &video.author.nickname,
            media_type_name(&video.media_type),
            video.create_time,
        );
        let filename = generate_filename_with_config(
            &downloader.config,
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

        downloader.tasks.lock().await.push(task);

        // 启动并发下载任务
        let handle = tokio::spawn(async move {
            let result = Downloader::download_single_with_progress(
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
    let was_cancelled = *downloader
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
            &downloader.progress_tx,
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
            &downloader.progress_tx,
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
        )
        .await;
    }

    // 清理
    downloader.cancel_tokens.lock().await.remove(&batch_task_id);
    downloader.pause_tokens.lock().await.remove(&batch_task_id);

    Ok(())
}
