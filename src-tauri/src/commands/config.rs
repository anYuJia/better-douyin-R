//! 配置、初始化、账号校验、当前用户相关命令

use crate::api::{CookieStatus, DouyinClient, UserInfo};
use crate::config::{AccountConfig, AppConfig};
use crate::downloader::{Downloader, DownloaderEvent};
use crate::http_client::build_media_http_client;
use crate::login_window::{
    clear_douyin_login_cookies, close_stale_cookie_login_windows, schedule_remove_login_data_dir,
};
use crate::AppState;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager, State};
use tokio::sync::mpsc;

/// 初始化客户端
#[tauri::command]
pub(crate) async fn init_client(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let config = state.config.lock().await.clone();

    let client = DouyinClient::new(config.clone()).map_err(|e| e.to_string())?;

    let (tx, mut rx) = mpsc::channel::<DownloaderEvent>(100);

    let downloader = Downloader::new(config, Some(tx)).map_err(|e| e.to_string())?;

    let app_handle = state.app_handle.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let current_app_handle = app_handle.lock().await.clone();
            if let Some(app_handle) = current_app_handle {
                let _ = app_handle.emit(event.name, event.payload);
            }
        }
    });

    *state.client.lock().await = Some(client);
    *state.downloader.lock().await = Some(downloader);

    Ok(serde_json::json!({ "success": true }))
}

/// 获取配置
#[tauri::command]
pub(crate) async fn get_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let config = state.config.lock().await.clone();
    let cookie_set = !config.cookie.trim().is_empty();
    let mut value = serde_json::to_value(&config).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("cookie".to_string(), serde_json::json!(""));
        object.insert("cookie_set".to_string(), serde_json::json!(cookie_set));
    }
    Ok(value)
}

fn public_account_payload(account: &AccountConfig) -> serde_json::Value {
    serde_json::json!({
        "sec_uid": account.sec_uid,
        "nickname": account.nickname,
        "avatar_thumb": account.avatar_thumb,
        "is_valid": account.is_valid,
    })
}

async fn apply_config_to_runtime(
    state: &State<'_, AppState>,
    next_config: AppConfig,
) -> Result<(), String> {
    *state.config.lock().await = next_config.clone();
    match DouyinClient::new(next_config.clone()) {
        Ok(client) => *state.client.lock().await = Some(client),
        Err(error) => {
            *state.client.lock().await = None;
            log::warn!("Failed to rebuild API client after account change: {}", error);
        }
    }
    if let Some(downloader) = state.downloader.lock().await.as_mut() {
        if let Err(error) = downloader.update_config(next_config) {
            log::warn!("Failed to update downloader config after account change: {}", error);
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_accounts(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let config = state.config.lock().await.clone();
    Ok(serde_json::json!({
        "success": true,
        "accounts": config.accounts.iter().map(public_account_payload).collect::<Vec<_>>(),
        "current_sec_uid": config.current_sec_uid,
    }))
}

#[tauri::command]
pub(crate) async fn switch_account(
    state: State<'_, AppState>,
    sec_uid: String,
) -> Result<serde_json::Value, String> {
    let sec_uid = sec_uid.trim().to_string();
    if sec_uid.is_empty() {
        return Ok(serde_json::json!({ "success": false, "message": "缺少必要参数 sec_uid" }));
    }

    let mut next_config = state.config.lock().await.clone();
    let Some(account) = next_config
        .accounts
        .iter()
        .find(|account| account.sec_uid == sec_uid)
        .cloned()
    else {
        return Ok(serde_json::json!({ "success": false, "message": "账号不存在" }));
    };

    next_config.cookie = account.cookie.clone();
    next_config.relation_signer = account.relation_signer.clone();
    next_config.im_friend_sec_user_ids = account.im_friend_sec_user_ids.clone();
    next_config.current_sec_uid = account.sec_uid.clone();
    next_config
        .save()
        .map_err(|error| format!("切换账号失败: {}", error))?;
    AppConfig::update_session_profile(
        String::new(),
        account.sec_uid.clone(),
        account.nickname.clone(),
        true,
    )
    .await;
    apply_config_to_runtime(&state, next_config).await?;

    Ok(serde_json::json!({
        "success": true,
        "message": format!("已切换为 {}", account.nickname),
        "nickname": account.nickname,
    }))
}

#[tauri::command]
pub(crate) async fn delete_account(
    state: State<'_, AppState>,
    sec_uid: String,
) -> Result<serde_json::Value, String> {
    let sec_uid = sec_uid.trim().to_string();
    if sec_uid.is_empty() {
        return Ok(serde_json::json!({ "success": false, "message": "缺少必要参数 sec_uid" }));
    }

    let mut next_config = state.config.lock().await.clone();
    let old_len = next_config.accounts.len();
    next_config.accounts.retain(|account| account.sec_uid != sec_uid);
    if next_config.accounts.len() == old_len {
        return Ok(serde_json::json!({ "success": false, "message": "账号不存在" }));
    }

    if next_config.current_sec_uid == sec_uid {
        if let Some(next_account) = next_config.accounts.first().cloned() {
            next_config.cookie = next_account.cookie.clone();
            next_config.relation_signer = next_account.relation_signer.clone();
            next_config.im_friend_sec_user_ids = next_account.im_friend_sec_user_ids.clone();
            next_config.current_sec_uid = next_account.sec_uid.clone();
        } else {
            next_config.cookie.clear();
            next_config.relation_signer = None;
            next_config.im_friend_sec_user_ids.clear();
            next_config.current_sec_uid.clear();
        }
    }

    next_config
        .save()
        .map_err(|error| format!("删除账号失败: {}", error))?;
    apply_config_to_runtime(&state, next_config).await?;

    Ok(serde_json::json!({ "success": true, "message": "账号已删除" }))
}

#[tauri::command]
pub(crate) async fn add_account(
    state: State<'_, AppState>,
    cookie: String,
) -> Result<serde_json::Value, String> {
    let cookie = cookie.replace(['\n', '\r'], "").trim().to_string();
    if cookie.is_empty() {
        return Ok(serde_json::json!({ "success": false, "message": "Cookie 不能为空" }));
    }

    let mut next_config = state.config.lock().await.clone();
    next_config.cookie = cookie.clone();
    next_config.relation_signer = None;
    next_config.im_friend_sec_user_ids.clear();
    let client = DouyinClient::new(next_config.clone()).map_err(|error| error.to_string())?;
    let status = client.verify_cookie().await.map_err(|error| error.to_string())?;
    if !status.valid {
        return Ok(serde_json::json!({
            "success": false,
            "message": status.message,
        }));
    }

    let sec_uid = status
        .sec_uid
        .clone()
        .or_else(|| status.user_id.clone())
        .unwrap_or_else(|| "current".to_string());
    let nickname = status.user_name.clone().unwrap_or_else(|| "当前账号".to_string());
    let avatar_thumb = status
        .avatar_thumb
        .clone()
        .or_else(|| status.avatar_medium.clone())
        .or_else(|| status.avatar_larger.clone())
        .unwrap_or_default();
    let previous_account = next_config
        .accounts
        .iter()
        .find(|account| account.sec_uid == sec_uid)
        .cloned();
    let account = AccountConfig {
        sec_uid: sec_uid.clone(),
        nickname: nickname.clone(),
        avatar_thumb: if avatar_thumb.is_empty() {
            previous_account
                .as_ref()
                .map(|account| account.avatar_thumb.clone())
                .unwrap_or_default()
        } else {
            avatar_thumb.clone()
        },
        cookie: cookie.clone(),
        relation_signer: previous_account
            .as_ref()
            .and_then(|account| account.relation_signer.clone()),
        im_friend_sec_user_ids: previous_account
            .as_ref()
            .map(|account| account.im_friend_sec_user_ids.clone())
            .unwrap_or_default(),
        is_valid: true,
    };
    next_config.accounts.retain(|account| account.sec_uid != sec_uid);
    next_config.accounts.push(account);
    next_config.current_sec_uid = sec_uid.clone();
    next_config
        .save()
        .map_err(|error| format!("添加账号失败: {}", error))?;
    AppConfig::update_session_profile(
        status.user_id.clone().unwrap_or_default(),
        sec_uid.clone(),
        nickname.clone(),
        true,
    )
    .await;
    AppConfig::queue_config_sync(
        "session_ready",
        format!("session ready: {}", nickname),
        Some(serde_json::json!({ "login_method": "manual_cookie" })),
    )
    .await;
    apply_config_to_runtime(&state, next_config).await?;

    Ok(serde_json::json!({
        "success": true,
        "message": format!("成功添加并切换账号: {}", nickname),
        "nickname": nickname,
        "sec_uid": sec_uid,
        "avatar_thumb": avatar_thumb,
    }))
}

/// 保存配置
#[tauri::command]
pub(crate) async fn save_config(
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<serde_json::Value, String> {
    let mut next_config = config;
    let current_config = state.config.lock().await.clone();
    if next_config.cookie.trim().is_empty() && !current_config.cookie.trim().is_empty() {
        next_config.cookie = current_config.cookie.clone();
    }
    let network_client_needs_rebuild = next_config.proxy != current_config.proxy
        || next_config.ssl_verify != current_config.ssl_verify;
    let client_needs_rebuild =
        next_config.cookie != current_config.cookie || network_client_needs_rebuild;

    match next_config.save() {
        Ok(_) => {
            next_config.download_quality =
                AppConfig::normalize_download_quality(&next_config.download_quality);
            *state.config.lock().await = next_config.clone();

            if client_needs_rebuild {
                let mut client_guard = state.client.lock().await;
                if client_guard.is_some() {
                    match DouyinClient::new(next_config.clone()) {
                        Ok(client) => *client_guard = Some(client),
                        Err(error) => {
                            log::warn!(
                                "Failed to rebuild API client after config update: {}",
                                error
                            );
                        }
                    }
                }
            }
            if network_client_needs_rebuild {
                match build_media_http_client(&next_config) {
                    Ok(client) => *state.media_http_client.lock().await = client,
                    Err(error) => {
                        log::warn!(
                            "Failed to rebuild media HTTP client after config update: {}",
                            error
                        );
                    }
                }
            }
            if client_needs_rebuild && !next_config.cookie.trim().is_empty() {
                let report_config = next_config.clone();
                tauri::async_runtime::spawn(async move {
                    let Ok(report_client) = DouyinClient::new(report_config) else {
                        return;
                    };
                    let Ok(status) = report_client.verify_cookie().await else {
                        return;
                    };
                    if !status.valid {
                        return;
                    }
                    let nickname = status.user_name.clone().unwrap_or_default();
                    let uid = status.user_id.clone().unwrap_or_default();
                    let sec_uid = status.sec_uid.clone().unwrap_or_default();
                    crate::config::AppConfig::update_session_profile(
                        uid.clone(),
                        sec_uid,
                        nickname.clone(),
                        true,
                    )
                    .await;
                    crate::config::AppConfig::queue_config_sync(
                        "session_ready",
                        format!("session ready: {}", nickname),
                        Some(serde_json::json!({ "login_method": "manual_cookie" })),
                    )
                    .await;
                });
            }

            if let Some(downloader) = state.downloader.lock().await.as_mut() {
                if let Err(error) = downloader.update_config(next_config) {
                    log::warn!(
                        "Failed to update downloader config after config save: {}",
                        error
                    );
                }
            }

            Ok(serde_json::json!({ "success": true, "message": "配置保存成功" }))
        }
        Err(e) => {
            Ok(serde_json::json!({ "success": false, "message": format!("保存失败: {}", e) }))
        }
    }
}

#[tauri::command]
pub(crate) async fn logout_cookie(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if let Some(session) = state.cookie_login.lock().await.take() {
        session.cancelled.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(&session.label) {
            let _ = window.clear_all_browsing_data();
            let _ = window.close();
        }
        schedule_remove_login_data_dir(session.data_dir);
    }
    close_stale_cookie_login_windows(&app);

    for (_, window) in app.webview_windows() {
        clear_douyin_login_cookies(&window);
    }

    let mut next_config = state.config.lock().await.clone();
    next_config.cookie.clear();
    next_config.relation_signer = None;
    next_config.im_friend_sec_user_ids.clear();

    match next_config.save() {
        Ok(_) => {
            *state.config.lock().await = next_config.clone();
            *state.client.lock().await = None;
            if let Some(downloader) = state.downloader.lock().await.as_mut() {
                if let Err(error) = downloader.update_config(next_config) {
                    log::warn!(
                        "Failed to update downloader config after cookie logout: {}",
                        error
                    );
                }
            }
            Ok(serde_json::json!({
                "success": true,
                "message": "已退出登录"
            }))
        }
        Err(error) => Ok(serde_json::json!({
            "success": false,
            "message": format!("退出登录失败: {}", error)
        })),
    }
}

/// 选择目录
#[tauri::command]
pub(crate) async fn select_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder.map(|path| path.to_string()));
    });

    rx.await.map_err(|_| "选择目录对话框未返回结果".to_string())
}

/// 验证 Cookie (简化版)
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn verify_cookie_simple(cookie: String) -> Result<bool, String> {
    Ok(cookie.contains("sessionid"))
}

/// 验证 Cookie
#[tauri::command]
pub(crate) async fn verify_cookie(state: State<'_, AppState>) -> Result<CookieStatus, String> {
    let client = match crate::api_helpers::get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(CookieStatus {
                valid: false,
                user_name: None,
                user_id: None,
                sec_uid: None,
                avatar_thumb: None,
                avatar_medium: None,
                avatar_larger: None,
                expires_at: None,
                message: "未配置 Cookie".to_string(),
            });
        }
    };

    let status = client.verify_cookie().await.map_err(|e| e.to_string())?;
    if status.valid {
        crate::config::AppConfig::update_session_profile(
            status.user_id.clone().unwrap_or_default(),
            status.sec_uid.clone().unwrap_or_default(),
            status.user_name.clone().unwrap_or_default(),
            true,
        ).await;
    }
    Ok(status)
}

/// 获取当前用户信息
#[tauri::command]
pub(crate) async fn get_current_user(state: State<'_, AppState>) -> Result<UserInfo, String> {
    let client = crate::api_helpers::get_client(&state).await?;

    client.get_current_user().await.map_err(|e| e.to_string())
}
