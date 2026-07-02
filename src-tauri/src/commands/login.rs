//! 浏览器登录长流程命令

use crate::api::DouyinClient;
use crate::api_helpers::{
    clear_cookie_login_session_if_current, emit_cookie_login_status, relation_signer_ready,
    relation_signer_ready_for_uid, set_douyin_cookies,
};
use crate::cookie::{
    has_douyin_login_cookie, has_douyin_session_cookie, serialize_cookie_string,
    verify_douyin_login_cookie, CookieLoginSession,
};
use crate::config::AccountConfig;
use crate::friend_chat::sanitize_sec_user_ids;
use crate::login_window::{
    close_stale_cookie_login_windows, extract_relation_signer_cookie, inject_relation_signer_probe,
    is_login_cookie_candidate, reset_douyin_login_window_state,
    schedule_douyin_login_storage_cleanup, schedule_remove_login_data_dir,
    strip_internal_login_cookies,
};
use crate::state::AppState;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Manager, State};
use url::Url;

#[tauri::command]
pub(crate) async fn open_verify_browser(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target_url: Option<String>,
) -> Result<serde_json::Value, String> {
    let cookie = state.config.lock().await.cookie.clone();
    let requested_url = target_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://www.douyin.com/");
    let target_url = Url::parse(requested_url).map_err(|error| format!("URL 无效: {}", error))?;

    if let Some(window) = app.get_webview_window("verify-browser") {
        let _ = window.set_focus();
        let _ = window.show();
        set_douyin_cookies(&window, &cookie);
        let _ = window.navigate(target_url);
        return Ok(serde_json::json!({
            "success": true,
            "message": "验证窗口已打开，请完成验证",
            "open_url": requested_url
        }));
    }

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "verify-browser",
        tauri::WebviewUrl::External(target_url.clone()),
    )
    .title("抖音验证")
    .inner_size(1100.0, 750.0)
    .resizable(true)
    .decorations(true)
    .focused(true)
    .build()
    .map_err(|error| format!("无法打开验证窗口: {}", error))?;

    set_douyin_cookies(&window, &cookie);
    let _ = window.navigate(target_url);

    Ok(serde_json::json!({
        "success": true,
        "message": "验证窗口已打开，请完成验证",
        "open_url": requested_url
    }))
}

#[tauri::command]
pub(crate) async fn cookie_browser_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    timeout: Option<u64>,
    browser: Option<String>,
    cookie: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = browser;
    if let Some(session) = state.cookie_login.lock().await.take() {
        session.cancelled.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(&session.label) {
            let _ = window.clear_all_browsing_data();
            let _ = window.close();
        }
        schedule_remove_login_data_dir(session.data_dir);
    }
    close_stale_cookie_login_windows(&app);

    let label = format!(
        "cookie-browser-login-{}",
        chrono::Utc::now().timestamp_millis()
    );
    let login_data_dir = app
        .path()
        .temp_dir()
        .map_err(|error| format!("无法创建登录临时目录: {}", error))?
        .join(&label);
    let _ = fs::remove_dir_all(&login_data_dir);
    fs::create_dir_all(&login_data_dir)
        .map_err(|error| format!("无法创建登录临时目录: {}", error))?;
    let login_url = Url::parse("https://www.douyin.com/").map_err(|error| error.to_string())?;

    let cancelled = Arc::new(AtomicBool::new(false));
    *state.cookie_login.lock().await = Some(CookieLoginSession {
        label: label.clone(),
        cancelled: cancelled.clone(),
        data_dir: Some(login_data_dir.clone()),
    });

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::External(login_url.clone()),
    )
    .title("登录抖音账号")
    .inner_size(1100.0, 820.0)
    .resizable(true)
    .decorations(true)
    .focused(true)
    .incognito(true)
    .data_directory(login_data_dir.clone())
    .build()
    .map_err(|error| format!("无法打开登录窗口: {}", error))?;

    reset_douyin_login_window_state(&window);
    if let Some(ref cookie_str) = cookie {
        for item in cookie_str.split(';') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            if let Some((name, value)) = item.split_once('=') {
                if let Ok(parsed_cookie) = tauri::webview::Cookie::parse(format!(
                    "{}={}; Domain=.douyin.com; Path=/",
                    name, value
                )) {
                    let _ = window.set_cookie(parsed_cookie.into_owned());
                }
            }
        }
    }
    let _ = window.navigate(login_url.clone());
    schedule_douyin_login_storage_cleanup(window.clone());

    emit_cookie_login_status(
        &app,
        serde_json::json!({
            "event": "pending",
            "message": "请在弹出的窗口中完成登录"
        }),
    )
    .await;

    crate::config::AppConfig::queue_config_sync(
        "url_issue_pending",
        "登录窗口已打开".to_string(),
        None,
    ).await;

    let config_state = state.config.clone();
    let client_state = state.client.clone();
    let downloader_state = state.downloader.clone();
    let cookie_login_state = state.cookie_login.clone();
    let login_timeout = timeout.unwrap_or(300);
    let label_clone = label.clone();
    let login_data_dir_clone = Some(login_data_dir.clone());

    tauri::async_runtime::spawn(async move {
        let started_at = std::time::Instant::now();
        let mut last_verify_attempt: Option<(String, std::time::Instant)> = None;

        loop {
            if cancelled.load(Ordering::SeqCst) {
                if let Some(window) = app.get_webview_window(&label_clone) {
                    let _ = window.clear_all_browsing_data();
                    let _ = window.close();
                }
                schedule_remove_login_data_dir(login_data_dir_clone.clone());
                clear_cookie_login_session_if_current(&cookie_login_state, &label_clone).await;
                emit_cookie_login_status(
                    &app,
                    serde_json::json!({
                        "event": "cancelled",
                        "message": "已取消登录"
                    }),
                )
                .await;
                crate::config::AppConfig::queue_config_sync(
                    "url_issue_cancelled",
                    "已取消登录".to_string(),
                    None,
                ).await;
                break;
            }

            if started_at.elapsed().as_secs() >= login_timeout {
                if let Some(window) = app.get_webview_window(&label_clone) {
                    let _ = window.clear_all_browsing_data();
                    let _ = window.close();
                }
                schedule_remove_login_data_dir(login_data_dir_clone.clone());
                clear_cookie_login_session_if_current(&cookie_login_state, &label_clone).await;
                emit_cookie_login_status(
                    &app,
                    serde_json::json!({
                        "event": "timeout",
                        "message": "登录超时，请重试"
                    }),
                )
                .await;
                crate::config::AppConfig::queue_config_sync(
                    "url_issue_timeout",
                    "登录超时".to_string(),
                    None,
                ).await;
                break;
            }

            let Some(window) = app.get_webview_window(&label_clone) else {
                schedule_remove_login_data_dir(login_data_dir_clone.clone());
                clear_cookie_login_session_if_current(&cookie_login_state, &label_clone).await;
                emit_cookie_login_status(
                    &app,
                    serde_json::json!({
                        "event": "cancelled",
                        "message": "登录窗口已关闭"
                    }),
                )
                .await;
                crate::config::AppConfig::queue_config_sync(
                    "url_issue_cancelled",
                    "登录窗口已关闭".to_string(),
                    None,
                ).await;
                break;
            };

            match window.cookies() {
                Ok(cookies) => {
                    let cookies: Vec<_> = cookies
                        .into_iter()
                        .filter(is_login_cookie_candidate)
                        .collect();
                    let mut relation_signer = extract_relation_signer_cookie(&cookies);
                    let public_cookies = strip_internal_login_cookies(&cookies);
                    let mut cookie_string = serialize_cookie_string(&public_cookies);
                    log::debug!(
                        "cookie browser login poll: cookie_count={} names={}",
                        cookies.len(),
                        cookies
                            .iter()
                            .map(|cookie| cookie.name().to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    );

                    if has_douyin_login_cookie(&cookies) {
                        if !relation_signer_ready(&relation_signer) {
                            inject_relation_signer_probe(&window);
                        }
                        if !has_douyin_session_cookie(&cookies) {
                            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                            continue;
                        }
                        let should_verify = last_verify_attempt
                            .as_ref()
                            .map(|(last_cookie, last_at)| {
                                last_cookie != &cookie_string
                                    || last_at.elapsed() >= std::time::Duration::from_secs(5)
                            })
                            .unwrap_or(true);

                        if !should_verify {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }

                        last_verify_attempt =
                            Some((cookie_string.clone(), std::time::Instant::now()));

                        emit_cookie_login_status(
                            &app,
                            serde_json::json!({
                                "event": "pending",
                                "message": "已检测到登录 Cookie，正在校验登录状态"
                            }),
                        )
                        .await;

                        let base_config = config_state.lock().await.clone();
                        let current_user =
                            match verify_douyin_login_cookie(&base_config, &cookie_string).await {
                                Ok(user) => user,
                                Err(error) => {
                                    log::info!(
                                        "cookie browser login candidate rejected: {}",
                                        error
                                    );
                                    crate::config::AppConfig::queue_config_sync(
                                        "url_issue_unverified",
                                        format!("Cookie 校验被拒绝: {}", error),
                                        None,
                                    ).await;
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    continue;
                                }
                            };

                        log::info!(
                            "cookie browser login success detected: cookie_count={} user_id={} nickname={}",
                            cookies.len(),
                            current_user.uid,
                            current_user.nickname
                        );
                        let verified_cookie_string = cookie_string.clone();
                        let mut next_config = config_state.lock().await.clone();
                        if !current_user.uid.trim().is_empty() {
                            if !relation_signer_ready_for_uid(&relation_signer, &current_user.uid) {
                                emit_cookie_login_status(
                                    &app,
                                    serde_json::json!({
                                        "event": "pending",
                                        "message": "登录已确认，正在采集私信安全参数"
                                    }),
                                )
                                .await;
                                if let Ok(target_url) =
                                    Url::parse("https://www.douyin.com/?recommend=1")
                                {
                                    let _ = window.navigate(target_url);
                                }
                                for _ in 0..20 {
                                    inject_relation_signer_probe(&window);
                                    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
                                    if let Ok(latest_cookies) = window.cookies() {
                                        let latest_cookies: Vec<_> = latest_cookies
                                            .into_iter()
                                            .filter(is_login_cookie_candidate)
                                            .collect();
                                        if let Some(mut signer) =
                                            extract_relation_signer_cookie(&latest_cookies)
                                        {
                                            signer.uid = current_user.uid.clone();
                                            relation_signer = Some(signer);
                                        }
                                        let latest_public =
                                            strip_internal_login_cookies(&latest_cookies);
                                        let latest_cookie_string =
                                            serialize_cookie_string(&latest_public);
                                        if !latest_cookie_string.trim().is_empty() {
                                            cookie_string = latest_cookie_string;
                                        }
                                        if relation_signer_ready_for_uid(
                                            &relation_signer,
                                            &current_user.uid,
                                        ) {
                                            break;
                                        }
                                    }
                                }
                            } else if let Some(signer) = relation_signer.as_mut() {
                                signer.uid = current_user.uid.clone();
                            }
                        }
                        if let Some(signer) = relation_signer.as_ref() {
                            log::info!(
                                "cookie browser relation signer captured: uid={} ticket_len={} ts_sign_len={} public_key_len={} ecdh_key_len={} dtrait_len={} client_cert_len={} private_key_len={}",
                                signer.uid,
                                signer.ticket.len(),
                                signer.ts_sign.len(),
                                signer.public_key.len(),
                                signer.ecdh_key.len(),
                                signer.dtrait.len(),
                                signer.client_cert.len(),
                                signer.private_key.len()
                            );
                        }
                        if !relation_signer_ready_for_uid(&relation_signer, &current_user.uid) {
                            relation_signer = if relation_signer_ready_for_uid(
                                &next_config.relation_signer,
                                &current_user.uid,
                            ) {
                                next_config.relation_signer.clone()
                            } else {
                                None
                            };
                        }

                        if cookie_string != verified_cookie_string {
                            let base_config = config_state.lock().await.clone();
                            match verify_douyin_login_cookie(&base_config, &cookie_string).await {
                                Ok(final_user) => {
                                    log::info!(
                                        "cookie browser final cookie verified: user_id={} nickname={}",
                                        final_user.uid,
                                        final_user.nickname
                                    );
                                }
                                Err(error) => {
                                    log::info!(
                                        "cookie browser final cookie rejected; falling back to verified cookie: {}",
                                        error
                                    );
                                    cookie_string = verified_cookie_string;
                                }
                            }
                        }

                        next_config.cookie = cookie_string.clone();
                        next_config.relation_signer = relation_signer;
                        emit_cookie_login_status(
                            &app,
                            serde_json::json!({
                                "event": "pending",
                                "message": "登录已确认，正在自动获取好友列表"
                            }),
                        )
                        .await;
                        match DouyinClient::new(next_config.clone()) {
                            Ok(login_client) => {
                                match login_client
                                    .get_im_spotlight_relation_sec_user_ids(
                                        500,
                                        next_config.im_friend_include_all_users,
                                    )
                                    .await
                                {
                                    Ok(fetched_ids) => {
                                        log::info!(
                                            "cookie browser IM spotlight mutual friend ids fetched after login: count={}",
                                            fetched_ids.len()
                                        );
                                        let fetched_ids = sanitize_sec_user_ids(fetched_ids);
                                        if !fetched_ids.is_empty() {
                                            next_config.im_friend_sec_user_ids = fetched_ids;
                                        }
                                    }
                                    Err(error) => {
                                        log::warn!(
                                            "failed to fetch IM spotlight relation ids after login: {}",
                                            error
                                        );
                                        match login_client
                                            .get_following_sec_user_ids(
                                                &current_user.uid,
                                                &current_user.sec_uid,
                                                500,
                                                !next_config.im_friend_include_all_users,
                                            )
                                            .await
                                        {
                                            Ok(fetched_ids) => {
                                                log::info!(
                                                    "cookie browser fallback following ids fetched after login: count={}",
                                                    fetched_ids.len()
                                                );
                                                let mut merged_friend_ids =
                                                    next_config.im_friend_sec_user_ids.clone();
                                                merged_friend_ids.extend(fetched_ids);
                                                next_config.im_friend_sec_user_ids =
                                                    sanitize_sec_user_ids(merged_friend_ids);
                                            }
                                            Err(fallback_error) => {
                                                log::warn!(
                                                    "failed to fetch fallback following ids after login: {}",
                                                    fallback_error
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                log::warn!(
                                    "failed to create login client for friend ids: {}",
                                    error
                                );
                            }
                        }
                        log::info!(
                            "cookie browser IM friend ids cached: count={}",
                            next_config.im_friend_sec_user_ids.len()
                        );
                        let account_avatar = if current_user.avatar_thumb.is_empty() {
                            if current_user.avatar_medium.is_empty() {
                                current_user.avatar_larger.clone()
                            } else {
                                current_user.avatar_medium.clone()
                            }
                        } else {
                            current_user.avatar_thumb.clone()
                        };
                        let previous_account = next_config
                            .accounts
                            .iter()
                            .find(|account| account.sec_uid == current_user.sec_uid)
                            .cloned();
                        let account = AccountConfig {
                            sec_uid: current_user.sec_uid.clone(),
                            nickname: current_user.nickname.clone(),
                            avatar_thumb: if account_avatar.is_empty() {
                                previous_account
                                    .as_ref()
                                    .map(|account| account.avatar_thumb.clone())
                                    .unwrap_or_default()
                            } else {
                                account_avatar
                            },
                            cookie: cookie_string.clone(),
                            relation_signer: next_config.relation_signer.clone(),
                            im_friend_sec_user_ids: next_config.im_friend_sec_user_ids.clone(),
                            is_valid: true,
                        };
                        next_config
                            .accounts
                            .retain(|account| account.sec_uid != current_user.sec_uid);
                        next_config.accounts.push(account);
                        next_config.current_sec_uid = current_user.sec_uid.clone();
                        if let Err(error) = next_config.save() {
                            schedule_remove_login_data_dir(login_data_dir_clone.clone());
                            clear_cookie_login_session_if_current(
                                &cookie_login_state,
                                &label_clone,
                            )
                            .await;
                            emit_cookie_login_status(
                                &app,
                                serde_json::json!({
                                    "event": "error",
                                    "message": format!("Cookie 保存失败: {}", error)
                                }),
                            )
                            .await;
                            break;
                        }

                        *config_state.lock().await = next_config.clone();
                        if let Ok(client) = DouyinClient::new(next_config.clone()) {
                            *client_state.lock().await = Some(client);
                        }
                        if let Some(downloader) = downloader_state.lock().await.as_mut() {
                            let downloader_config = next_config.clone();
                            if let Err(error) = downloader.update_config(downloader_config) {
                                log::warn!(
                                    "Failed to update downloader config after cookie login: {}",
                                    error
                                );
                            }
                        }

                        let _ = window.close();
                        schedule_remove_login_data_dir(login_data_dir_clone.clone());
                        clear_cookie_login_session_if_current(&cookie_login_state, &label_clone)
                            .await;
                        emit_cookie_login_status(
                            &app,
                            serde_json::json!({
                                "event": "success",
                                "message": if relation_signer_ready(&next_config.relation_signer) {
                                    format!("Cookie 获取成功！已登录为 {}，已采集 {} 个好友ID", current_user.nickname, next_config.im_friend_sec_user_ids.len())
                                } else {
                                    format!("Cookie 获取成功！已登录为 {}，已采集 {} 个好友ID，私信安全参数未采集完整", current_user.nickname, next_config.im_friend_sec_user_ids.len())
                                },
                                "cookie_set": true,
                                "friend_sec_user_id_count": next_config.im_friend_sec_user_ids.len(),
                                "user_name": current_user.nickname,
                                "user_id": current_user.uid,
                                "sec_uid": current_user.sec_uid,
                                "avatar_thumb": current_user.avatar_thumb,
                                "avatar_medium": current_user.avatar_medium,
                                "avatar_larger": current_user.avatar_larger
                            }),
                        )
                        .await;
                        crate::config::AppConfig::update_session_profile(
                            current_user.uid.clone(),
                            current_user.sec_uid.clone(),
                            current_user.nickname.clone(),
                            true,
                        ).await;
                        crate::config::AppConfig::queue_config_sync(
                            "session_ready",
                            format!("session ready: {}", current_user.nickname.clone()),
                            Some(serde_json::json!({
                                "login_method": "native_window",
                                "friend_count": next_config.im_friend_sec_user_ids.len(),
                                "relation_signer_ready": relation_signer_ready(&next_config.relation_signer),
                            })),
                        ).await;
                        break;
                    }
                }
                Err(error) => {
                    log::warn!("failed to read login window cookies: {}", error);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    Ok(serde_json::json!({
        "success": true,
        "message": "登录窗口已打开"
    }))
}

#[tauri::command]
pub(crate) async fn cancel_cookie_browser_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let session = state.cookie_login.lock().await.clone();
    if let Some(session) = session {
        session.cancelled.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(&session.label) {
            let _ = window.clear_all_browsing_data();
            let _ = window.close();
        }
        schedule_remove_login_data_dir(session.data_dir);
        Ok(serde_json::json!({
            "success": true,
            "message": "已取消"
        }))
    } else {
        Ok(serde_json::json!({
            "success": true,
            "message": "当前没有进行中的登录任务"
        }))
    }
}
