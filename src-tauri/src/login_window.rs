//! 登录窗口/WebView 清理相关 helper。

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tauri::Manager;

use crate::config::RelationSignerConfig;

const DOUYIN_LOGIN_COOKIE_NAMES: &[&str] = &[
    "sessionid",
    "sessionid_ss",
    "sid_guard",
    "uid_tt",
    "uid_tt_ss",
    "sid_tt",
    "sid_ucp_v1",
    "ssid_ucp_v1",
    "session_tlb_tag",
    "passport_auth_status",
    "passport_auth_status_ss",
    "passport_mfa_token",
    "d_ticket",
    "n_mh",
    "odin_tt",
    "_bd_ticket_crypt_cookie",
];

const DOUYIN_COOKIE_CLEAR_DOMAINS: &[&str] = &[
    ".douyin.com",
    "douyin.com",
    "www.douyin.com",
    "sso.douyin.com",
    "login.douyin.com",
];

pub(crate) const RELATION_SIGNER_COOKIE_NAME: &str = "dy_relation_signer";
pub(crate) const IM_FRIEND_IDS_COOKIE_PREFIX: &str = "dy_im_sec_user_ids";

pub(crate) fn clear_douyin_login_cookies(window: &tauri::WebviewWindow) {
    let mut names = DOUYIN_LOGIN_COOKIE_NAMES
        .iter()
        .map(|name| name.to_string())
        .collect::<HashSet<_>>();
    let mut domains = DOUYIN_COOKIE_CLEAR_DOMAINS
        .iter()
        .map(|domain| domain.to_string())
        .collect::<HashSet<_>>();

    let mut deleted = 0usize;
    if let Ok(cookies) = window.cookies() {
        for cookie in cookies {
            let name = cookie.name().to_string();
            let is_douyin_domain = cookie
                .domain()
                .map(|domain| {
                    let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
                    domain == "douyin.com" || domain.ends_with(".douyin.com")
                })
                .unwrap_or(false);
            let is_login_cookie = DOUYIN_LOGIN_COOKIE_NAMES.contains(&name.as_str())
                || name == RELATION_SIGNER_COOKIE_NAME
                || name.starts_with(IM_FRIEND_IDS_COOKIE_PREFIX);

            if is_douyin_domain || is_login_cookie {
                names.insert(cookie.name().to_string());
                if let Some(domain) = cookie.domain() {
                    domains.insert(domain.to_string());
                }
                if window.delete_cookie(cookie).is_ok() {
                    deleted += 1;
                }
            }
        }
    }

    let mut cleared = 0usize;
    for domain in domains {
        for name in &names {
            for suffix in [
                "Path=/; Max-Age=0",
                "Path=/; Max-Age=0; Secure; SameSite=None",
                "Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
                "Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Secure; SameSite=None",
            ] {
                if let Ok(cookie) =
                    tauri::webview::Cookie::parse(format!("{name}=; Domain={domain}; {suffix}"))
                {
                    let _ = window.set_cookie(cookie.into_owned());
                    cleared += 1;
                }
            }
        }
    }
    log::info!(
        "cleared douyin webview cookies: names={} deleted={} writes={}",
        names.len(),
        deleted,
        cleared
    );
}

pub(crate) fn clear_douyin_login_storage(window: &tauri::WebviewWindow) {
    let script = r#"
        (() => {
            try { localStorage.clear(); } catch (error) {}
            try { sessionStorage.clear(); } catch (error) {}
            try {
                if (window.caches && caches.keys) {
                    caches.keys().then((keys) => keys.forEach((key) => caches.delete(key))).catch(() => {});
                }
            } catch (error) {}
            try {
                if (window.indexedDB && indexedDB.databases) {
                    indexedDB.databases()
                        .then((databases) => databases.forEach((db) => db && db.name && indexedDB.deleteDatabase(db.name)))
                        .catch(() => {});
                }
            } catch (error) {}
        })();
    "#;
    if let Err(error) = window.eval(script) {
        log::debug!("failed to clear douyin login storage: {}", error);
    }
}

pub(crate) fn reset_douyin_login_window_state(window: &tauri::WebviewWindow) {
    clear_douyin_login_cookies(window);
    clear_douyin_login_storage(window);
}

pub(crate) fn schedule_douyin_login_storage_cleanup(window: tauri::WebviewWindow) {
    tauri::async_runtime::spawn(async move {
        for delay_ms in [300_u64, 1200, 2500] {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            clear_douyin_login_storage(&window);
        }
    });
}

pub(crate) fn schedule_remove_login_data_dir(path: Option<PathBuf>) {
    let Some(path) = path else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Err(error) = fs::remove_dir_all(&path) {
            if path.exists() {
                log::debug!(
                    "failed to remove douyin login webview data dir {}: {}",
                    path.display(),
                    error
                );
            }
        }
    });
}

pub(crate) fn close_stale_cookie_login_windows(app: &tauri::AppHandle) {
    for (label, window) in app.webview_windows() {
        if label == "cookie-browser-login" || label.starts_with("cookie-browser-login-") {
            let _ = window.clear_all_browsing_data();
            let _ = window.close();
        }
    }
}

pub(crate) fn extract_relation_signer_cookie(
    cookies: &[tauri::webview::Cookie<'static>],
) -> Option<RelationSignerConfig> {
    let raw_value = cookies
        .iter()
        .rev()
        .find(|cookie| cookie.name() == RELATION_SIGNER_COOKIE_NAME)?
        .value()
        .to_string();
    let decoded = urlencoding::decode(&raw_value).ok()?.into_owned();
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        decoded.as_bytes(),
    )
    .ok()?;
    let signer = serde_json::from_slice::<RelationSignerConfig>(&bytes).ok()?;
    if signer.ticket.trim().is_empty()
        || signer.ts_sign.trim().is_empty()
        || signer.public_key.trim().is_empty()
        || signer.ecdh_key.trim().is_empty()
    {
        return None;
    }
    Some(signer)
}

pub(crate) fn strip_internal_login_cookies(
    cookies: &[tauri::webview::Cookie<'static>],
) -> Vec<tauri::webview::Cookie<'static>> {
    cookies
        .iter()
        .filter(|cookie| {
            cookie.name() != RELATION_SIGNER_COOKIE_NAME
                && !cookie.name().starts_with(IM_FRIEND_IDS_COOKIE_PREFIX)
        })
        .cloned()
        .collect()
}

pub(crate) fn is_login_cookie_candidate(cookie: &tauri::webview::Cookie<'static>) -> bool {
    let name = cookie.name();
    if name == RELATION_SIGNER_COOKIE_NAME || name.starts_with(IM_FRIEND_IDS_COOKIE_PREFIX) {
        return true;
    }
    cookie
        .domain()
        .map(|domain| {
            let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
            "www.douyin.com" == domain || "www.douyin.com".ends_with(&format!(".{}", domain))
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
}

pub(crate) fn inject_relation_signer_probe(window: &tauri::WebviewWindow) {
    // NOTE: This JS string is injected verbatim into the webview via Tauri's
    // window.eval() API. It does NOT use JS eval() at runtime.
    let script = include_str!("login_window_probe.js");
    if let Err(error) = window.eval(script) {
        log::debug!("failed to inject relation signer probe: {}", error);
    }
}
