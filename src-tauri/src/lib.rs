//! 抖音视频下载器 - Tauri 应用

pub mod api;
pub mod config;
pub mod cookie;
pub mod downloader;
pub mod history;
pub mod media_proxy;
pub mod media_utils;
pub mod sign;

use api::{CookieStatus, DouyinClient, DownloadHistory, UserInfo, VideoInfo};
use config::AppConfig;
use cookie::{
    has_douyin_login_cookie, parse_cookie_string, serialize_cookie_string,
    verify_douyin_login_cookie, CookieLoginSession,
};
use downloader::{Downloader, DownloaderEvent};
use history::HistoryManager;
use media_utils::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};
use url::Url;

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub(crate) config: Arc<Mutex<AppConfig>>,
    pub(crate) client: Arc<Mutex<Option<DouyinClient>>>,
    pub(crate) downloader: Arc<Mutex<Option<Downloader>>>,
    pub(crate) history: Arc<Mutex<HistoryManager>>,
    pub(crate) app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    pub(crate) cookie_login: Arc<Mutex<Option<CookieLoginSession>>>,
    pub(crate) media_http_client: reqwest::Client,
    pub(crate) media_redirect_cache: Arc<Mutex<HashMap<String, String>>>,
}

impl AppState {
    pub fn new() -> Self {
        let config = AppConfig::load();
        let history = HistoryManager::load();
        let media_http_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build media HTTP client");
        Self {
            config: Arc::new(Mutex::new(config)),
            client: Arc::new(Mutex::new(None)),
            downloader: Arc::new(Mutex::new(None)),
            history: Arc::new(Mutex::new(history)),
            app_handle: Arc::new(Mutex::new(None)),
            cookie_login: Arc::new(Mutex::new(None)),
            media_http_client,
            media_redirect_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

async fn get_client(state: &State<'_, AppState>) -> Result<DouyinClient, String> {
    state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Client not initialized".to_string())
}

async fn emit_cookie_login_status(app: &tauri::AppHandle, payload: serde_json::Value) {
    let _ = app.emit("cookie-login-status", payload);
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// 初始化客户端
#[tauri::command]
async fn init_client(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
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

// ==================== 配置 API ====================

/// 获取配置
#[tauri::command]
fn get_config(state: State<'_, AppState>) -> serde_json::Value {
    let config = state.config.blocking_lock().clone();
    let cookie_set = !config.cookie.trim().is_empty();
    let mut value = serde_json::to_value(&config).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("cookie".to_string(), serde_json::json!(""));
        object.insert("cookie_set".to_string(), serde_json::json!(cookie_set));
    }
    value
}

/// 保存配置
#[tauri::command]
fn save_config(state: State<'_, AppState>, config: AppConfig) -> serde_json::Value {
    let mut next_config = config;
    {
        let current_config = state.config.blocking_lock();
        if next_config.cookie.trim().is_empty() && !current_config.cookie.trim().is_empty() {
            next_config.cookie = current_config.cookie.clone();
        }
    }

    match next_config.save() {
        Ok(_) => {
            *state.config.blocking_lock() = next_config.clone();
            serde_json::json!({ "success": true, "message": "配置保存成功" })
        }
        Err(e) => {
            serde_json::json!({ "success": false, "message": format!("保存失败: {}", e) })
        }
    }
}

/// 选择目录
#[tauri::command]
fn select_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let folder = app.dialog().file().blocking_pick_folder();

    match folder {
        Some(path) => Ok(Some(path.to_string())),
        None => Ok(None),
    }
}

/// 验证 Cookie (简化版)
#[tauri::command]
#[allow(dead_code)]
async fn verify_cookie_simple(cookie: String) -> Result<bool, String> {
    Ok(cookie.contains("sessionid"))
}

#[tauri::command]
async fn open_verify_browser(
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
        return Ok(serde_json::json!({
            "success": true,
            "message": "验证窗口已打开，请完成验证"
        }));
    }

    let cookie_literal =
        serde_json::to_string(&cookie).map_err(|error| format!("Cookie 序列化失败: {}", error))?;
    let init_script = format!(
        r#"
        (() => {{
            const rawCookie = {cookie_literal};
            if (!rawCookie) return;
            if (window.sessionStorage && sessionStorage.getItem('__dy_verify_cookie_applied') === '1') return;
            rawCookie.split(';').map(item => item.trim()).filter(Boolean).forEach(item => {{
                try {{
                    document.cookie = `${{item}}; domain=.douyin.com; path=/`;
                }} catch (error) {{}}
            }});
            try {{
                if (window.sessionStorage) {{
                    sessionStorage.setItem('__dy_verify_cookie_applied', '1');
                    setTimeout(() => window.location.reload(), 120);
                }}
            }} catch (error) {{}}
        }})();
        "#
    );

    tauri::WebviewWindowBuilder::new(
        &app,
        "verify-browser",
        tauri::WebviewUrl::External(target_url),
    )
    .title("抖音验证")
    .inner_size(1100.0, 750.0)
    .resizable(true)
    .focused(true)
    .initialization_script(&init_script)
    .build()
    .map_err(|error| format!("无法打开验证窗口: {}", error))?;

    Ok(serde_json::json!({
        "success": true,
        "message": "验证窗口已打开，请完成验证"
    }))
}

#[tauri::command]
async fn cookie_browser_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    timeout: Option<u64>,
    browser: Option<String>,
) -> Result<serde_json::Value, String> {
    let _ = browser;
    let label = "cookie-browser-login".to_string();
    let login_url = Url::parse("https://www.douyin.com/").map_err(|error| error.to_string())?;

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(serde_json::json!({
            "success": true,
            "message": "登录窗口已打开"
        }));
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    *state.cookie_login.lock().await = Some(CookieLoginSession {
        label: label.clone(),
        cancelled: cancelled.clone(),
    });

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::External(login_url.clone()),
    )
    .title("登录抖音账号")
    .inner_size(1100.0, 820.0)
    .resizable(true)
    .focused(true)
    .build()
    .map_err(|error| format!("无法打开登录窗口: {}", error))?;

    let existing_cookie = state.config.lock().await.cookie.clone();
    for cookie in parse_cookie_string(&existing_cookie) {
        let _ = window.set_cookie(cookie);
    }
    let _ = window.navigate(login_url.clone());

    emit_cookie_login_status(
        &app,
        serde_json::json!({
            "event": "pending",
            "message": "请在弹出的窗口中完成登录"
        }),
    )
    .await;

    let config_state = state.config.clone();
    let client_state = state.client.clone();
    let downloader_state = state.downloader.clone();
    let cookie_login_state = state.cookie_login.clone();
    let login_timeout = timeout.unwrap_or(300);
    let label_clone = label.clone();

    tauri::async_runtime::spawn(async move {
        let started_at = std::time::Instant::now();
        let mut last_verify_attempt: Option<(String, std::time::Instant)> = None;

        loop {
            if cancelled.load(Ordering::SeqCst) {
                if let Some(window) = app.get_webview_window(&label_clone) {
                    let _ = window.close();
                }
                *cookie_login_state.lock().await = None;
                emit_cookie_login_status(
                    &app,
                    serde_json::json!({
                        "event": "cancelled",
                        "message": "已取消登录"
                    }),
                )
                .await;
                break;
            }

            if started_at.elapsed().as_secs() >= login_timeout {
                if let Some(window) = app.get_webview_window(&label_clone) {
                    let _ = window.close();
                }
                *cookie_login_state.lock().await = None;
                emit_cookie_login_status(
                    &app,
                    serde_json::json!({
                        "event": "timeout",
                        "message": "登录超时，请重试"
                    }),
                )
                .await;
                break;
            }

            let Some(window) = app.get_webview_window(&label_clone) else {
                *cookie_login_state.lock().await = None;
                emit_cookie_login_status(
                    &app,
                    serde_json::json!({
                        "event": "cancelled",
                        "message": "登录窗口已关闭"
                    }),
                )
                .await;
                break;
            };

            match window.cookies() {
                Ok(cookies) => {
                    let cookies: Vec<_> = cookies
                        .into_iter()
                        .filter(|cookie| {
                            let name = cookie.name();
                            cookie
                                .domain()
                                .map(|domain| {
                                    domain.contains("douyin.com")
                                        || domain.contains("amemv.com")
                                        || domain.contains("bytedance.com")
                                        || domain.contains("snssdk.com")
                                })
                                .unwrap_or_else(|| {
                                    matches!(
                                        name,
                                        "sessionid"
                                            | "sessionid_ss"
                                            | "sid_guard"
                                            | "uid_tt"
                                            | "passport_csrf_token"
                                            | "passport_auth_status"
                                            | "ttwid"
                                            | "msToken"
                                            | "s_v_web_id"
                                    )
                                })
                        })
                        .collect();
                    let cookie_string = serialize_cookie_string(&cookies);
                    log::info!(
                        "cookie browser login poll: cookie_count={} names={}",
                        cookies.len(),
                        cookies
                            .iter()
                            .map(|cookie| cookie.name().to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    );

                    if has_douyin_login_cookie(&cookies) {
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
                        let mut next_config = config_state.lock().await.clone();
                        next_config.cookie = cookie_string.clone();
                        if let Err(error) = next_config.save() {
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
                            downloader.update_config(next_config);
                        }

                        let _ = window.close();
                        *cookie_login_state.lock().await = None;
                        emit_cookie_login_status(
                            &app,
                            serde_json::json!({
                                "event": "success",
                                "message": format!("Cookie 获取成功！已登录为 {}", current_user.nickname),
                                "cookie_set": true
                            }),
                        )
                        .await;
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
async fn cancel_cookie_browser_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let session = state.cookie_login.lock().await.clone();
    if let Some(session) = session {
        session.cancelled.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(&session.label) {
            let _ = window.close();
        }
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

// ==================== 视频/用户 API ====================

/// 解析视频链接
#[tauri::command]
async fn parse_url(state: State<'_, AppState>, url: String) -> Result<VideoInfo, String> {
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
async fn parse_link(state: State<'_, AppState>, link: String) -> Result<serde_json::Value, String> {
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
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    let video = match client.parse_share_link(&trimmed_link).await {
        Ok(video) => video,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("解析链接失败: {}", e)
            }));
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

/// 获取视频详情
#[tauri::command]
async fn get_video_detail(
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
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    match client.get_video_detail(&aweme_id).await {
        Ok(video) => Ok(serde_json::json!({
            "success": true,
            "video": python_video_detail_value(&video)
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("获取视频详情失败: {}", e)
        })),
    }
}

/// 搜索用户
#[tauri::command]
async fn search_user(
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
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    match client.search_user(&keyword).await {
        Ok(api::SearchUserResult::NeedVerify { verify_url }) => Ok(serde_json::json!({
            "success": false,
            "need_verify": true,
            "verify_url": verify_url,
            "message": "需要完成滑块验证"
        })),
        Ok(api::SearchUserResult::NotFound) => Ok(serde_json::json!({
            "success": false,
            "message": "未找到用户"
        })),
        Ok(api::SearchUserResult::Single(user)) => Ok(serde_json::json!({
            "success": true,
            "type": "single",
            "user": python_user_value(user.as_ref())
        })),
        Ok(api::SearchUserResult::Multiple(users)) => Ok(serde_json::json!({
            "success": true,
            "type": "multiple",
            "users": users.iter().map(python_user_value).collect::<Vec<_>>()
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("搜索失败: {}", e)
        })),
    }
}

/// 获取用户详情
#[tauri::command]
async fn get_user_detail(
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
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    match client.get_user_detail(&sec_uid).await {
        Ok(user_detail) => Ok(serde_json::json!({
            "success": true,
            "user": python_user_value(&user_detail.info)
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("获取用户详情失败: {}", e)
        })),
    }
}

/// 获取用户视频列表
#[tauri::command]
async fn get_user_videos(
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
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
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
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("获取用户视频列表失败: {}", e)
        })),
    }
}

/// 获取点赞视频列表
#[tauri::command]
async fn get_liked_videos(
    state: State<'_, AppState>,
    sec_uid: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    match client
        .get_liked_videos_python_style(&sec_uid, cursor, count)
        .await
    {
        Ok(videos) if !videos.is_empty() => {
            let count = videos.len();
            Ok(serde_json::json!({
                "success": true,
                "data": videos,
                "count": count
            }))
        }
        Ok(_) => Ok(serde_json::json!({
            "success": false,
            "message": "获取点赞视频失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。如果Cookie已过期请重新获取。"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("获取点赞视频失败: {}", e)
        })),
    }
}

/// 获取点赞作者列表
#[tauri::command]
async fn get_liked_authors(
    state: State<'_, AppState>,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    let liked_videos = match client.get_liked_videos_python_style("", 0, count).await {
        Ok(videos) => videos,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("获取点赞作者失败: {}", e)
            }));
        }
    };

    if liked_videos.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "获取点赞作者失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。"
        }));
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
        return Ok(serde_json::json!({
            "success": false,
            "message": "获取点赞作者失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。"
        }));
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
async fn get_recommended(
    state: State<'_, AppState>,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先登录抖音账号"
            }));
        }
    };

    log::info!("get_recommended invoked: cursor={} count={}", cursor, count);

    let (videos, next_cursor, has_more) = match client.get_recommended_feed(cursor, count).await {
        Ok(result) => result,
        Err(e) => {
            log::error!(
                "get_recommended failed: cursor={} count={} error={}",
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

    log::info!(
        "get_recommended completed: cursor={} count={} next_cursor={} has_more={} videos={}",
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
        "count": count
    }))
}

/// 获取评论列表
#[tauri::command]
async fn get_comments(
    state: State<'_, AppState>,
    aweme_id: String,
    cursor: i64,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = get_client(&state).await?;

    let (comments, next_cursor, has_more) = client
        .get_comments(&aweme_id, cursor, count)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "comments": comments,
        "cursor": next_cursor,
        "has_more": has_more
    }))
}

// ==================== Cookie API ====================

/// 验证 Cookie
#[tauri::command]
async fn verify_cookie(state: State<'_, AppState>) -> Result<CookieStatus, String> {
    let client = get_client(&state).await?;

    client.verify_cookie().await.map_err(|e| e.to_string())
}

/// 获取当前用户信息
#[tauri::command]
async fn get_current_user(state: State<'_, AppState>) -> Result<UserInfo, String> {
    let client = get_client(&state).await?;

    client.get_current_user().await.map_err(|e| e.to_string())
}

// ==================== 下载 API ====================

/// 下载单个视频
#[tauri::command]
async fn download_video(
    state: State<'_, AppState>,
    video: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let raw_media_type = download_media_type_from_payload(&video);
    let aweme_id = video
        .get("aweme_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut desc = video
        .get("desc")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut author_name = video
        .get("author_name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if author_name.is_empty() {
        author_name = video
            .get("author")
            .and_then(|value| value.get("nickname"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let mut cover = video
        .get("cover_url")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if cover.is_empty() {
        cover = video
            .get("video")
            .and_then(|value| value.get("cover"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    if cover.is_empty() {
        cover = video
            .get("video")
            .and_then(|value| value.get("origin_cover"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let mut media_urls = parse_download_media_items(&video, &raw_media_type);
    let mut media_type = media_type_from_payload_or_items(&raw_media_type, &media_urls);

    if (media_urls.is_empty() || desc.is_empty() || cover.is_empty()) && !aweme_id.is_empty() {
        if let Ok(client) = get_client(&state).await {
            if let Ok(fresh_video) = client.get_video_detail(&aweme_id).await {
                if desc.is_empty() {
                    desc = fresh_video.desc.clone();
                }
                if author_name.is_empty() {
                    author_name = fresh_video.author.nickname.clone();
                }
                if cover.is_empty() {
                    cover = fresh_video.video.cover.clone();
                }
                if media_urls.is_empty() {
                    media_urls = download_media_items_from_video(&fresh_video);
                    media_type = fresh_video.media_type.clone();
                }
            }
        }
    }

    if media_urls.is_empty() {
        log::warn!(
            "download_video has no media urls after normalization: aweme_id={} desc={} author={} raw_media_type={}",
            aweme_id,
            desc,
            author_name,
            raw_media_type
        );
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有可用的媒体URL"
        }));
    }

    log::info!(
        "download_video normalized payload: aweme_id={} media_count={} media_type={:?} author={} cover_present={}",
        aweme_id,
        media_urls.len(),
        media_type,
        author_name,
        !cover.is_empty()
    );

    if desc.is_empty() {
        desc = format!("作品_{}", aweme_id);
    }
    if author_name.is_empty() {
        author_name = "未知作者".to_string();
    }

    let downloader_guard = state.downloader.lock().await;
    let downloader = match downloader_guard.as_ref() {
        Some(downloader) => downloader,
        None => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "服务未完全初始化"
            }));
        }
    };

    let task_id = match downloader
        .add_media_task(
            aweme_id.clone(),
            desc.clone(),
            author_name.clone(),
            String::new(),
            cover,
            media_type,
            media_urls,
            None,
        )
        .await
    {
        Ok(task_id) => task_id,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("下载启动失败: {}", e)
            }));
        }
    };

    if let Err(e) = downloader.start_download(&task_id).await {
        return Ok(serde_json::json!({
            "success": false,
            "message": format!("下载启动失败: {}", e)
        }));
    }

    Ok(serde_json::json!({
        "success": true,
        "task_id": task_id,
        "message": "下载任务已启动"
    }))
}

/// 批量下载用户视频（边获取边下载）
#[tauri::command]
async fn download_user_videos(
    state: State<'_, AppState>,
    sec_uid: String,
    nickname: String,
    aweme_count: i64,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    let batch_task_id = uuid::Uuid::new_v4().to_string();
    let total_videos = aweme_count.max(0) as usize;

    let downloader = {
        let downloader_guard = state.downloader.lock().await;
        match downloader_guard.as_ref() {
            Some(d) => d.clone(),
            None => {
                return Ok(serde_json::json!({
                    "success": false,
                    "message": "服务未完全初始化"
                }));
            }
        }
    };

    let batch_id = batch_task_id.clone();
    let nickname_clone = nickname.clone();
    let sec_uid_clone = sec_uid.clone();
    let client_clone = client.clone();
    let downloader_clone = downloader.clone();

    downloader
        .emit_batch_started(&batch_id, &nickname, total_videos)
        .await;

    tokio::spawn(async move {
        if let Err(e) = downloader_clone
            .start_streaming_download(
                client_clone,
                sec_uid_clone,
                batch_id,
                nickname_clone,
                total_videos,
            )
            .await
        {
            log::error!("Streaming download error: {}", e);
        }
    });

    Ok(serde_json::json!({
        "success": true,
        "task_id": batch_task_id,
        "message": format!("开始下载 {} 个视频", total_videos),
        "nickname": nickname,
        "total_videos": total_videos
    }))
}

/// 下载点赞视频
#[tauri::command]
async fn download_liked_videos(
    state: State<'_, AppState>,
    count: u32,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    let (videos, _, _) = match client.get_liked_videos("", 0, count).await {
        Ok(result) => result,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("获取视频列表失败: {}", e)
            }));
        }
    };

    if videos.is_empty() {
        return Ok(serde_json::json!({
            "success": false,
            "message": "没有找到点赞视频"
        }));
    }

    let batch_task_id = uuid::Uuid::new_v4().to_string();
    let total_videos = videos.len();
    let batch_task_id_clone = batch_task_id.clone();

    {
        let downloader_guard = state.downloader.lock().await;
        let downloader = match downloader_guard.as_ref() {
            Some(d) => d,
            None => {
                return Ok(serde_json::json!({
                    "success": false,
                    "message": "服务未完全初始化"
                }));
            }
        };

        let downloader_clone = downloader.clone();
        let videos_clone = videos.clone();

        tokio::spawn(async move {
            if let Err(e) = downloader_clone
                .start_batch_download(videos_clone, batch_task_id_clone, "点赞视频".to_string())
                .await
            {
                log::error!("Batch download error: {}", e);
            }
        });
    }

    Ok(serde_json::json!({
        "success": true,
        "task_id": batch_task_id,
        "message": format!("开始下载 {} 个点赞视频", total_videos),
        "total_videos": total_videos
    }))
}

/// 下载点赞作者作品
#[tauri::command]
async fn download_liked_authors(
    state: State<'_, AppState>,
    count: Option<u32>,
) -> Result<serde_json::Value, String> {
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "请先设置Cookie"
            }));
        }
    };

    let downloader_guard = state.downloader.lock().await;
    let downloader = match downloader_guard.as_ref() {
        Some(downloader) => downloader,
        None => {
            return Ok(serde_json::json!({
                "success": false,
                "message": "服务未完全初始化"
            }));
        }
    };

    let liked_videos = match client
        .get_liked_videos_python_style("", 0, count.unwrap_or(20))
        .await
    {
        Ok(videos) => videos,
        Err(e) => {
            return Ok(serde_json::json!({
                "success": false,
                "message": format!("下载失败: {}", e)
            }));
        }
    };

    let mut seen = HashSet::new();
    let mut task_ids = Vec::new();

    for video in liked_videos {
        let sec_uid = video.author.sec_uid.trim().to_string();
        if sec_uid.is_empty() || !seen.insert(sec_uid.clone()) {
            continue;
        }

        if let Ok((videos, _, _)) = client.get_user_videos(&sec_uid, 0, 100).await {
            for user_video in &videos {
                if let Ok(task_id) = downloader.add_task(user_video, None).await {
                    let _ = downloader.start_download(&task_id).await;
                    task_ids.push(task_id);
                }
            }
        }
    }
    let count = task_ids.len();

    Ok(serde_json::json!({
        "success": true,
        "task_id": uuid::Uuid::new_v4().to_string(),
        "task_ids": task_ids,
        "message": "点赞作者作品下载任务已开始",
        "count": count
    }))
}

/// 添加下载任务
#[tauri::command]
async fn add_download_task(
    state: State<'_, AppState>,
    video: VideoInfo,
    save_path: Option<String>,
) -> Result<String, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    let path = save_path.map(std::path::PathBuf::from);
    downloader
        .add_task(&video, path)
        .await
        .map_err(|e| e.to_string())
}

/// 开始下载
#[tauri::command]
async fn start_download(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    downloader
        .start_download(&task_id)
        .await
        .map_err(|e| e.to_string())
}

/// 获取下载任务列表
#[tauri::command]
async fn get_download_tasks(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    Ok(serde_json::json!({
        "success": true,
        "tasks": downloader.get_tasks().await
    }))
}

/// 取消下载任务
#[tauri::command]
async fn cancel_download_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    match downloader.cancel_task(&task_id).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "message": "任务已取消"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("取消任务失败: {}", e)
        })),
    }
}

/// 删除下载任务
#[tauri::command]
async fn remove_download_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    downloader
        .remove_task(&task_id)
        .await
        .map_err(|e| e.to_string())
}

/// 暂停下载
#[tauri::command]
async fn pause_download(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    match downloader.pause_task(&task_id).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "message": "任务已暂停"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("暂停任务失败: {}", e)
        })),
    }
}

/// 恢复下载
#[tauri::command]
async fn resume_download(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let downloader_guard = state.downloader.lock().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or("Downloader not initialized")?;

    match downloader.resume_task(&task_id).await {
        Ok(_) => Ok(serde_json::json!({
            "success": true,
            "message": "任务已恢复"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("恢复任务失败: {}", e)
        })),
    }
}

// ==================== 下载历史 API ====================

/// 获取下载历史
#[tauri::command]
async fn get_history(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let history = HistoryManager::load();
    *state.history.lock().await = history;
    let history = state.history.lock().await;
    let items = history.get_all();
    Ok(serde_json::json!({
        "success": true,
        "items": items
    }))
}

#[derive(Debug, Clone, Serialize)]
struct DownloadFileEntry {
    id: String,
    filename: String,
    path: String,
    author: String,
    desc: String,
    size: u64,
    timestamp: i64,
    file_type: String,
}

fn scan_download_directory_entries(
    dir: &Path,
    items: &mut Vec<DownloadFileEntry>,
) -> Result<(), String> {
    let read_dir = fs::read_dir(dir).map_err(|e| e.to_string())?;

    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|e| e.to_string())?;

        if metadata.is_dir() {
            scan_download_directory_entries(&path, items)?;
            continue;
        }

        if !metadata.is_file() {
            continue;
        }

        let filename = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("未命名文件")
            .to_string();
        let author = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let timestamp = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let file_type = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();

        items.push(DownloadFileEntry {
            id: path.to_string_lossy().to_string(),
            filename,
            path: path.to_string_lossy().to_string(),
            author,
            desc: String::new(),
            size: metadata.len(),
            timestamp,
            file_type,
        });
    }

    Ok(())
}

#[tauri::command]
async fn list_download_files(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let target = configured_download_directory(&state).await?;
    let mut items = Vec::new();
    scan_download_directory_entries(&target, &mut items)?;
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(serde_json::json!({
        "success": true,
        "items": items
    }))
}

/// 清空下载历史
#[tauri::command]
async fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    let mut history = state.history.lock().await;
    history.clear().map_err(|e| e.to_string())
}

/// 删除历史记录
#[tauri::command]
async fn delete_history(state: State<'_, AppState>, aweme_id: String) -> Result<(), String> {
    let mut history = state.history.lock().await;
    history.delete(&aweme_id).map_err(|e| e.to_string())
}

/// 添加历史记录
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn add_history(
    state: State<'_, AppState>,
    aweme_id: String,
    title: String,
    author: String,
    author_id: String,
    cover: String,
    file_path: String,
    media_type: String,
    file_size: u64,
) -> Result<(), String> {
    let mut history = state.history.lock().await;
    history
        .add(DownloadHistory {
            aweme_id,
            title,
            author,
            author_id,
            cover,
            file_path,
            media_type,
            file_size,
            create_time: chrono::Utc::now().timestamp(),
        })
        .map_err(|e| e.to_string())
}

// ==================== 文件操作 API ====================

fn canonical_existing_file(raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("文件路径不能为空".to_string());
    }

    let path = Path::new(trimmed);
    let canonical = path
        .canonicalize()
        .map_err(|_| "文件不存在或无法访问".to_string())?;

    if !canonical.is_file() {
        return Err("只能操作文件".to_string());
    }

    Ok(canonical)
}

fn canonical_existing_directory(raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("目录路径不能为空".to_string());
    }

    let path = Path::new(trimmed);
    let canonical = path
        .canonicalize()
        .map_err(|_| "目录不存在或无法访问".to_string())?;

    if !canonical.is_dir() {
        return Err("只能打开目录".to_string());
    }

    Ok(canonical)
}

async fn allowed_existing_file_path(
    state: &State<'_, AppState>,
    raw_path: &str,
) -> Result<PathBuf, String> {
    let target = canonical_existing_file(raw_path)?;

    let download_path = {
        let config = state.config.lock().await;
        config.download_path.clone()
    };

    if !download_path.trim().is_empty() {
        if let Ok(download_root) = Path::new(&download_path).canonicalize() {
            if target.starts_with(download_root) {
                return Ok(target);
            }
        }
    }

    let history_items = {
        let history = state.history.lock().await;
        history.get_all()
    };

    let is_history_file = history_items.iter().any(|item| {
        Path::new(&item.file_path)
            .canonicalize()
            .map(|history_path| history_path == target)
            .unwrap_or(false)
    });

    if is_history_file {
        Ok(target)
    } else {
        Err("仅允许操作下载目录或下载历史中的文件".to_string())
    }
}

async fn configured_download_directory(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    let download_path = {
        let config = state.config.lock().await;
        config.download_path.clone()
    };

    let trimmed = download_path.trim();
    if trimmed.is_empty() {
        return Err("下载目录未设置".to_string());
    }

    std::fs::create_dir_all(trimmed).map_err(|e| format!("创建下载目录失败: {e}"))?;
    canonical_existing_directory(trimmed)
}

async fn allowed_existing_download_directory_path(
    state: &State<'_, AppState>,
    raw_path: &str,
) -> Result<PathBuf, String> {
    let target = canonical_existing_directory(raw_path)?;
    let download_root = configured_download_directory(state).await?;

    if target.starts_with(download_root) {
        Ok(target)
    } else {
        Err("仅允许打开下载目录下的文件夹".to_string())
    }
}

fn open_file_with_system(target: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn open_directory_with_system(target: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    Command::new("explorer")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn reveal_file_with_system(target: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg("-R")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    {
        let mut select_arg = std::ffi::OsString::from("/select,");
        select_arg.push(target);
        Command::new("explorer")
            .arg(select_arg)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target.parent().unwrap_or(Path::new(".")))
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// 打开文件
#[tauri::command]
async fn open_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let target = allowed_existing_file_path(&state, &path).await?;
    open_file_with_system(&target)
}

/// 打开下载目录
#[tauri::command]
async fn open_download_directory(state: State<'_, AppState>) -> Result<(), String> {
    let target = configured_download_directory(&state).await?;
    open_directory_with_system(&target)
}

/// 打开文件所在目录
#[tauri::command]
async fn open_file_location(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let canonical = Path::new(path.trim())
        .canonicalize()
        .map_err(|_| "文件或目录不存在或无法访问".to_string())?;

    if canonical.is_dir() {
        let target = allowed_existing_download_directory_path(&state, &path).await?;
        return open_directory_with_system(&target);
    }

    let target = allowed_existing_file_path(&state, &path).await?;
    reveal_file_with_system(&target)
}

/// 删除文件
#[tauri::command]
async fn delete_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let target = allowed_existing_file_path(&state, &path).await?;
    std::fs::remove_file(target).map_err(|e| e.to_string())
}

/// 获取应用版本号
#[tauri::command]
fn get_app_version(app_handle: tauri::AppHandle) -> String {
    app_handle.package_info().version.to_string()
}

/// 重启应用
#[tauri::command]
fn restart_app(app_handle: tauri::AppHandle) {
    app_handle.request_restart();
}

#[cfg(windows)]
fn is_windows_portable_runtime() -> bool {
    tauri::utils::platform::bundle_type().is_none()
}

#[cfg(not(windows))]
fn is_windows_portable_runtime() -> bool {
    false
}

fn updater_install_mode() -> &'static str {
    if is_windows_portable_runtime() {
        "portable"
    } else {
        "bundled"
    }
}

#[cfg(windows)]
fn powershell_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[cfg(windows)]
async fn download_portable_update(
    app_handle: tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> Result<serde_json::Value, String> {
    use std::process::Command;

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "无法确定当前便携版程序目录".to_string())?;
    let exe_stem = current_exe
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("douyin-downloader");
    let update_path = exe_dir.join(format!("{exe_stem}.update.exe"));
    let script_path = exe_dir.join(format!("{exe_stem}.update.ps1"));

    let progress_app = app_handle.clone();
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            move |chunk_len, content_len| {
                downloaded += chunk_len as u64;
                let progress = content_len
                    .filter(|total| *total > 0)
                    .map(|total| downloaded as f64 / total as f64 * 100.0);
                let _ = progress_app.emit(
                    "update-download-progress",
                    serde_json::json!({
                        "downloaded": downloaded,
                        "total": content_len,
                        "progress": progress
                    }),
                );
            },
            || {},
        )
        .await
        .map_err(|e| format!("下载便携版更新失败: {e}"))?;

    std::fs::write(&update_path, bytes)
        .map_err(|e| format!("写入新版便携程序失败: {} ({})", update_path.display(), e))?;

    let target = powershell_quote_path(&current_exe);
    let update_file = powershell_quote_path(&update_path);
    let script_file = powershell_quote_path(&script_path);
    let pid = std::process::id();
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$pidToWait = {pid}
$target = {target}
$update = {update_file}
$backup = "$target.bak"
$log = "$target.update.log"
try {{
  Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 800
  if (Test-Path -LiteralPath $backup) {{
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
  }}
  if (Test-Path -LiteralPath $target) {{
    Move-Item -LiteralPath $target -Destination $backup -Force
  }}
  Move-Item -LiteralPath $update -Destination $target -Force
  Start-Process -FilePath $target
  Start-Sleep -Seconds 2
  Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
}} catch {{
  if ((Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $target)) {{
    Move-Item -LiteralPath $backup -Destination $target -Force -ErrorAction SilentlyContinue
  }}
  Add-Content -LiteralPath $log -Value $_.Exception.ToString()
}} finally {{
  Remove-Item -LiteralPath {script_file} -Force -ErrorAction SilentlyContinue
}}
"#
    );
    std::fs::write(&script_path, script).map_err(|e| format!("写入便携版替换脚本失败: {}", e))?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .spawn()
        .map_err(|e| format!("启动便携版替换脚本失败: {}", e))?;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app_handle.exit(0);
    });

    Ok(serde_json::json!({
        "success": true,
        "portable": true,
        "message": "便携版更新已下载，应用即将关闭并自动替换重启"
    }))
}

/// 检查更新
#[tauri::command]
async fn check_update(app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;

    let portable = is_windows_portable_runtime();
    let mut updater_builder = app_handle.updater_builder();
    if portable {
        updater_builder = updater_builder.target("windows-x86_64-portable");
    }

    let updater = updater_builder.build().map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(Some(update)) => Ok(serde_json::json!({
            "success": true,
            "has_update": true,
            "version": update.version.clone(),
            "current_version": update.current_version.clone(),
            "notes": update.body.unwrap_or_else(|| "无更新说明".to_string()),
            "date": update.date.map(|d| d.to_string()),
            "download_url": update.download_url.to_string(),
            "portable": portable,
            "install_mode": updater_install_mode()
        })),
        Ok(None) => Ok(serde_json::json!({
            "success": true,
            "has_update": false,
            "portable": portable,
            "install_mode": updater_install_mode()
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "portable": portable,
            "install_mode": updater_install_mode(),
            "message": format!("检查更新失败: {}", e)
        })),
    }
}

/// 下载并安装更新
#[tauri::command]
async fn download_update(app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;

    let portable = is_windows_portable_runtime();
    let mut updater_builder = app_handle.updater_builder();
    if portable {
        updater_builder = updater_builder.target("windows-x86_64-portable");
    }

    let updater = updater_builder.build().map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(Some(update)) => {
            #[cfg(windows)]
            if portable {
                return download_portable_update(app_handle, update).await;
            }

            let progress_app = app_handle.clone();
            let finished_app = app_handle.clone();
            let mut downloaded = 0u64;

            match update
                .download_and_install(
                    move |chunk_len, content_len| {
                        downloaded += chunk_len as u64;
                        let progress = content_len
                            .filter(|total| *total > 0)
                            .map(|total| downloaded as f64 / total as f64 * 100.0);
                        let _ = progress_app.emit(
                            "update-download-progress",
                            serde_json::json!({
                                "downloaded": downloaded,
                                "total": content_len,
                                "progress": progress
                            }),
                        );
                    },
                    move || {
                        let _ = finished_app.emit(
                            "update-download-finished",
                            serde_json::json!({ "success": true }),
                        );
                    },
                )
                .await
            {
                Ok(_) => Ok(serde_json::json!({
                    "success": true,
                    "message": "更新下载并安装完成"
                })),
                Err(e) => {
                    log::error!("failed to download and install update: {}", e);
                    let _ = app_handle.emit(
                        "update-download-error",
                        serde_json::json!({
                            "success": false,
                            "message": e.to_string()
                        }),
                    );
                    Ok(serde_json::json!({
                        "success": false,
                        "message": format!("下载更新失败: {}", e)
                    }))
                }
            }
        }
        Ok(None) => Ok(serde_json::json!({
            "success": false,
            "message": "没有可用更新"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("下载更新失败: {}", e)
        })),
    }
}

// ============================================================================
// 应用入口
// ============================================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let log_level = if cfg!(debug_assertions) {
                log::LevelFilter::Info
            } else {
                log::LevelFilter::Warn
            };
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log_level)
                    .build(),
            )?;

            let state = AppState::new();
            *state.app_handle.blocking_lock() = Some(app.handle().clone());
            tauri::async_runtime::spawn({
                let state = state.clone();
                async move {
                    if let Err(error) = media_proxy::spawn_media_proxy(state).await {
                        log::error!("failed to start media proxy: {}", error);
                    }
                }
            });
            app.manage(state);

            #[cfg(debug_assertions)]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_version,
            restart_app,
            check_update,
            download_update,
            init_client,
            get_config,
            save_config,
            select_directory,
            parse_url,
            parse_link,
            get_video_detail,
            search_user,
            get_user_detail,
            get_user_videos,
            get_liked_videos,
            get_liked_authors,
            get_recommended,
            get_comments,
            verify_cookie,
            get_current_user,
            open_verify_browser,
            cookie_browser_login,
            cancel_cookie_browser_login,
            download_video,
            download_user_videos,
            download_liked_videos,
            download_liked_authors,
            add_download_task,
            start_download,
            get_download_tasks,
            cancel_download_task,
            remove_download_task,
            pause_download,
            resume_download,
            list_download_files,
            get_history,
            clear_history,
            delete_history,
            add_history,
            open_file,
            open_download_directory,
            open_file_location,
            delete_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::media_utils::{download_media_type_from_payload, parse_download_media_items};

    #[test]
    fn parses_flat_download_media_items() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "raw_media_type": "video",
            "media_type": "video",
            "media_urls": [{ "type": "video", "url": "https://example.com/test.mp4" }],
        });

        let parsed = parse_download_media_items(&payload, "video");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "video");
        assert_eq!(parsed[0].url, "https://example.com/test.mp4");
    }

    #[test]
    fn parses_nested_react_video_payload() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "media_type": "video",
            "author": { "nickname": "tester" },
            "video": {
                "cover": "https://example.com/cover.jpg",
                "play_addr": "https://example.com/play.mp4"
            }
        });

        let parsed = parse_download_media_items(&payload, "video");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "video");
        assert_eq!(parsed[0].url, "https://example.com/play.mp4");
    }

    #[test]
    fn parses_image_and_live_photo_payloads() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "media_type": "mixed",
            "images": ["https://example.com/1.jpg"],
            "live_photos": ["https://example.com/1.mp4"]
        });

        let parsed = parse_download_media_items(&payload, "mixed");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].r#type, "live_photo");
        assert_eq!(parsed[1].r#type, "image");
    }

    #[test]
    fn resolves_download_media_type_from_string_and_numeric_payloads() {
        assert_eq!(
            download_media_type_from_payload(
                &serde_json::json!({ "raw_media_type": "live_photo" })
            ),
            "live_photo"
        );
        assert_eq!(
            download_media_type_from_payload(&serde_json::json!({ "raw_media_type": 1 })),
            "image"
        );
        assert_eq!(
            download_media_type_from_payload(&serde_json::json!({ "media_type": "mixed" })),
            "mixed"
        );
    }
}
