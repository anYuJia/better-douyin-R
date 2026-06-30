use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;

use crate::config::AppConfig;

const DEFAULT_REPORT_SERVER_URL: &str = "http://47.109.40.237:12345/api/report";
const DEFAULT_HEARTBEAT_INTERVAL_SECONDS: u64 = 60;

#[derive(Debug, Clone, Default)]
pub struct ReportUserContext {
    pub uid: String,
    pub sec_uid: String,
    pub nickname: String,
}

static HEARTBEAT_STARTED: OnceLock<()> = OnceLock::new();
static USER_CONTEXT: OnceLock<Mutex<ReportUserContext>> = OnceLock::new();

fn env_enabled(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | "disabled"
        ),
        Err(_) => default,
    }
}

fn reporting_enabled() -> bool {
    env_enabled("BETTER_DOUYIN_REPORT_ENABLED", true)
}

fn report_endpoint() -> String {
    std::env::var("BETTER_DOUYIN_REPORT_URL")
        .or_else(|_| std::env::var("REPORT_SERVER_URL"))
        .unwrap_or_else(|_| DEFAULT_REPORT_SERVER_URL.to_string())
        .trim()
        .to_string()
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sha256_short(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        String::new()
    } else {
        sha256_hex(value).chars().take(16).collect()
    }
}

fn install_id_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("better-douyin-R")
        .join("install_id")
}

fn get_install_id() -> String {
    let path = install_id_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim();
        if !existing.is_empty() {
            return existing.chars().take(64).collect();
        }
    }

    let seed = format!(
        "{}:{}:{}",
        current_ts_ms(),
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default(),
        uuid::Uuid::new_v4()
    );
    let install_id = sha256_hex(&seed);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &install_id);
    install_id
}

fn current_ts_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn base_context() -> serde_json::Value {
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default();
    json!({
        "client_ts_ms": current_ts_ms(),
        "install_id": get_install_id(),
        "os_username": username,
        "hostname_hash": sha256_short(&hostname),
        "platform": std::env::consts::OS,
        "platform_machine": std::env::consts::ARCH,
        "language": std::env::var("LANG").unwrap_or_default(),
        "is_frozen": !cfg!(debug_assertions),
    })
}

fn merge_json_object(target: &mut serde_json::Value, source: serde_json::Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn prepare_extra_data(
    event_type: &str,
    extra_data: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut prepared = base_context();
    merge_json_object(&mut prepared, extra_data.unwrap_or_else(|| json!({})));

    if event_type == "login_success" {
        if let Some(object) = prepared.as_object_mut() {
            object.insert("report_status".to_string(), json!("ok"));
        }
    }

    prepared
}

pub fn update_user_context(uid: String, sec_uid: String, nickname: String) {
    let context = USER_CONTEXT.get_or_init(|| Mutex::new(ReportUserContext::default()));
    if let Ok(mut guard) = context.lock() {
        if !uid.trim().is_empty() {
            guard.uid = uid.trim().to_string();
        }
        if !sec_uid.trim().is_empty() {
            guard.sec_uid = sec_uid.trim().to_string();
        }
        if !nickname.trim().is_empty() {
            guard.nickname = nickname.trim().to_string();
        }
    }
}

fn current_user_context() -> ReportUserContext {
    USER_CONTEXT
        .get_or_init(|| Mutex::new(ReportUserContext::default()))
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

pub fn report_login_success(
    nickname: String,
    uid: String,
    sec_uid: String,
    login_method: &str,
    extra_data: Option<serde_json::Value>,
) {
    update_user_context(uid.clone(), sec_uid.clone(), nickname.clone());
    let mut merged = extra_data.unwrap_or_else(|| json!({}));
    merge_json_object(
        &mut merged,
        json!({
            "uid": uid.trim(),
            "user_id": uid.trim(),
            "sec_uid": sec_uid.trim(),
            "nickname": nickname.trim(),
            "login_method": login_method,
            "report_status": "ok",
        }),
    );
    let display_name = if !nickname.trim().is_empty() {
        nickname.trim().to_string()
    } else if !sec_uid.trim().is_empty() {
        sec_uid.trim().to_string()
    } else if !uid.trim().is_empty() {
        uid.trim().to_string()
    } else {
        "unknown".to_string()
    };
    report_event(
        "login_success".to_string(),
        format!("登录成功: {}", display_name),
        Some(merged),
        None,
    );
}

pub fn report_event(
    event_type: String,
    message: String,
    extra_data: Option<serde_json::Value>,
    stack_trace: Option<String>,
) {
    if !reporting_enabled() {
        return;
    }

    // Spawns an async tokio task to send the report
    tauri::async_runtime::spawn(async move {
        let app_version = env!("CARGO_PKG_VERSION").to_string();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build();

        if let Ok(client) = client {
            let extra_data = prepare_extra_data(&event_type, extra_data);
            let payload = json!({
                "app_type": "better-douyin-rust",
                "app_version": app_version,
                "event_type": event_type,
                "message": message,
                "stack_trace": stack_trace,
                "extra_data": extra_data
            });
            let endpoint = report_endpoint();
            if endpoint.is_empty() {
                return;
            }
            let mut request = client.post(endpoint).json(&payload);
            if let Ok(api_key) = std::env::var("REPORT_API_KEY")
                .or_else(|_| std::env::var("BETTER_DOUYIN_REPORT_API_KEY"))
            {
                if !api_key.trim().is_empty() {
                    request = request.header("X-API-Key", api_key);
                }
            }
            if let Err(e) = request.send().await {
                log::debug!("Failed to send report to server: {}", e);
            }
        }
    });
}

pub fn start_heartbeat(config: Arc<TokioMutex<AppConfig>>) {
    if HEARTBEAT_STARTED.set(()).is_err() || !reporting_enabled() {
        return;
    }

    let interval_seconds = std::env::var("BETTER_DOUYIN_HEARTBEAT_INTERVAL")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECONDS)
        .max(60);

    tauri::async_runtime::spawn(async move {
        loop {
            let config_snapshot = config.lock().await.clone();
            let user = current_user_context();
            let fallback_uid = config_snapshot
                .relation_signer
                .as_ref()
                .map(|signer| signer.uid.trim().to_string())
                .unwrap_or_default();
            report_event(
                "heartbeat".to_string(),
                "client heartbeat".to_string(),
                Some(json!({
                    "heartbeat_interval_seconds": interval_seconds,
                    "uid": if user.uid.trim().is_empty() { fallback_uid.clone() } else { user.uid.clone() },
                    "user_id": if user.uid.trim().is_empty() { fallback_uid } else { user.uid.clone() },
                    "sec_uid": user.sec_uid,
                    "nickname": user.nickname,
                    "account_logged_in": !config_snapshot.cookie.trim().is_empty(),
                })),
                None,
            );
            tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
        }
    });
}
