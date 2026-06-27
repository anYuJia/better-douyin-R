use crate::api::SearchUserResult;
use crate::api_helpers::*;
use crate::media_utils::*;
use crate::state::AppState;
use std::collections::HashSet;
use tauri::State;

/// 搜索用户
#[tauri::command]
pub(crate) async fn search_user(
    state: State<'_, AppState>,
    keyword: String,
) -> Result<serde_json::Value, String> {
    let keyword = keyword.trim().to_string();
    if keyword.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "请输入搜索关键词"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client.search_user(&keyword).await {
        Ok(SearchUserResult::NeedVerify { verify_url }) => {
            if let Some(response) = login_required_if_cookie_invalid(&client).await {
                Ok(response)
            } else {
                Ok(serde_json::json!({
                    "success": false,
                    "need_verify": true,
                    "verify_url": verify_url,
                    "message": "需要完成滑块验证"
                }))
            }
        }
        Ok(SearchUserResult::NotFound) => Ok(serde_json::json!({
            "success": false,
            "message": "未找到用户"
        })),
        Ok(SearchUserResult::Single(user)) => Ok(serde_json::json!({
            "success": true,
            "type": "single",
            "user": python_user_value(user.as_ref())
        })),
        Ok(SearchUserResult::Multiple(users)) => Ok(serde_json::json!({
            "success": true,
            "type": "multiple",
            "users": users.iter().map(python_user_value).collect::<Vec<_>>()
        })),
        Err(e) => {
            let message = e.to_string();
            if looks_like_login_error(&message) || looks_like_verify_error(&message) {
                Ok(login_or_verify_response(&client, &message, "https://www.douyin.com/").await)
            } else {
                Ok(serde_json::json!({
                    "success": false,
                    "message": format!("搜索失败: {}", e)
                }))
            }
        }
    }
}

/// 获取用户详情
#[tauri::command]
pub(crate) async fn get_user_detail(
    state: State<'_, AppState>,
    sec_uid: String,
    nickname: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = nickname;
    let sec_uid = sec_uid.trim().to_string();
    if sec_uid.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "用户ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client.get_user_detail(&sec_uid).await {
        Ok(user_detail) => Ok(serde_json::json!({
            "success": true,
            "user": python_user_value(&user_detail.info)
        })),
        Err(e) => {
            let message = e.to_string();
            if looks_like_login_error(&message) {
                Ok(login_required_response(&message))
            } else if looks_like_verify_error(&message) {
                Ok(login_or_verify_response(
                    &client,
                    &message,
                    &format!("https://www.douyin.com/user/{}", sec_uid),
                )
                .await)
            } else {
                Ok(serde_json::json!({
                    "success": false,
                    "message": format!("获取用户详情失败: {}", e)
                }))
            }
        }
    }
}

/// 获取用户视频列表
#[tauri::command]
pub(crate) async fn get_user_videos(
    state: State<'_, AppState>,
    sec_uid: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let sec_uid = sec_uid.trim().to_string();
    if sec_uid.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "用户ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client.get_user_videos(&sec_uid, cursor, count).await {
        Ok((videos, next_cursor, has_more)) => {
            let formatted = videos
                .iter()
                .map(|video| python_video_summary(video, true, true))
                .collect::<Vec<_>>();
            let total_count = formatted.len();

            Ok(serde_json::json!({
                "success": true,
                "videos": formatted,
                "has_more": has_more,
                "cursor": next_cursor,
                "total_count": total_count
            }))
        }
        Err(e) => {
            let message = e.to_string();
            if looks_like_login_error(&message) {
                Ok(login_required_response(&message))
            } else if looks_like_verify_error(&message) {
                Ok(login_or_verify_response(
                    &client,
                    &message,
                    &format!("https://www.douyin.com/user/{}", sec_uid),
                )
                .await)
            } else {
                Ok(serde_json::json!({
                    "success": false,
                    "message": format!("获取用户视频列表失败: {}", e)
                }))
            }
        }
    }
}

/// 获取点赞作者列表
#[tauri::command]
pub(crate) async fn get_liked_authors(
    state: State<'_, AppState>,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let liked_videos = match client.get_liked_videos_python_style("", 0, count).await {
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
                "message": format!("获取点赞作者失败: {}", e)
            }));
        }
    };

    if liked_videos.is_empty() {
        if let Some(response) = login_required_if_cookie_invalid(&client).await {
            return Ok(response);
        }
        return Ok(verify_required_response(
            "获取点赞作者失败，请完成验证后重试",
            "https://www.douyin.com/",
        ));
    }

    let mut seen = HashSet::new();
    let mut authors = Vec::new();

    for video in liked_videos {
        let sec_uid = video.author.sec_uid.trim().to_string();
        if sec_uid.is_empty() || !seen.insert(sec_uid.clone()) {
            continue;
        }

        if let Ok(detail) = client.get_user_detail(&sec_uid).await {
            authors.push(python_user_value(&detail.info));
        } else {
            authors.push(serde_json::json!({
                "nickname": video.author.nickname,
                "unique_id": "",
                "follower_count": 0,
                "following_count": 0,
                "total_favorited": 0,
                "aweme_count": 0,
                "signature": "",
                "sec_uid": sec_uid,
                "avatar_thumb": video.author.avatar_thumb,
            }));
        }
    }

    if authors.is_empty() {
        if let Some(response) = login_required_if_cookie_invalid(&client).await {
            return Ok(response);
        }
        return Ok(verify_required_response(
            "获取点赞作者失败，请完成验证后重试",
            "https://www.douyin.com/",
        ));
    }

    let count = authors.len();
    Ok(serde_json::json!({
        "success": true,
        "data": authors,
        "count": count
    }))
}
