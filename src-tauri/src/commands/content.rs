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

/// 获取评论列表
#[tauri::command]
pub(crate) async fn get_comments(
    state: State<'_, AppState>,
    aweme_id: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let (comments, next_cursor, has_more, total) =
        match client.get_comments(&aweme_id, cursor, count).await {
            Ok(result) => result,
            Err(e) => {
                return Ok(api_login_or_verify_error_response(
                    &client,
                    "获取评论失败",
                    e,
                    &format!("https://www.douyin.com/video/{}", aweme_id),
                )
                .await)
            }
        };

    Ok(serde_json::json!({
        "success": true,
        "comments": comments,
        "cursor": next_cursor,
        "has_more": has_more,
        "total": total
    }))
}

/// 获取评论的二级回复列表
#[tauri::command]
pub(crate) async fn get_comment_replies(
    state: State<'_, AppState>,
    aweme_id: String,
    comment_id: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    let (comments, next_cursor, has_more, total) = match client
        .get_comment_replies(&aweme_id, &comment_id, cursor, count)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            return Ok(api_login_or_verify_error_response(
                &client,
                "获取评论回复失败",
                e,
                &format!("https://www.douyin.com/video/{}", aweme_id),
            )
            .await)
        }
    };

    Ok(serde_json::json!({
        "success": true,
        "comments": comments,
        "cursor": next_cursor,
        "has_more": has_more,
        "total": total
    }))
}

/// 点赞或取消点赞评论
#[tauri::command]
pub(crate) async fn set_comment_liked(
    state: State<'_, AppState>,
    aweme_id: String,
    comment_id: String,
    liked: bool,
    level: u32,
) -> Result<serde_json::Value, String> {
    let aweme_id = aweme_id.trim().to_string();
    let comment_id = comment_id.trim().to_string();
    if aweme_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "作品ID不能为空"
        }));
    }
    if comment_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "评论ID不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client
        .set_comment_liked(&aweme_id, &comment_id, liked, level)
        .await
    {
        Ok(response) => Ok(serde_json::json!({
            "success": true,
            "aweme_id": aweme_id,
            "cid": comment_id,
            "user_digged": if liked { 1 } else { 0 },
            "raw": response,
            "message": if liked { "评论点赞成功" } else { "已取消评论点赞" }
        })),
        Err(e) => Ok(api_login_or_verify_error_response(
            &client,
            "评论点赞失败",
            e,
            &format!("https://www.douyin.com/video/{}", aweme_id),
        )
        .await),
    }
}

/// 发布一级评论或回复评论
#[tauri::command]
pub(crate) async fn publish_comment(
    state: State<'_, AppState>,
    aweme_id: String,
    text: String,
    reply_id: String,
    reply_to_reply_id: String,
) -> Result<serde_json::Value, String> {
    let aweme_id = aweme_id.trim().to_string();
    let text = text.trim().to_string();
    if aweme_id.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "作品ID不能为空"
        }));
    }
    if text.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "评论内容不能为空"
        }));
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };

    match client
        .publish_comment(&aweme_id, &text, &reply_id, &reply_to_reply_id)
        .await
    {
        Ok((response, comment)) => Ok(serde_json::json!({
            "success": true,
            "aweme_id": aweme_id,
            "comment": comment,
            "raw": response,
            "message": "评论已发布"
        })),
        Err(e) => Ok(api_verify_or_error_response(
            "发表评论失败",
            e,
            &format!("https://www.douyin.com/video/{}", aweme_id),
        )),
    }
}
