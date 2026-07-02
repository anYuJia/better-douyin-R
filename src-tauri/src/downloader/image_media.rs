use crate::api::types::{DownloadMediaItem, DownloadTask, MediaType};
use crate::config::AppConfig;
use crate::downloader::control::DownloadControl;
use crate::downloader::downloader::DownloaderEvent;
use crate::downloader::events::{emit_event, wait_if_control_paused, PROGRESS_EMIT_INTERVAL};
use crate::downloader::filename::{
    create_unique_output_file_with_same_stem, media_download_success_action, media_extension,
    media_type_display, media_type_name,
};
use crate::downloader::media_request::request_media_with_fallback;
use anyhow::{anyhow, Result};
use chrono::Local;
use futures::StreamExt;
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};

const IMAGE_MEDIA_CONCURRENCY: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MediaFilePlan {
    pub(crate) source_index: usize,
    pub(crate) filename_index: usize,
    pub(crate) filename_total: usize,
    pub(crate) same_stem: bool,
}

#[derive(Clone)]
pub(crate) enum ImageMediaProgress {
    SingleTask {
        tasks: Arc<Mutex<Vec<DownloadTask>>>,
        task_id: String,
        title: String,
        display_name: String,
        save_path: String,
        media_type: MediaType,
        emit_logs: bool,
    },
    BatchCurrent {
        task_id: String,
        aweme_id: String,
        name: String,
    },
}

pub(crate) struct ImageMediaDownloadOptions {
    pub(crate) client: reqwest::Client,
    pub(crate) config: AppConfig,
    pub(crate) aweme_id: String,
    pub(crate) media_urls: Vec<DownloadMediaItem>,
    pub(crate) headers: HeaderMap,
    pub(crate) save_dir: PathBuf,
    pub(crate) filename: String,
    pub(crate) control: DownloadControl,
    pub(crate) progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    pub(crate) progress: ImageMediaProgress,
    pub(crate) pair_live_photo_stems: bool,
}

#[derive(Debug)]
struct ImageMediaResult {
    index: usize,
    file_path: PathBuf,
    file_size: u64,
}

struct SharedProgress {
    started_at: Instant,
    last_emit_at: Instant,
    file_downloaded: Vec<u64>,
    file_total: Vec<u64>,
    file_completed: Vec<bool>,
    completed_files: usize,
    total_downloaded: u64,
}

pub(crate) fn is_image_like_media_type(media_type: &str) -> bool {
    matches!(media_type, "image" | "live_photo")
}

pub(crate) fn should_download_image_media_concurrently(media_urls: &[DownloadMediaItem]) -> bool {
    media_urls.len() > 1
        && media_urls
            .iter()
            .all(|item| is_image_like_media_type(item.r#type.as_str()))
}

pub(crate) fn plan_media_files(
    media_urls: &[DownloadMediaItem],
    pair_live_photo_stems: bool,
) -> Vec<MediaFilePlan> {
    let live_count = media_urls
        .iter()
        .filter(|item| item.r#type == "live_photo")
        .count();
    let image_count = media_urls
        .iter()
        .filter(|item| item.r#type == "image")
        .count();
    let live_pair_count = if pair_live_photo_stems
        && media_urls
            .iter()
            .all(|item| is_image_like_media_type(item.r#type.as_str()))
    {
        live_count.min(image_count)
    } else {
        0
    };

    let mut live_position = 0usize;
    let mut image_position = 0usize;
    media_urls
        .iter()
        .enumerate()
        .map(|(index, media)| {
            let pair_index = if live_pair_count > 0 {
                match media.r#type.as_str() {
                    "live_photo" => {
                        let current = live_position;
                        live_position += 1;
                        Some(current)
                    }
                    "image" => {
                        let current = image_position;
                        image_position += 1;
                        Some(current)
                    }
                    _ => None,
                }
            } else {
                None
            };
            let same_stem = pair_index.is_some();
            MediaFilePlan {
                source_index: index,
                filename_index: pair_index.unwrap_or(index),
                filename_total: if same_stem {
                    live_pair_count
                } else {
                    media_urls.len()
                },
                same_stem,
            }
        })
        .collect()
}

pub(crate) async fn download_image_media_files_concurrently(
    options: ImageMediaDownloadOptions,
) -> Result<(Vec<PathBuf>, u64)> {
    let total_files = options.media_urls.len();
    let plans = plan_media_files(&options.media_urls, options.pair_live_photo_stems);
    let failed = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(SharedProgress {
        started_at: Instant::now(),
        last_emit_at: Instant::now(),
        file_downloaded: vec![0; total_files],
        file_total: vec![0; total_files],
        file_completed: vec![false; total_files],
        completed_files: 0,
        total_downloaded: 0,
    }));
    let same_stem_allocator = Arc::new(Mutex::new(0usize));

    let mut successes: Vec<ImageMediaResult> = Vec::with_capacity(total_files);
    let mut first_error: Option<anyhow::Error> = None;
    let mut stream = futures::stream::iter(options.media_urls.into_iter().zip(plans))
        .map(|(media, plan)| {
            let client = options.client.clone();
            let config = options.config.clone();
            let aweme_id = options.aweme_id.clone();
            let headers = options.headers.clone();
            let save_dir = options.save_dir.clone();
            let filename = options.filename.clone();
            let control = options.control.clone();
            let progress_tx = options.progress_tx.clone();
            let progress = options.progress.clone();
            let shared = shared.clone();
            let failed = failed.clone();
            let same_stem_allocator = same_stem_allocator.clone();

            async move {
                download_one_image_media(
                    client,
                    config,
                    aweme_id,
                    media,
                    plan,
                    headers,
                    save_dir,
                    filename,
                    control,
                    progress_tx,
                    progress,
                    shared,
                    failed,
                    total_files,
                    same_stem_allocator,
                )
                .await
            }
        })
        .buffer_unordered(IMAGE_MEDIA_CONCURRENCY);

    while let Some(result) = stream.next().await {
        match result {
            Ok(file) => successes.push(file),
            Err(error) => {
                failed.store(true, Ordering::Release);
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if let Some(error) = first_error {
        for file in &successes {
            let _ = tokio::fs::remove_file(&file.file_path).await;
        }
        return Err(error);
    }

    successes.sort_by_key(|file| file.index);
    let downloaded_files = successes
        .iter()
        .map(|file| file.file_path.clone())
        .collect::<Vec<_>>();
    let total_size = successes.iter().map(|file| file.file_size).sum();
    Ok((downloaded_files, total_size))
}

#[allow(clippy::too_many_arguments)]
async fn download_one_image_media(
    client: reqwest::Client,
    config: AppConfig,
    aweme_id: String,
    media: DownloadMediaItem,
    plan: MediaFilePlan,
    headers: HeaderMap,
    save_dir: PathBuf,
    filename: String,
    control: DownloadControl,
    progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    progress: ImageMediaProgress,
    shared: Arc<Mutex<SharedProgress>>,
    failed: Arc<AtomicBool>,
    total_files: usize,
    same_stem_allocator: Arc<Mutex<usize>>,
) -> Result<ImageMediaResult> {
    if failed.load(Ordering::Acquire) || control.is_cancelled() {
        return Err(anyhow!("Download cancelled"));
    }
    wait_if_control_paused(&control).await?;

    emit_start_log(
        &progress_tx,
        &progress,
        &media,
        plan.source_index,
        total_files,
    )
    .await;

    let (response, response_url) =
        request_media_with_fallback(&client, &config, &aweme_id, &media, &headers).await?;
    let response_size = response.content_length().unwrap_or(0);
    {
        let mut state = shared.lock().await;
        state.file_total[plan.source_index] = response_size;
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let extension = if plan.same_stem && media.r#type == "live_photo" {
        "mp4".to_string()
    } else {
        media_extension(media.r#type.as_str(), &response_url, content_type)
    };
    let (file_path, mut file) = create_output_file_for_plan(
        &save_dir,
        &filename,
        &extension,
        &plan,
        &same_stem_allocator,
        &failed,
        &control,
    )
    .await?;

    let mut stream = response.bytes_stream();
    let mut file_downloaded = 0u64;
    let file_started_at = Instant::now();
    while let Some(chunk_result) = stream.next().await {
        if failed.load(Ordering::Acquire) || control.is_cancelled() {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(anyhow!("Download cancelled"));
        }
        wait_if_control_paused(&control).await?;

        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(error.into());
            }
        };
        if let Err(error) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(error.into());
        }
        let chunk_len = chunk.len() as u64;
        file_downloaded += chunk_len;

        let payload = {
            let mut state = shared.lock().await;
            state.file_downloaded[plan.source_index] = file_downloaded;
            state.total_downloaded += chunk_len;
            if state.last_emit_at.elapsed() >= PROGRESS_EMIT_INTERVAL {
                state.last_emit_at = Instant::now();
                Some(build_progress_payload(
                    &progress,
                    &media,
                    plan.source_index,
                    file_downloaded,
                    response_size,
                    file_started_at,
                    &state,
                    total_files,
                    Some(file_path.clone()),
                ))
            } else {
                None
            }
        };

        if let Some(payload) = payload {
            update_single_task_progress(&progress, &payload).await;
            emit_progress_payload(&progress_tx, &progress, payload).await;
        }
    }

    if let Err(error) = file.flush().await {
        let _ = tokio::fs::remove_file(&file_path).await;
        return Err(error.into());
    }

    let payload = {
        let mut state = shared.lock().await;
        state.file_downloaded[plan.source_index] = file_downloaded;
        if !state.file_completed[plan.source_index] {
            state.file_completed[plan.source_index] = true;
            state.completed_files += 1;
        }
        build_progress_payload(
            &progress,
            &media,
            plan.source_index,
            file_downloaded,
            response_size,
            file_started_at,
            &state,
            total_files,
            Some(file_path.clone()),
        )
    };
    update_single_task_progress(&progress, &payload).await;
    emit_progress_payload(&progress_tx, &progress, payload).await;
    emit_success_log(
        &progress_tx,
        &progress,
        &media,
        plan.source_index,
        total_files,
        &file_path,
    )
    .await;

    log::info!(
        "{} ({}/{}) 成功：{}",
        media_download_success_action(media.r#type.as_str()),
        plan.source_index + 1,
        total_files,
        file_path.to_string_lossy()
    );

    Ok(ImageMediaResult {
        index: plan.source_index,
        file_path,
        file_size: file_downloaded,
    })
}

async fn create_output_file_for_plan(
    save_dir: &std::path::Path,
    filename: &str,
    extension: &str,
    plan: &MediaFilePlan,
    same_stem_allocator: &Arc<Mutex<usize>>,
    failed: &Arc<AtomicBool>,
    control: &DownloadControl,
) -> Result<(PathBuf, tokio::fs::File)> {
    if !plan.same_stem {
        return create_unique_output_file_with_same_stem(
            save_dir,
            filename,
            plan.filename_index,
            plan.filename_total,
            extension,
            false,
        )
        .await;
    }

    loop {
        if failed.load(Ordering::Acquire) || control.is_cancelled() {
            return Err(anyhow!("Download cancelled"));
        }
        {
            let mut next_index = same_stem_allocator.lock().await;
            if *next_index == plan.source_index {
                let result = create_unique_output_file_with_same_stem(
                    save_dir, filename, 0, 1, extension, true,
                )
                .await;
                *next_index += 1;
                return result;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

fn build_progress_payload(
    progress: &ImageMediaProgress,
    media: &DownloadMediaItem,
    index: usize,
    file_downloaded: u64,
    response_size: u64,
    file_started_at: Instant,
    state: &SharedProgress,
    total_files: usize,
    file_path: Option<PathBuf>,
) -> serde_json::Value {
    let file_progress = if response_size > 0 {
        ((file_downloaded as f64 / response_size as f64) * 100.0) as f32
    } else if file_downloaded > 0 {
        100.0
    } else {
        0.0
    }
    .clamp(0.0, 100.0);
    let progress_units = state
        .file_downloaded
        .iter()
        .zip(state.file_total.iter())
        .enumerate()
        .map(|(file_index, (downloaded, total))| {
            if state.file_completed[file_index] {
                1.0
            } else if *total > 0 {
                (*downloaded as f32 / *total as f32).clamp(0.0, 1.0)
            } else if *downloaded > 0 {
                1.0
            } else {
                0.0
            }
        })
        .sum::<f32>();
    let overall_progress = ((progress_units / total_files as f32) * 100.0).clamp(0.0, 100.0);
    let elapsed = file_started_at.elapsed().as_secs_f64().max(0.001);
    let speed_bps = (file_downloaded as f64 / elapsed) as u64;
    let eta_seconds = if response_size > 0 && speed_bps > 0 {
        Some(response_size.saturating_sub(file_downloaded) / speed_bps)
    } else {
        None
    };

    match progress {
        ImageMediaProgress::SingleTask {
            task_id,
            title,
            display_name,
            save_path,
            media_type,
            ..
        } => serde_json::json!({
            "task_id": task_id,
            "progress": overall_progress,
            "completed": state.completed_files,
            "total": total_files,
            "status": "downloading",
            "desc": title,
            "display_name": display_name,
            "file_index": index + 1,
            "file_total": total_files,
            "file_progress": file_progress,
            "bytes_downloaded": file_downloaded,
            "overall_bytes_downloaded": state.total_downloaded,
            "bytes_total": response_size,
            "speed_bps": speed_bps,
            "eta_seconds": eta_seconds,
            "file_type": media.r#type,
            "file_type_display": media_type_display(media.r#type.as_str()),
            "save_path": save_path,
            "file_path": file_path.map(|path| path.to_string_lossy().to_string()),
            "media_type": media_type_name(media_type)
        }),
        ImageMediaProgress::BatchCurrent {
            task_id,
            aweme_id,
            name,
        } => {
            let elapsed = state.started_at.elapsed().as_secs_f64().max(0.001);
            let aggregate_speed_bps = (state.total_downloaded as f64 / elapsed) as u64;
            serde_json::json!({
                "task_id": task_id,
                "aweme_id": aweme_id,
                "name": name,
                "progress": overall_progress as u32,
                "speed_bps": aggregate_speed_bps
            })
        }
    }
}

async fn update_single_task_progress(progress: &ImageMediaProgress, payload: &serde_json::Value) {
    let ImageMediaProgress::SingleTask { tasks, task_id, .. } = progress else {
        return;
    };
    let mut tasks_lock = tasks.lock().await;
    if let Some(current_task) = tasks_lock.iter_mut().find(|task| task.id == *task_id) {
        current_task.progress = payload
            .get("progress")
            .and_then(|value| value.as_f64())
            .unwrap_or(current_task.progress as f64) as f32;
        current_task.downloaded_size = payload
            .get("overall_bytes_downloaded")
            .and_then(|value| value.as_u64())
            .unwrap_or(current_task.downloaded_size);
        current_task.completed_files = payload
            .get("completed")
            .and_then(|value| value.as_u64())
            .unwrap_or(current_task.completed_files as u64)
            as u32;
        if let Some(bytes_total) = payload.get("bytes_total").and_then(|value| value.as_u64()) {
            current_task.total_size = current_task.total_size.max(bytes_total);
        }
    }
}

async fn emit_progress_payload(
    progress_tx: &Option<mpsc::Sender<DownloaderEvent>>,
    progress: &ImageMediaProgress,
    payload: serde_json::Value,
) {
    let event_name = match progress {
        ImageMediaProgress::SingleTask { .. } => "download-progress",
        ImageMediaProgress::BatchCurrent { .. } => "current-video-progress",
    };
    emit_event(progress_tx, event_name, payload).await;
}

async fn emit_start_log(
    progress_tx: &Option<mpsc::Sender<DownloaderEvent>>,
    progress: &ImageMediaProgress,
    media: &DownloadMediaItem,
    index: usize,
    total_files: usize,
) {
    let ImageMediaProgress::SingleTask {
        task_id,
        display_name,
        emit_logs,
        ..
    } = progress
    else {
        return;
    };
    if !emit_logs {
        return;
    }
    emit_event(
        progress_tx,
        "download-log",
        serde_json::json!({
            "task_id": task_id,
            "display_name": display_name,
            "message": format!("正在下载第 {}/{} 个文件 ({})", index + 1, total_files, media_type_display(media.r#type.as_str())),
            "timestamp": Local::now().format("%H:%M:%S").to_string()
        }),
    )
    .await;
}

async fn emit_success_log(
    progress_tx: &Option<mpsc::Sender<DownloaderEvent>>,
    progress: &ImageMediaProgress,
    media: &DownloadMediaItem,
    index: usize,
    total_files: usize,
    file_path: &PathBuf,
) {
    let ImageMediaProgress::SingleTask {
        task_id,
        display_name,
        emit_logs,
        ..
    } = progress
    else {
        return;
    };
    if !emit_logs {
        return;
    }
    emit_event(
        progress_tx,
        "download-log",
        serde_json::json!({
            "task_id": task_id,
            "display_name": display_name,
            "message": format!(
                "{} ({}/{}) 成功：{}",
                media_download_success_action(media.r#type.as_str()),
                index + 1,
                total_files,
                file_path.to_string_lossy()
            ),
            "timestamp": Local::now().format("%H:%M:%S").to_string()
        }),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(media_type: &str) -> DownloadMediaItem {
        DownloadMediaItem {
            r#type: media_type.to_string(),
            url: format!("https://example.com/{media_type}"),
            fallback_urls: Vec::new(),
        }
    }

    #[test]
    fn image_like_groups_are_concurrent_candidates_only_without_video_or_audio() {
        assert!(should_download_image_media_concurrently(&[
            item("image"),
            item("live_photo"),
            item("image"),
        ]));
        assert!(!should_download_image_media_concurrently(&[item("image")]));
        assert!(!should_download_image_media_concurrently(&[
            item("image"),
            item("video"),
        ]));
        assert!(!should_download_image_media_concurrently(&[
            item("live_photo"),
            item("audio"),
        ]));
    }

    #[test]
    fn media_file_plan_preserves_source_order_and_live_pair_stems() {
        let media = vec![
            item("live_photo"),
            item("live_photo"),
            item("image"),
            item("image"),
        ];

        let plans = plan_media_files(&media, true);

        assert_eq!(
            plans,
            vec![
                MediaFilePlan {
                    source_index: 0,
                    filename_index: 0,
                    filename_total: 2,
                    same_stem: true,
                },
                MediaFilePlan {
                    source_index: 1,
                    filename_index: 1,
                    filename_total: 2,
                    same_stem: true,
                },
                MediaFilePlan {
                    source_index: 2,
                    filename_index: 0,
                    filename_total: 2,
                    same_stem: true,
                },
                MediaFilePlan {
                    source_index: 3,
                    filename_index: 1,
                    filename_total: 2,
                    same_stem: true,
                },
            ]
        );
    }

    #[test]
    fn media_file_plan_keeps_batch_numbering_when_pairing_disabled() {
        let media = vec![item("live_photo"), item("image"), item("image")];

        let plans = plan_media_files(&media, false);

        assert_eq!(
            plans,
            vec![
                MediaFilePlan {
                    source_index: 0,
                    filename_index: 0,
                    filename_total: 3,
                    same_stem: false,
                },
                MediaFilePlan {
                    source_index: 1,
                    filename_index: 1,
                    filename_total: 3,
                    same_stem: false,
                },
                MediaFilePlan {
                    source_index: 2,
                    filename_index: 2,
                    filename_total: 3,
                    same_stem: false,
                },
            ]
        );
    }
}
