//! 媒体组下载逻辑

use crate::api::types::{DownloadMediaItem, DownloadStatus, VideoInfo};
use crate::api::DouyinClient;
use crate::config::AppConfig;
use anyhow::{anyhow, Result};
use chrono::Local;
use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use std::path::PathBuf;
use std::time::Instant;
use tokio::io::AsyncWriteExt;

use super::downloaded_cache::{add_to_downloaded_cache, record_downloaded};
use super::downloader::DownloadRuntime;
use super::events::{emit_event, wait_if_paused};
use super::filename::{create_unique_output_file, media_extension, media_type_display, media_type_name, truncate_chars};
use super::http::build_download_headers;
use super::media_request::request_media_with_fallback;
use super::quality::{select_video_url, DownloadQuality};

pub(crate) fn collect_media_items(video: &VideoInfo, config: &AppConfig) -> Vec<DownloadMediaItem> {
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

pub(crate) async fn download_media_group(runtime: DownloadRuntime, task_id: String) -> Result<()> {
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

        let (response, response_url) = request_media_with_fallback(
            &runtime.client,
            &runtime.config,
            &task.aweme_id,
            media,
            &headers,
        )
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
                Some(response_size.saturating_sub(file_downloaded_size) / speed_bps)
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
    if record_downloaded(&save_dir, &task.aweme_id, &runtime.record_write_lock)
        .await
        .is_ok()
    {
        add_to_downloaded_cache(&runtime.downloaded_cache, &task.aweme_id);
    }

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
