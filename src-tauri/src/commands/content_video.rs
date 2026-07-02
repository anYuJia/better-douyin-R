use crate::api::{DouyinClient, VideoInfo};
use crate::api_helpers::*;
use crate::media_utils::*;
use crate::state::AppState;
use tauri::State;

/// 解析视频链接
#[tauri::command]
pub(crate) async fn parse_url(
    state: State<'_, AppState>,
    url: String,
) -> Result<VideoInfo, String> {
    let client = get_client(&state).await?;

    let aweme_id = DouyinClient::extract_aweme_id(&url)
        .ok_or_else(|| "Invalid URL or video ID".to_string())?;

    let video = client
        .get_video_detail(&aweme_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(video)
}

/// 解析分享链接 (处理重定向)
#[tauri::command]
pub(crate) async fn parse_link(
    state: State<'_, AppState>,
    link: String,
) -> Result<serde_json::Value, String> {
    let trimmed_link = link.trim().to_string();
    if trimmed_link.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "链接不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let video = match client.parse_share_link(&trimmed_link).await {
        Ok(video) => video,
        Err(e) => {
            return Ok(api_login_or_verify_error_response(
                &client,
                "解析链接失败",
                e,
                "https://www.douyin.com/",
            )
            .await)
        }
    };

    let formatted_video = serde_json::json!({
        "author": {
            "nickname": video.author.nickname,
            "avatar_thumb": video.author.avatar_thumb,
            "sec_uid": video.author.sec_uid,
        },
        "aweme_id": video.aweme_id,
        "comment_count": video.statistics.comment_count,
        "cover_url": python_cover_url(&video),
        "create_time": video.create_time,
        "desc": video.desc,
        "digg_count": video.statistics.digg_count,
        "is_liked": video.is_liked,
        "is_collected": video.is_collected,
        "statistics": {
            "digg_count": video.statistics.digg_count,
            "comment_count": video.statistics.comment_count,
            "share_count": video.statistics.share_count,
            "collect_count": video.statistics.collect_count,
            "play_count": video.statistics.play_count,
        },
        "media_type": python_media_type(&video),
        "media_urls": python_media_urls(&video),
        "share_count": video.statistics.share_count
    });

    let mut response = serde_json::json!({
        "success": true,
        "type": "link_parse",
        "video": formatted_video.clone(),
        "videos": [formatted_video]
    });

    if !video.author.sec_uid.is_empty() {
        if let Ok(user_detail) = client.get_user_detail(&video.author.sec_uid).await {
            response["user"] = python_user_value(&user_detail.info);
        }
    }

    Ok(response)
}

/// 获取视频详情
#[tauri::command]
pub(crate) async fn get_video_detail(
    state: State<'_, AppState>,
    aweme_id: String,
) -> Result<serde_json::Value, String> {
    let aweme_id = aweme_id.trim().to_string();
    if aweme_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "视频ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client.get_video_detail(&aweme_id).await {
        Ok(video) => Ok(serde_json::json!({
            "success": true,
            "video": python_video_detail_value(&video)
        })),
        Err(e) => Ok(api_login_or_verify_error_response(
            &client,
            "获取视频详情失败",
            e,
            &format!("https://www.douyin.com/video/{}", aweme_id),
        )
        .await),
    }
}
