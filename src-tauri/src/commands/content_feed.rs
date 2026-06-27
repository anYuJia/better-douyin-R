use crate::api_helpers::*;
use crate::media_utils::*;
use crate::state::AppState;
use tauri::State;

/// 获取推荐视频
#[tauri::command]
pub(crate) async fn get_recommended(
    state: State<'_, AppState>,
    cursor: i64,
    count: u32,
    feed_type: Option<String>,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let feed_type = normalize_recommended_feed_type(feed_type.as_deref().unwrap_or("featured"));

    log::debug!(
        "get_recommended invoked: feed_type={} cursor={} count={}",
        feed_type,
        cursor,
        count
    );

    let (videos, next_cursor, has_more) =
        match client.get_recommended_feed(cursor, count, feed_type).await {
            Ok(result) => result,
            Err(e) => {
                let message = e.to_string();
                if looks_like_login_error(&message) {
                    return Ok(login_required_response(&message));
                }
                if looks_like_verify_error(&message) {
                    return Ok(login_or_verify_response(
                        &client,
                        &message,
                        "https://www.douyin.com/?recommend=1",
                    )
                    .await);
                }
                log::error!(
                    "get_recommended failed: feed_type={} cursor={} count={} error={}",
                    feed_type,
                    cursor,
                    count,
                    e
                );
                return Ok(serde_json::json!({
                    "success": false,
                    "message": "获取推荐视频失败，请稍后重试"
                }));
            }
        };

    log::debug!(
        "get_recommended completed: feed_type={} cursor={} count={} next_cursor={} has_more={} videos={}",
        feed_type,
        cursor,
        count,
        next_cursor,
        has_more,
        videos.len()
    );

    let formatted = videos
        .iter()
        .map(python_recommended_video)
        .collect::<Vec<_>>();
    let count = formatted.len();

    Ok(serde_json::json!({
        "success": true,
        "videos": formatted,
        "cursor": next_cursor,
        "has_more": has_more,
        "count": count,
        "feed_type": feed_type
    }))
}
