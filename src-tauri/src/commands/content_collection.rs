use crate::api_helpers::*;
use crate::state::AppState;
use tauri::State;

/// 获取点赞视频列表
#[tauri::command]
pub(crate) async fn get_liked_videos(
    state: State<'_, AppState>,
    sec_uid: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(feature_login_required_response("点赞视频"));
        }
    };
    if let Some(response) = ensure_feature_login(&state, &client, "点赞视频").await {
        return Ok(response);
    }

    match client
        .get_liked_videos_python_style(&sec_uid, cursor, count)
        .await
    {
        Ok((videos, next_cursor, has_more)) if !videos.is_empty() => {
            let count = videos.len();
            Ok(serde_json::json!({
                "success": true,
                "data": videos,
                "count": count,
                "cursor": next_cursor,
                "has_more": has_more
            }))
        }
        Ok((videos, next_cursor, _has_more)) => {
            if cursor > 0 {
                Ok(serde_json::json!({
                    "success": true,
                    "data": videos,
                    "count": 0,
                    "cursor": next_cursor,
                    "has_more": false
                }))
            } else if login_required_if_cookie_invalid(&client).await.is_some() {
                Ok(feature_login_required_response("点赞视频"))
            } else {
                Ok(verify_required_response(
                    "获取点赞视频失败，请完成验证后重试",
                    "https://www.douyin.com/",
                ))
            }
        }
        Err(e) => {
            let message = e.to_string();
            if looks_like_login_error(&message) {
                Ok(feature_login_required_response("点赞视频"))
            } else if looks_like_verify_error(&message) {
                Ok(verify_required_response(
                    &format!("获取点赞视频失败: {}", message),
                    "https://www.douyin.com/",
                ))
            } else {
                Ok(serde_json::json!({
                    "success": false,
                    "message": format!("获取点赞视频失败: {}", e)
                }))
            }
        }
    }
}

/// 获取收藏视频列表
#[tauri::command]
pub(crate) async fn get_collected_videos(
    state: State<'_, AppState>,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(feature_login_required_response("收藏视频"));
        }
    };
    if let Some(response) = ensure_feature_login(&state, &client, "收藏视频").await {
        return Ok(response);
    }

    match client
        .get_collected_videos_python_style(cursor, count)
        .await
    {
        Ok((videos, next_cursor, has_more)) => Ok(serde_json::json!({
            "success": true,
            "data": videos,
            "count": videos.len(),
            "cursor": next_cursor,
            "has_more": has_more
        })),
        Err(error) => {
            let message = error.to_string();
            if looks_like_login_error(&message) {
                Ok(feature_login_required_response("收藏视频"))
            } else {
                Ok(api_verify_or_error_response(
                    "获取收藏视频失败",
                    error,
                    "https://www.douyin.com/user/self?showTab=favorite_collection",
                ))
            }
        }
    }
}

/// 获取收藏合集列表
#[tauri::command]
pub(crate) async fn get_collected_mixes(
    state: State<'_, AppState>,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(feature_login_required_response("收藏合集"));
        }
    };
    if let Some(response) = ensure_feature_login(&state, &client, "收藏合集").await {
        return Ok(response);
    }

    match client.get_collected_mixes(cursor, count).await {
        Ok((mixes, next_cursor, has_more)) => Ok(serde_json::json!({
            "success": true,
            "data": mixes,
            "count": mixes.len(),
            "cursor": next_cursor,
            "has_more": has_more
        })),
        Err(error) => {
            let message = error.to_string();
            if looks_like_login_error(&message) {
                Ok(feature_login_required_response("收藏合集"))
            } else {
                Ok(api_verify_or_error_response(
                    "获取收藏合集失败",
                    error,
                    "https://www.douyin.com/user/self?showTab=favorite_collection",
                ))
            }
        }
    }
}

/// 获取合集内的视频列表
#[tauri::command]
pub(crate) async fn get_mix_videos(
    state: State<'_, AppState>,
    series_id: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let series_id = series_id.trim().to_string();
    if series_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "合集ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(feature_login_required_response("收藏合集"));
        }
    };
    if let Some(response) = ensure_feature_login(&state, &client, "收藏合集").await {
        return Ok(response);
    }

    match client.get_mix_videos(&series_id, cursor, count).await {
        Ok((videos, next_cursor, has_more)) => Ok(serde_json::json!({
            "success": true,
            "data": videos,
            "count": videos.len(),
            "cursor": next_cursor,
            "has_more": has_more
        })),
        Err(error) => Ok(api_login_or_verify_error_response(
            &client,
            "获取合集视频失败",
            error,
            "https://www.douyin.com/user/self?showTab=favorite_collection",
        )
        .await),
    }
}
