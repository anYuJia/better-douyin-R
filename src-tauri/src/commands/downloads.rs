use crate::api::VideoInfo;
use crate::api_helpers::*;
use crate::download_payload::{combined_video_info_for_download, video_info_from_download_payload};
use crate::friend_chat::coerce_i64;
use crate::media_utils::*;
use crate::state::AppState;
use std::collections::HashSet;
use tauri::State;

// ==================== 下载 API ====================

/// 下载单个视频
#[tauri::command]
pub(crate) async fn download_video(
    state: State<'_, AppState>,
    video: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let raw_media_type = download_media_type_from_payload(&video);
    let aweme_id = video
        .get("aweme_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut published_at = coerce_i64(video.get("create_time"), 0);
    let mut desc = video
        .get("desc")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut author_name = video
        .get("author_name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if author_name.is_empty() {
        author_name = video
            .get("author")
            .and_then(|value| value.get("nickname"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let mut cover = video
        .get("cover_url")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if cover.is_empty() {
        cover = video
            .get("video")
            .and_then(|value| value.get("cover"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    if cover.is_empty() {
        cover = video
            .get("video")
            .and_then(|value| value.get("origin_cover"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let mut media_urls = parse_download_media_items(&video, &raw_media_type);
    let mut media_type = media_type_from_payload_or_items(&raw_media_type, &media_urls);
    let should_refresh_video_media =
        raw_media_type == MEDIA_TYPE_VIDEO || raw_media_type == "unknown";
    let mut payload_video_info = if should_refresh_video_media {
        video_info_from_download_payload(&video)
    } else {
        None
    };
    let mut fresh_video: Option<VideoInfo> = None;

    if (should_refresh_video_media || media_urls.is_empty() || desc.is_empty() || cover.is_empty())
        && !aweme_id.is_empty()
    {
        if let Ok(client) = get_client(&state).await {
            if let Ok(refreshed_video) = client.get_video_detail(&aweme_id).await {
                if desc.is_empty() {
                    desc = refreshed_video.desc.clone();
                }
                if author_name.is_empty() {
                    author_name = refreshed_video.author.nickname.clone();
                }
                if cover.is_empty() {
                    cover = refreshed_video.video.cover.clone();
                }
                if published_at <= 0 {
                    published_at = refreshed_video.create_time;
                }
                if media_urls.is_empty() {
                    media_urls = download_media_items_from_video(&refreshed_video);
                    media_type = refreshed_video.media_type.clone();
                }
                fresh_video = Some(refreshed_video);
            }
        }
    }

    if let Some(payload_video) = payload_video_info.as_mut() {
        if payload_video.aweme_id.trim().is_empty() {
            payload_video.aweme_id = aweme_id.clone();
        }
        if payload_video.desc.trim().is_empty() {
            payload_video.desc = desc.clone();
        }
        if payload_video.author.nickname.trim().is_empty() {
            payload_video.author.nickname = author_name.clone();
        }
        if payload_video.create_time <= 0 {
            payload_video.create_time = published_at;
        }
        if payload_video.video.cover.trim().is_empty() {
            payload_video.video.cover = cover.clone();
        }
    }
    if media_urls.is_empty()
        && !(should_refresh_video_media && (fresh_video.is_some() || payload_video_info.is_some()))
    {
        log::warn!(
            "download_video has no media urls after normalization: aweme_id={} desc={} author={} raw_media_type={}",
            aweme_id,
            desc,
            author_name,
            raw_media_type
        );
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有可用的媒体URL"
        }));
    }

    log::debug!(
        "download_video normalized payload: aweme_id={} media_count={} media_type={:?} author={} cover_present={}",
        aweme_id,
        media_urls.len(),
        media_type,
        author_name,
        !cover.is_empty()
    );

    if desc.is_empty() {
        desc = format!("作品_{}", aweme_id);
    }
    if author_name.is_empty() {
        author_name = "未知作者".to_string();
    }

    let downloader_guard = state.downloader.lock().await;
    let downloader = match downloader_guard.as_ref() {
        Some(downloader) => downloader,
        None => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "服务未完全初始化"
            }));
        }
    };

    let combined_video = if should_refresh_video_media {
        combined_video_info_for_download(
            fresh_video.as_ref(),
            payload_video_info.as_ref(),
            &aweme_id,
        )
    } else {
        None
    };
    let task_result = if should_refresh_video_media {
        if let Some(video_info) = combined_video.as_ref() {
            downloader.add_task(video_info, None).await
        } else {
            downloader
                .add_media_task(
                    aweme_id.clone(),
                    desc.clone(),
                    author_name.clone(),
                    String::new(),
                    cover,
                    media_type,
                    media_urls,
                    published_at,
                    None,
                )
                .await
        }
    } else {
        downloader
            .add_media_task(
                aweme_id.clone(),
                desc.clone(),
                author_name.clone(),
                String::new(),
                cover,
                media_type,
                media_urls,
                published_at,
                None,
            )
            .await
    };

    let task_id = match task_result {
        Ok(task_id) => task_id,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("下载启动失败: {}", e)
            }));
        }
    };

    if let Err(e) = downloader.start_download(&task_id).await {
        return Ok(serde_json::json!({
            "success": false,
            "message": format!("下载启动失败: {}", e)
        }));
    }

    Ok(serde_json::json!({
        "success": true,
        "task_id": task_id,
        "message": "下载任务已启动"
    }))
}

/// 批量下载用户视频（边获取边下载）
#[tauri::command]
pub(crate) async fn download_user_videos(
    state: State<'_, AppState>,
    sec_uid: String,
    nickname: String,
    aweme_count: i64,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    if let Some(response) = login_required_if_cookie_invalid(&client).await {
        return Ok(response);
    }

    let batch_task_id = uuid::Uuid::new_v4().to_string();
    let total_videos = aweme_count.max(0) as usize;

    let downloader = {
        let downloader_guard = state.downloader.lock().await;
        match downloader_guard.as_ref() {
            Some(d) => d.clone(),
            None => {
                return Ok(serde_json::json!({
                    "success": false,
                    "message": "服务未完全初始化"
                }));
            }
        }
    };

    let batch_id = batch_task_id.clone();
    let nickname_clone = nickname.clone();
    let sec_uid_clone = sec_uid.clone();
    let client_clone = client.clone();
    let downloader_clone = downloader.clone();

    downloader
        .emit_batch_started(&batch_id, &nickname, total_videos)
        .await;

    tokio::spawn(async move {
        if let Err(e) = downloader_clone
            .start_streaming_download(
                client_clone,
                sec_uid_clone,
                batch_id,
                nickname_clone,
                total_videos,
            )
            .await
        {
            log::error!("Streaming download error: {}", e);
        }
    });

    Ok(serde_json::json!({
        "success": true,
        "task_id": batch_task_id,
        "message": format!("开始下载 {} 个视频", total_videos),
        "nickname": nickname,
        "total_videos": total_videos
    }))
}

/// 下载点赞视频
#[tauri::command]
pub(crate) async fn download_liked_videos(
    state: State<'_, AppState>,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let (videos, _, _) = match client.get_liked_videos("", 0, count).await {
        Ok(result) => result,
        Err(e) => {
            let message = e.to_string();
            if looks_like_login_error(&message) {
                return Ok(login_required_response(&message));
            }
            if looks_like_verify_error(&message) {
                return Ok(
                    login_or_verify_response(&client, &message, "https://www.douyin.com/").await,
                );
            }
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("获取视频列表失败: {}", e)
            }));
        }
    };

    if videos.is_empty() {
        if let Some(response) = login_required_if_cookie_invalid(&client).await {
            return Ok(response);
        }
        return Ok(verify_required_response(
            "没有找到点赞视频，请完成验证后重试",
            "https://www.douyin.com/",
        ));
    }

    let batch_task_id = uuid::Uuid::new_v4().to_string();
    let total_videos = videos.len();
    let batch_task_id_clone = batch_task_id.clone();

    {
        let downloader_guard = state.downloader.lock().await;
        let downloader = match downloader_guard.as_ref() {
            Some(d) => d,
            None => {
                return Ok(serde_json::json!({
                    "success": false,
                    "message": "服务未完全初始化"
                }));
            }
        };

        let downloader_clone = downloader.clone();
        let videos_clone = videos.clone();

        tokio::spawn(async move {
            if let Err(e) = downloader_clone
                .start_batch_download(videos_clone, batch_task_id_clone, "点赞视频".to_string())
                .await
            {
                log::error!("Batch download error: {}", e);
            }
        });
    }

    Ok(serde_json::json!({
        "success": true,
        "task_id": batch_task_id,
        "message": format!("开始下载 {} 个点赞视频", total_videos),
        "total_videos": total_videos
    }))
}

/// 下载点赞作者作品
#[tauri::command]
pub(crate) async fn download_liked_authors(
    state: State<'_, AppState>,
    count: Option<u32>,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let downloader_guard = state.downloader.lock().await;
    let downloader = match downloader_guard.as_ref() {
        Some(downloader) => downloader,
        None => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "服务未完全初始化"
            }));
        }
    };

    let liked_videos = match client
        .get_liked_videos_python_style("", 0, count.unwrap_or(20))
        .await
    {
        Ok((videos, _, _)) => videos,
        Err(e) => {
            let message = e.to_string();
            if looks_like_login_error(&message) {
                return Ok(login_required_response(&message));
            }
            if looks_like_verify_error(&message) {
                return Ok(
                    login_or_verify_response(&client, &message, "https://www.douyin.com/").await,
                );
            }
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("下载失败: {}", e)
            }));
        }
    };

    if liked_videos.is_empty() {
        if let Some(response) = login_required_if_cookie_invalid(&client).await {
            return Ok(response);
        }
        return Ok(verify_required_response(
            "没有找到点赞作者，请完成验证后重试",
            "https://www.douyin.com/",
        ));
    }

    let mut seen = HashSet::new();
    let mut task_ids = Vec::new();

    for video in liked_videos {
        let sec_uid = video.author.sec_uid.trim().to_string();
        if sec_uid.is_empty() || !seen.insert(sec_uid.clone()) {
            continue;
        }

        match client.get_user_videos(&sec_uid, 0, 100).await {
            Ok((videos, _, _)) => {
                for user_video in &videos {
                    if let Ok(task_id) = downloader.add_task(user_video, None).await {
                        let _ = downloader.start_download(&task_id).await;
                        task_ids.push(task_id);
                    }
                }
            }
            Err(e) => {
                let message = e.to_string();
                if looks_like_login_error(&message) {
                    return Ok(login_required_response(&message));
                }
                if looks_like_verify_error(&message) {
                    return Ok(login_or_verify_response(
                        &client,
                        &message,
                        &format!("https://www.douyin.com/user/{}", sec_uid),
                    )
                    .await);
                }
            }
        }
    }
    let count = task_ids.len();

    if task_ids.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有可下载的点赞作者作品"
        }));
    }

    Ok(serde_json::json!({
        "success": true,
        "task_id": uuid::Uuid::new_v4().to_string(),
        "task_ids": task_ids,
        "message": "点赞作者作品下载任务已开始",
        "count": count
    }))
}

/// 添加下载任务
#[tauri::command]
pub(crate) async fn add_download_task(
    state: State<'_, AppState>,
    video: serde_json::Value,
    save_path: Option<String>,
) -> Result<String, String> {
    let raw_media_type = download_media_type_from_payload(&video);
    let should_refresh_video_media =
        raw_media_type == MEDIA_TYPE_VIDEO || raw_media_type == "unknown";
    let aweme_id = video
        .get("aweme_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let path = save_path.map(std::path::PathBuf::from);
    let mut fresh_video: Option<VideoInfo> = None;
    let payload_video_info = if should_refresh_video_media {
        video_info_from_download_payload(&video)
    } else {
        None
    };

    if should_refresh_video_media && !aweme_id.is_empty() {
        if let Ok(client) = get_client(&state).await {
            if let Ok(refreshed_video) = client.get_video_detail(&aweme_id).await {
                fresh_video = Some(refreshed_video);
            }
        }
    }

    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    let combined_video = combined_video_info_for_download(
        fresh_video.as_ref(),
        payload_video_info.as_ref(),
        &aweme_id,
    );
    if let Some(video_info) = combined_video.as_ref() {
        return downloader
            .add_task(video_info, path)
            .await
            .map_err(|e| e.to_string());
    }

    let media_urls = parse_download_media_items(&video, &raw_media_type);
    if media_urls.is_empty() {
        return Err("没有可用的媒体URL".to_string());
    }
    let media_type = media_type_from_payload_or_items(&raw_media_type, &media_urls);
    let published_at = coerce_i64(video.get("create_time"), 0);
    let desc = video
        .get("desc")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let author_name = video
        .get("author")
        .and_then(|value| value.get("nickname"))
        .and_then(|value| value.as_str())
        .unwrap_or("未知作者")
        .trim()
        .to_string();
    let cover = video
        .get("cover_url")
        .and_then(|value| value.as_str())
        .or_else(|| {
            video
                .get("video")
                .and_then(|value| value.get("cover"))
                .and_then(|value| value.as_str())
        })
        .unwrap_or("")
        .trim()
        .to_string();

    downloader
        .add_media_task(
            aweme_id,
            desc,
            author_name,
            String::new(),
            cover,
            media_type,
            media_urls,
            published_at,
            path,
        )
        .await
        .map_err(|e| e.to_string())
}

/// 开始下载
#[tauri::command]
pub(crate) async fn start_download(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    downloader
        .start_download(&task_id)
        .await
        .map_err(|e| e.to_string())
}

/// 获取下载任务列表
#[tauri::command]
pub(crate) async fn get_download_tasks(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    Ok(serde_json::json!({
        "success": true,
        "tasks": downloader.get_tasks().await
    }))
}

/// 取消下载任务
#[tauri::command]
pub(crate) async fn cancel_download_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    match downloader.cancel_task(&task_id).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "message": "任务已取消"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("取消任务失败: {}", e)
        })),
    }
}

/// 删除下载任务
#[tauri::command]
pub(crate) async fn remove_download_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    downloader
        .remove_task(&task_id)
        .await
        .map_err(|e| e.to_string())
}

/// 暂停下载
#[tauri::command]
pub(crate) async fn pause_download(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    match downloader.pause_task(&task_id).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "message": "任务已暂停"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("暂停任务失败: {}", e)
        })),
    }
}

/// 恢复下载
#[tauri::command]
pub(crate) async fn resume_download(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    match downloader.resume_task(&task_id).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "message": "任务已恢复"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("恢复任务失败: {}", e)
        })),
    }
}
