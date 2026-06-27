//! API 请求辅助函数：登录校验、错误判断、响应构建

use crate::api::DouyinClient;
use crate::config::RelationSignerConfig;
use crate::cookie::{has_douyin_login_cookie, parse_cookie_string};
use crate::state::AppState;
use tauri::{Emitter, State};

pub(crate) fn looks_like_login_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    message.contains("用户未登录")
        || message.contains("未登录")
        || message.contains("登录态")
        || message.contains("重新登录")
        || message.contains("请先设置Cookie")
        || message.contains("请先设置 Cookie")
        || message.contains("Cookie 为空")
        || lower.contains("error decoding response body")
        || lower.contains("expected value")
        || lower.contains("invalid type")
        || lower.contains("text/html")
        || lower.contains("not login")
        || lower.contains("not logged in")
        || lower.contains("login required")
        || lower.contains("session expired")
}

pub(crate) fn looks_like_verify_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    message.contains("验证")
        || message.contains("风控")
        || message.contains("访问频繁")
        || message.contains("请稍后重试")
        || lower.contains("verify")
        || lower.contains("captcha")
        || lower.contains("passport")
}

pub(crate) fn normalize_recommended_feed_type(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "recommended" | "recommend" | "tab" | "home" | "feed" => "recommended",
        _ => "featured",
    }
}

pub(crate) fn looks_like_relation_security_error(message: &str) -> bool {
    message.contains("RELATION_SECURITY_GATEWAY")
        || message.contains("bd-ticket-guard")
        || message.contains("安全校验拒绝")
}

pub(crate) fn relation_security_blocked_response(prefix: &str, message: &str) -> serde_json::Value {
    let hint = if message.trim().is_empty() {
        "抖音安全校验拒绝了本次操作，请稍后重试，或先在抖音网页/客户端完成一次同类操作后再回来使用。"
    } else {
        message
    };

    serde_json::json!({
        "success": false,
        "security_blocked": true,
        "message": format!("{}: {}", prefix, hint)
    })
}

pub(crate) fn login_required_message(message: &str) -> String {
    if message.trim().is_empty() || looks_like_login_error(message) {
        "用户未登录，请在设置中重新登录并刷新 Cookie".to_string()
    } else {
        format!("登录态校验失败: {}", message)
    }
}

pub(crate) fn login_required_response(message: &str) -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "need_login": true,
        "message": login_required_message(message)
    })
}

pub(crate) fn cookie_required_response() -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "need_login": true,
        "message": "请先设置Cookie"
    })
}

pub(crate) fn feature_login_required_response(feature: &str) -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "need_login": true,
        "message": format!("请登录后获取{}", feature)
    })
}

pub(crate) async fn state_has_login_cookie(state: &State<'_, AppState>) -> bool {
    let config = state.config.lock().await;
    has_douyin_login_cookie(&parse_cookie_string(&config.cookie))
}

pub(crate) async fn ensure_feature_login(
    state: &State<'_, AppState>,
    client: &DouyinClient,
    feature: &str,
) -> Option<serde_json::Value> {
    if !state_has_login_cookie(state).await {
        return Some(feature_login_required_response(feature));
    }

    match client.verify_cookie().await {
        Ok(status) if status.valid => None,
        Ok(_) | Err(_) => Some(feature_login_required_response(feature)),
    }
}

pub(crate) fn verify_required_response(message: &str, verify_url: &str) -> serde_json::Value {
    let message = if message.trim().is_empty() {
        "需要完成滑块验证后重试"
    } else {
        message
    };

    serde_json::json!({
        "success": false,
        "need_verify": true,
        "verify_url": verify_url,
        "message": message
    })
}

pub(crate) async fn login_required_if_cookie_invalid(client: &DouyinClient) -> Option<serde_json::Value> {
    match client.verify_cookie().await {
        Ok(status) if status.valid => None,
        Ok(status) => Some(login_required_response(&status.message)),
        Err(error) => Some(login_required_response(&error.to_string())),
    }
}

pub(crate) async fn login_or_verify_response(
    client: &DouyinClient,
    message: &str,
    verify_url: &str,
) -> serde_json::Value {
    if let Some(response) = login_required_if_cookie_invalid(client).await {
        response
    } else {
        verify_required_response(message, verify_url)
    }
}

pub(crate) async fn api_login_or_verify_error_response(
    client: &DouyinClient,
    prefix: &str,
    error: impl std::fmt::Display,
    verify_url: &str,
) -> serde_json::Value {
    let message = error.to_string();
    let user_message = prefixed_error_message(prefix, &message);
    if looks_like_relation_security_error(&message) {
        relation_security_blocked_response(prefix, &message)
    } else if looks_like_login_error(&message) || looks_like_verify_error(&message) {
        login_or_verify_response(client, &user_message, verify_url).await
    } else {
        serde_json::json!({
            "success": false,
            "message": user_message
        })
    }
}

pub(crate) fn api_verify_or_error_response(
    prefix: &str,
    error: impl std::fmt::Display,
    verify_url: &str,
) -> serde_json::Value {
    let message = error.to_string();
    let user_message = prefixed_error_message(prefix, &message);
    if looks_like_relation_security_error(&message) {
        relation_security_blocked_response(prefix, &message)
    } else if looks_like_login_error(&message) || looks_like_verify_error(&message) {
        verify_required_response(&user_message, verify_url)
    } else {
        serde_json::json!({
            "success": false,
            "message": user_message
        })
    }
}

pub(crate) fn prefixed_error_message(prefix: &str, message: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        return prefix.to_string();
    }
    if message == prefix
        || message.starts_with(&format!("{}: ", prefix))
        || message.starts_with(&format!("{}：", prefix))
    {
        message.to_string()
    } else {
        format!("{}: {}", prefix, message)
    }
}

pub(crate) async fn get_client(state: &State<'_, AppState>) -> Result<DouyinClient, String> {
    state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Client not initialized".to_string())
}

pub(crate) fn relation_signer_ready(signer: &Option<RelationSignerConfig>) -> bool {
    signer
        .as_ref()
        .map(|signer| !signer.dtrait.trim().is_empty())
        .unwrap_or(false)
}

pub(crate) fn relation_signer_ready_for_uid(signer: &Option<RelationSignerConfig>, uid: &str) -> bool {
    let uid = uid.trim();
    signer
        .as_ref()
        .map(|signer| {
            !uid.is_empty()
                && signer.uid.trim() == uid
                && !signer.ticket.trim().is_empty()
                && !signer.ts_sign.trim().is_empty()
                && !signer.public_key.trim().is_empty()
                && !signer.ecdh_key.trim().is_empty()
                && !signer.dtrait.trim().is_empty()
        })
        .unwrap_or(false)
}

pub(crate) fn set_douyin_cookies(window: &tauri::WebviewWindow, cookie_string: &str) {
    let mut count = 0usize;
    for cookie in parse_cookie_string(cookie_string) {
        if window.set_cookie(cookie).is_ok() {
            count += 1;
        }
    }

    for item in cookie_string.split(';') {
        let item = item.trim();
        let Some((name, value)) = item.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        for domain in [".douyin.com", "www.douyin.com"] {
            if let Ok(cookie) = tauri::webview::Cookie::parse(format!(
                "{}={}; Domain={}; Path=/; Secure; SameSite=None",
                name,
                value.trim(),
                domain
            )) {
                let _ = window.set_cookie(cookie.into_owned());
            }
        }
    }
    log::info!("injected {} saved douyin cookies into webview", count);
}

pub(crate) async fn emit_cookie_login_status(app: &tauri::AppHandle, payload: serde_json::Value) {
    let _ = app.emit("cookie-login-status", payload);
}

pub(crate) async fn clear_cookie_login_session_if_current(
    cookie_login_state: &std::sync::Arc<tokio::sync::Mutex<Option<crate::cookie::CookieLoginSession>>>,
    label: &str,
) {
    let mut guard = cookie_login_state.lock().await;
    if guard
        .as_ref()
        .map(|session| session.label.as_str())
        .is_some_and(|current_label| current_label == label)
    {
        *guard = None;
    }
}
