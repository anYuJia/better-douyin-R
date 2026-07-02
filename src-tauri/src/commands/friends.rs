use crate::api_helpers::*;
use crate::friend_chat::{
    coerce_i64, friend_chat_state_path, json_object_with_success, sanitize_friend_chat_state,
    sanitize_sec_user_ids,
};
use crate::im_listener::ensure_im_message_listener;
use crate::state::AppState;
use std::collections::HashSet;
use std::fs;
use tauri::State;

#[tauri::command]
pub(crate) async fn get_friend_online_status(
    state: State<'_, AppState>,
    sec_user_ids: Vec<String>,
    conv_ids: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    let mut seen = HashSet::new();
    let mut sec_user_ids = sec_user_ids
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect::<Vec<_>>();
    let has_provided_sec_user_ids = !sec_user_ids.is_empty();
    sec_user_ids = sanitize_sec_user_ids(sec_user_ids);

    if sec_user_ids.is_empty() {
        sec_user_ids = state
            .config
            .lock()
            .await
            .im_friend_sec_user_ids
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && seen.insert(value.clone()))
            .collect::<Vec<_>>();
    }

    if has_provided_sec_user_ids && !sec_user_ids.is_empty() {
        let mut config = state.config.lock().await;
        let mut merged = config.im_friend_sec_user_ids.clone();
        merged.extend(sec_user_ids.clone());
        let merged = sanitize_sec_user_ids(merged);
        if merged.len() != config.im_friend_sec_user_ids.len() {
            config.im_friend_sec_user_ids = merged;
            if let Err(error) = config.save() {
                log::warn!("failed to save provided IM friend ids cache: {}", error);
            }
        }
    }

    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(cookie_required_response());
        }
    };
    ensure_im_message_listener(state.inner(), client.clone()).await;

    let mut auto_fetch_failed = None;
    let mut auto_fetch_succeeded = false;
    let include_all_users = state.config.lock().await.im_friend_include_all_users;
    match client
        .get_im_spotlight_relation_sec_user_ids(500, include_all_users)
        .await
    {
        Ok(fetched_ids) => {
            log::debug!(
                "friend online auto IM spotlight ids fetched: include_all_users={} raw_count={}",
                include_all_users,
                fetched_ids.len()
            );
            auto_fetch_succeeded = true;
            let fetched_ids = sanitize_sec_user_ids(fetched_ids);
            sec_user_ids = fetched_ids.clone();

            let mut config = state.config.lock().await;
            if config.im_friend_sec_user_ids != sec_user_ids {
                config.im_friend_sec_user_ids = sec_user_ids.clone();
                if let Err(error) = config.save() {
                    log::warn!("failed to save IM spotlight friend ids cache: {}", error);
                }
            }
        }
        Err(error) => {
            log::warn!(
                "friend online auto IM spotlight relation ids failed: {}",
                error
            );
            if looks_like_login_error(&error.to_string()) {
                return Ok(api_login_or_verify_error_response(
                    &client,
                    "自动获取 IM 好友关系失败",
                    error,
                    "https://www.douyin.com/",
                )
                .await);
            }
            auto_fetch_failed = Some(error);
        }
    }

    if sec_user_ids.is_empty() && !auto_fetch_succeeded {
        match client.get_current_user().await {
            Ok(current_user) => {
                let user_id = current_user.uid.trim().to_string();
                let sec_uid = current_user.sec_uid.trim().to_string();
                match client
                    .get_following_sec_user_ids(&user_id, &sec_uid, 500, !include_all_users)
                    .await
                {
                    Ok(fetched_ids) => {
                        log::debug!(
                            "friend online auto following ids fetched: raw_count={}",
                            fetched_ids.len()
                        );
                        sec_user_ids = sanitize_sec_user_ids(fetched_ids);
                        if !sec_user_ids.is_empty() {
                            let mut config = state.config.lock().await;
                            let mut merged = config.im_friend_sec_user_ids.clone();
                            merged.extend(sec_user_ids.clone());
                            config.im_friend_sec_user_ids = sanitize_sec_user_ids(merged);
                            if let Err(error) = config.save() {
                                log::warn!("failed to save IM friend ids cache: {}", error);
                            }
                        }
                    }
                    Err(error) => {
                        let error = auto_fetch_failed.unwrap_or(error);
                        return Ok(api_login_or_verify_error_response(
                            &client,
                            "自动获取 IM 好友关系失败",
                            error,
                            "https://www.douyin.com/",
                        )
                        .await);
                    }
                }
            }
            Err(error) => {
                return Ok(api_login_or_verify_error_response(
                    &client,
                    "自动获取当前用户失败",
                    error,
                    "https://www.douyin.com/",
                )
                .await);
            }
        }
    }

    if sec_user_ids.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有获取到 IM 好友关系；Cookie 可用，但 spotlight relation 和关注列表都没有返回可用 sec_user_id。"
        }));
    }

    let conv_ids = conv_ids.unwrap_or_default();
    let mut user_info_data = Vec::new();
    let mut active_status_data = Vec::new();
    let mut not_friend_data = Vec::new();
    let mut active_status_sec_user_ids = HashSet::new();
    let mut user_info_extra = serde_json::Value::Null;
    let mut active_status_extra = serde_json::Value::Null;

    for (index, chunk) in sec_user_ids.chunks(20).enumerate() {
        let chunk_ids = chunk.to_vec();
        log::debug!(
            "friend online IM batch request: batch={} size={} total={}",
            index + 1,
            chunk_ids.len(),
            sec_user_ids.len()
        );

        let user_info = match client.get_im_user_info(&chunk_ids).await {
            Ok(response) => response,
            Err(error) => {
                return Ok(api_login_or_verify_error_response(
                    &client,
                    "获取好友资料失败",
                    error,
                    "https://www.douyin.com/",
                )
                .await)
            }
        };
        if user_info_extra.is_null() {
            user_info_extra = user_info
                .get("extra")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
        let user_info_count = user_info
            .get("data")
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or_default();
        log::debug!(
            "friend online IM user info batch response: batch={} requested={} returned={}",
            index + 1,
            chunk_ids.len(),
            user_info_count
        );
        if let Some(items) = user_info.get("data").and_then(|value| value.as_array()) {
            user_info_data.extend(items.iter().cloned());
        }

        let active_status = match client
            .get_im_user_active_status(&chunk_ids, &conv_ids)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return Ok(api_login_or_verify_error_response(
                    &client,
                    "获取好友在线状态失败",
                    error,
                    "https://www.douyin.com/",
                )
                .await)
            }
        };
        if active_status_extra.is_null() {
            active_status_extra = active_status
                .get("extra")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
        let active_status_count = active_status
            .get("data")
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or_default();
        log::debug!(
            "friend online IM active status batch response: batch={} requested={} returned={}",
            index + 1,
            chunk_ids.len(),
            active_status_count
        );
        if let Some(items) = active_status.get("data").and_then(|value| value.as_array()) {
            for item in items {
                if let Some(sec_uid) = item
                    .get("sec_uid")
                    .and_then(|value| value.as_str())
                    .or_else(|| item.get("sec_user_id").and_then(|value| value.as_str()))
                {
                    active_status_sec_user_ids.insert(sec_uid.to_string());
                }
                active_status_data.push(item.clone());
            }
        }
        if let Some(items) = active_status
            .get("not_friend_data")
            .and_then(|value| value.as_array())
        {
            not_friend_data.extend(items.iter().cloned());
        }
    }

    sec_user_ids.retain(|id| active_status_sec_user_ids.contains(id));
    user_info_data.retain(|item| {
        item.get("sec_uid")
            .and_then(|value| value.as_str())
            .or_else(|| item.get("sec_user_id").and_then(|value| value.as_str()))
            .map(|id| active_status_sec_user_ids.contains(id))
            .unwrap_or(false)
    });

    Ok(serde_json::json!({
        "success": true,
        "sec_user_ids": sec_user_ids,
        "user_info": {
            "status_code": 0,
            "data": user_info_data,
            "extra": user_info_extra
        },
        "active_status": {
            "status_code": 0,
            "data": active_status_data,
            "not_friend_data": not_friend_data,
            "extra": active_status_extra
        }
    }))
}

/// 获取视频分享面板可展示的好友列表。
#[tauri::command]
pub(crate) async fn get_share_friends(
    state: State<'_, AppState>,
    count: Option<usize>,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => return Ok(cookie_required_response()),
    };
    ensure_im_message_listener(state.inner(), client.clone()).await;

    match client.get_im_share_friends(count.unwrap_or(50)).await {
        Ok(response) => Ok(response),
        Err(error) => Ok(api_login_or_verify_error_response(
            &client,
            "获取分享好友失败",
            error,
            "https://www.douyin.com/",
        )
        .await),
    }
}

/// 发送文本私信。
#[tauri::command]
pub(crate) async fn send_friend_message(
    state: State<'_, AppState>,
    to_user_id: Option<String>,
    uid: Option<String>,
    content: String,
) -> Result<serde_json::Value, String> {
    let to_user_id = to_user_id.or(uid).unwrap_or_default();
    if to_user_id.trim().is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "缺少好友数字 uid，无法发送私信"
        }));
    }
    if content.trim().is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "消息内容不能为空"
        }));
    }
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => return Ok(cookie_required_response()),
    };
    ensure_im_message_listener(state.inner(), client.clone()).await;
    match client.send_im_text_message(&to_user_id, &content).await {
        Ok(result) => Ok(json_object_with_success(result)),
        Err(error) => Ok(api_login_or_verify_error_response(
            &client,
            "发送私信失败",
            error,
            "https://www.douyin.com/",
        )
        .await),
    }
}

/// 发送视频分享卡片私信。
#[tauri::command]
pub(crate) async fn send_friend_video_share(
    state: State<'_, AppState>,
    to_user_id: Option<String>,
    uid: Option<String>,
    video: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let to_user_id = to_user_id.or(uid).unwrap_or_default();
    if to_user_id.trim().is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "缺少好友数字 uid，无法分享视频"
        }));
    }
    if !video.is_object()
        || video
            .get("aweme_id")
            .or_else(|| video.get("itemId"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Ok(serde_json::json!({
            "success": false,
            "message": "缺少作品信息，无法分享视频"
        }));
    }
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => return Ok(cookie_required_response()),
    };
    ensure_im_message_listener(state.inner(), client.clone()).await;
    match client.send_im_video_share_message(&to_user_id, video).await {
        Ok(result) => Ok(json_object_with_success(result)),
        Err(error) => Ok(api_login_or_verify_error_response(
            &client,
            "分享视频失败",
            error,
            "https://www.douyin.com/",
        )
        .await),
    }
}

/// 发送图片私信。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_friend_image_message(
    state: State<'_, AppState>,
    to_user_id: Option<String>,
    uid: Option<String>,
    image_data_url: Option<String>,
    image_data: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    file_name: Option<String>,
    mime_type: Option<String>,
) -> Result<serde_json::Value, String> {
    let to_user_id = to_user_id.or(uid).unwrap_or_default();
    if to_user_id.trim().is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "缺少好友数字 uid，无法发送图片"
        }));
    }
    let image_data_url = image_data_url.or(image_data).unwrap_or_default();
    if image_data_url.trim().is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "图片内容不能为空"
        }));
    }
    if image_data_url.len() > 8 * 1024 * 1024 {
        return Ok(serde_json::json!({
            "success": false,
            "message": "图片不能超过 8MB"
        }));
    }
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => return Ok(cookie_required_response()),
    };
    ensure_im_message_listener(state.inner(), client.clone()).await;
    match client
        .send_im_image_message(
            &to_user_id,
            &image_data_url,
            width.unwrap_or_default(),
            height.unwrap_or_default(),
            file_name.as_deref().unwrap_or_default(),
            mime_type.as_deref().unwrap_or_default(),
        )
        .await
    {
        Ok(result) => Ok(json_object_with_success(result)),
        Err(error) => Ok(api_login_or_verify_error_response(
            &client,
            "发送图片私信失败",
            error,
            "https://www.douyin.com/",
        )
        .await),
    }
}

/// 获取最近的 IM 历史消息。
#[tauri::command]
pub(crate) async fn get_friend_message_history(
    state: State<'_, AppState>,
    cursor: Option<i64>,
    to_user_id: Option<String>,
    uid: Option<String>,
    conversation_id: Option<String>,
    conversation_short_id: Option<serde_json::Value>,
    conversation_type: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => return Ok(cookie_required_response()),
    };
    ensure_im_message_listener(state.inner(), client.clone()).await;
    let to_user_id = to_user_id.or(uid);
    let conversation_short_id = coerce_i64(conversation_short_id.as_ref(), 0);
    let conversation_type = coerce_i64(conversation_type.as_ref(), 1).max(1);
    log::debug!(
        "get_friend_message_history invoked: cursor={} to_user_id_present={} conversation_id_present={} conversation_short_id={}",
        cursor.unwrap_or_default().max(0),
        to_user_id.as_ref().map(|value| !value.trim().is_empty()).unwrap_or(false),
        conversation_id.as_ref().map(|value| !value.trim().is_empty()).unwrap_or(false),
        conversation_short_id
    );

    match client
        .get_im_history_messages(
            cursor.unwrap_or_default().max(0),
            to_user_id.as_deref(),
            conversation_id.as_deref(),
            if conversation_short_id > 0 {
                Some(conversation_short_id)
            } else {
                None
            },
            conversation_type,
        )
        .await
    {
        Ok(result) => {
            let count = result
                .get("messages")
                .and_then(|value| value.as_array())
                .map(|items| items.len())
                .unwrap_or_default();
            log::debug!(
                "get_friend_message_history completed: messages={} next_cursor={}",
                count,
                result.get("next_cursor").cloned().unwrap_or_default()
            );
            Ok(json_object_with_success(result))
        }
        Err(error) => Ok(api_login_or_verify_error_response(
            &client,
            "获取历史消息失败",
            error,
            "https://www.douyin.com/",
        )
        .await),
    }
}

/// 读取好友聊天列表状态。
#[tauri::command]
pub(crate) async fn get_friend_chat_state(
    current_sec_uid: Option<String>,
) -> Result<serde_json::Value, String> {
    let path = friend_chat_state_path(current_sec_uid.as_deref());
    if !path.exists() {
        return Ok(serde_json::json!({
            "success": true,
            "summaries": {},
            "unreadCounts": {}
        }));
    }
    match fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
    {
        Some(value) => {
            let state = sanitize_friend_chat_state(value);
            Ok(json_object_with_success(state))
        }
        None => Ok(serde_json::json!({
            "success": true,
            "summaries": {},
            "unreadCounts": {}
        })),
    }
}

/// 保存好友聊天列表状态。
#[tauri::command]
pub(crate) async fn save_friend_chat_state(
    payload: serde_json::Value,
    current_sec_uid: Option<String>,
) -> Result<serde_json::Value, String> {
    let state = sanitize_friend_chat_state(payload);
    let path = friend_chat_state_path(current_sec_uid.as_deref());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("保存好友聊天状态失败: {}", error))?;
    }
    let temp_path = path.with_extension("json.tmp");
    let mut content = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("保存好友聊天状态失败: {}", error))?;
    content.push('\n');
    fs::write(&temp_path, content).map_err(|error| format!("保存好友聊天状态失败: {}", error))?;
    fs::rename(&temp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!("保存好友聊天状态失败: {}", error)
    })?;
    Ok(serde_json::json!({"success": true}))
}
