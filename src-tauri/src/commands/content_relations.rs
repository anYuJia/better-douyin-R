use crate::api_helpers::*;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub(crate) async fn set_video_liked(
    state: State<'_, AppState>,
    aweme_id: String,
    liked: bool,
) -> Result<serde_json::Value, String> {
    let aweme_id = aweme_id.trim().to_string();
    if aweme_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "作品ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client.set_video_liked(&aweme_id, liked).await {
        Ok(response) => Ok(serde_json::json!({
                "success": true,
                "aweme_id": aweme_id,
                "is_liked": liked,
                "raw": response,
                "message": if liked { "点赞成功" } else { "已取消点赞" }
        })),
        Err(e) => Ok(api_login_or_verify_error_response(
            &client,
            if liked {
                "点赞失败"
            } else {
                "取消点赞失败"
            },
            e,
            &format!("https://www.douyin.com/video/{}", aweme_id),
        )
        .await),
    }
}

#[tauri::command]
pub(crate) async fn set_video_collected(
    state: State<'_, AppState>,
    aweme_id: String,
    collected: bool,
) -> Result<serde_json::Value, String> {
    let aweme_id = aweme_id.trim().to_string();
    if aweme_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "作品ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client.set_video_collected(&aweme_id, collected).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "aweme_id": aweme_id,
            "is_collected": collected,
            "message": if collected { "收藏成功" } else { "已取消收藏" }
        })),
        Err(e) => Ok(api_login_or_verify_error_response(
            &client,
            if collected {
                "收藏失败"
            } else {
                "取消收藏失败"
            },
            e,
            &format!("https://www.douyin.com/video/{}", aweme_id),
        )
        .await),
    }
}

#[tauri::command]
pub(crate) async fn set_user_followed(
    state: State<'_, AppState>,
    user_id: String,
    follow: bool,
) -> Result<serde_json::Value, String> {
    let user_id = user_id.trim().to_string();
    if user_id.is_empty() {
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

    match client.set_user_followed(&user_id, follow).await {
        Ok(resp) => {
            let follow_status = resp.get("follow_status")
                .and_then(|v| v.as_i64())
                .unwrap_or(if follow { 1 } else { 0 });
            Ok(serde_json::json!({
                "success": true,
                "user_id": user_id,
                "is_follow": follow,
                "follow_status": follow_status,
                "message": if follow { "关注成功" } else { "已取消关注" }
            }))
        }
        Err(e) => Ok(api_login_or_verify_error_response(
            &client,
            if follow {
                "关注失败"
            } else {
                "取消关注失败"
            },
            e,
            "https://www.douyin.com/",
        )
        .await),
    }
}
