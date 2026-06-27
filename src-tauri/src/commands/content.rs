use crate::api::{DouyinClient, SearchUserResult, VideoInfo};
use crate::api_helpers::*;
use crate::media_utils::*;
use crate::state::AppState;
use std::collections::HashSet;
use tauri::State;

// ==================== 视频/用户 API ====================

/// 解析视频链接
#[tauri::command]
pub(crate) async fn parse_url(state: State<'_, AppState>, url: String) -> Result<VideoInfo, String> {
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
pub(crate) async fn parse_link(state: State<'_, AppState>, link: String) -> Result<serde_json::Value, String> {
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
