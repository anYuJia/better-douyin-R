//! IM WebSocket 消息监听器

use crate::api;
use crate::api::DouyinClient;
use crate::state::AppState;
use futures::StreamExt;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn extract_im_text_message(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|parsed| {
            if let Some(parsed_obj) = parsed.as_object() {
                if parsed_obj.contains_key("command_type") || parsed_obj.get("command_type").and_then(|v| v.as_i64()) == Some(6) {
                    let mut found_spark = false;
                    let mut text = String::new();
                    if let Some(ext_data) = parsed_obj.get("ext_data").and_then(|v| v.as_array()) {
                        for ext_item in ext_data {
                            if let Some(ext_obj) = ext_item.as_object() {
                                if ext_obj.get("key").and_then(|v| v.as_str()) == Some("a:consecutive_chat_data") {
                                    text = "🔥 连续聊天火花已亮起".to_string();
                                    found_spark = true;
                                    if let Some(val_str) = ext_obj.get("value").and_then(|v| v.as_str()) {
                                        if let Ok(val_json) = serde_json::from_str::<serde_json::Value>(val_str) {
                                            if let Some(count_info) = val_json.get("consecutive_count_info") {
                                                let count = count_info.get("consecutive_count").and_then(|v| v.as_i64()).unwrap_or(1);
                                                text = format!("🔥 连续聊天火花已亮起（第 {} 天）", count);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if found_spark {
                        return Some(text);
                    } else {
                        return Some("__FILTERED_CONTROL_MESSAGE__".to_string());
                    }
                }
            }
            parsed
                .get("text")
                .or_else(|| parsed.get("tips"))
                .or_else(|| parsed.get("hint_text"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| content.to_string())
}

fn emit_im_message(app: &tauri::AppHandle, response: &serde_json::Value) {
    let Some(sent) = api::im_proto::sent_message(response) else {
        return;
    };
    let content = extract_im_text_message(&sent.content);
    if content == "__FILTERED_CONTROL_MESSAGE__" || content.is_empty() {
        return;
    }
    let payload = serde_json::json!({
        "conversation_id": sent.conversation_id,
        "conversation_short_id": sent.conversation_short_id,
        "conversation_type": sent.conversation_type,
        "server_message_id": sent.server_message_id,
        "index_in_conversation": sent.index_in_conversation,
        "sender_uid": sent.sender.to_string(),
        "content": content,
        "raw_content": sent.content,
        "created_at": chrono::Utc::now().timestamp_millis(),
    });
    log::debug!(
        "Douyin IM websocket message: conversation={} sender={} message_id={} text_len={}",
        payload
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        payload
            .get("sender_uid")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        payload
            .get("server_message_id")
            .and_then(|value| value.as_i64())
            .unwrap_or_default(),
        content.len(),
    );
    let _ = app.emit("im-message", payload);
}

fn emit_im_status(app: &tauri::AppHandle, connected: bool, message: impl Into<String>) {
    let _ = app.emit(
        "im-status",
        serde_json::json!({
            "connected": connected,
            "message": message.into(),
            "updated_at": chrono::Utc::now().timestamp_millis(),
        }),
    );
}

async fn run_im_message_listener(
    app: tauri::AppHandle,
    client: DouyinClient,
) -> anyhow::Result<()> {
    let Some(sessionid) = client.im_session_id() else {
        log::info!("IM WebSocket not started: saved cookie has no sessionid");
        emit_im_status(&app, false, "Cookie 缺少 sessionid，私信接收未启动");
        return Ok(());
    };
    let cookie = client.cookie().trim().to_string();
    if cookie.is_empty() {
        emit_im_status(&app, false, "Cookie 为空，私信接收未启动");
        return Ok(());
    }
    emit_im_status(&app, false, "正在连接私信接收");
    let device_id = client.get_im_device_id().await?;
    let app_key = "e1bd35ec9db7b8d846de66ed140b1ad9";
    let fp_id = "9";
    let access_key = format!(
        "{:x}",
        md5::compute(format!("{fp_id}{app_key}{device_id}f8a69f1719916z").as_bytes())
    );
    let params = serde_urlencoded::to_string(HashMap::from([
        ("aid", "6383".to_string()),
        ("device_platform", "douyin_pc".to_string()),
        ("fpid", fp_id.to_string()),
        ("device_id", device_id),
        ("token", sessionid),
        ("access_key", access_key),
    ]))?;
    let url = format!("wss://frontier-im.douyin.com/ws/v2?{params}");
    let mut request = url.into_client_request()?;
    let headers = request.headers_mut();
    headers.insert("Pragma", "no-cache".parse()?);
    headers.insert(
        "Accept-Language",
        "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6".parse()?,
    );
    headers.insert("User-Agent", crate::config::get_user_agent().parse()?);
    headers.insert("Cache-Control", "no-cache".parse()?);
    headers.insert("Sec-WebSocket-Protocol", "pbbp2".parse()?);
    headers.insert(
        "Sec-WebSocket-Extensions",
        "permessage-deflate; client_max_window_bits".parse()?,
    );
    headers.insert("Cookie", cookie.parse()?);
    headers.insert("Origin", "https://www.douyin.com".parse()?);

    let (mut ws, _) = tokio_tungstenite::connect_async(request).await?;
    log::info!("Douyin IM WebSocket connected");
    emit_im_status(&app, true, "私信接收已连接");
    while let Some(message) = ws.next().await {
        let message = message?;
        if message.is_binary() {
            let frame = api::im_proto::parse_push_frame(&message.into_data());
            if let Some(response) = frame.get("response").filter(|value| value.is_object()) {
                emit_im_message(&app, response);
            }
        } else if message.is_text() {
            log::debug!("Douyin IM WebSocket text: {}", message.into_text()?);
        }
    }
    log::info!("Douyin IM WebSocket disconnected");
    emit_im_status(&app, false, "私信接收已断开");
    Ok(())
}

pub(crate) async fn ensure_im_message_listener(state: &AppState, client: DouyinClient) {
    let app = state.app_handle.lock().await.clone();
    let Some(app) = app else {
        return;
    };
    let mut listener = state.im_message_listener.lock().await;
    if listener
        .as_ref()
        .map(|handle| !handle.is_finished())
        .unwrap_or(false)
    {
        return;
    }
    let mut attempted_at = state.im_message_listener_attempted_at.lock().await;
    if attempted_at
        .as_ref()
        .map(|instant| instant.elapsed() < Duration::from_secs(10))
        .unwrap_or(false)
    {
        return;
    }
    *attempted_at = Some(Instant::now());
    drop(attempted_at);
    *listener = Some(tokio::spawn(async move {
        if let Err(error) = run_im_message_listener(app.clone(), client).await {
            log::warn!("Douyin IM WebSocket listener exited: {}", error);
            emit_im_status(&app, false, format!("私信接收连接错误: {error}"));
        }
    }));
}
