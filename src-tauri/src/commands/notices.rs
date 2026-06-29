use crate::api_helpers::*;
use crate::state::AppState;
use tauri::State;

/// 获取通知消息列表（点赞/关注/评论等互动通知）。
///
/// `max_time` 为翻历史游标：传入上一批返回的 cursor。首次传 0/None。
#[tauri::command]
pub(crate) async fn get_notices(
    state: State<'_, AppState>,
    count: Option<u32>,
    max_time: Option<i64>,
    min_time: Option<i64>,
    notice_group: Option<i64>,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let count = count.unwrap_or(10).clamp(1, 50);
    let max_time = max_time.unwrap_or(0).max(0);
    let min_time = min_time.unwrap_or(0).max(0);
    let notice_group = notice_group.unwrap_or(960);

    log::debug!(
        "get_notices invoked: count={} min_time={} max_time={} notice_group={}",
        count,
        min_time,
        max_time,
        notice_group,
    );

    let result = client
        .get_notices(count, min_time, max_time, notice_group)
        .await;

    let result = match result {
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
                    "https://www.douyin.com/",
                )
                .await);
            }
            log::error!(
                "get_notices failed: count={} max_time={} error={}",
                count,
                max_time,
                e
            );
            return Ok(serde_json::json!({
                "success": false,
                "message": "获取通知失败，请稍后重试"
            }));
        }
    };

    log::debug!(
        "get_notices completed: count={} has_more={} cursor={} unread={} total={}",
        count,
        result.has_more,
        result.cursor,
        result.unread_count,
        result.notices.len(),
    );

    Ok(serde_json::json!({
        "success": true,
        "message": "获取通知成功",
        "notices": result.notices,
        "count": result.notices.len(),
        "unread_count": result.unread_count,
        "has_more": result.has_more,
        "cursor": result.cursor,
    }))
}
