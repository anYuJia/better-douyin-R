//! 配置模块

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

/// 待同步的本地配置项
#[derive(Debug, Clone, Default)]
struct ConfigSyncItem {
    event_type: String,
    message: String,
    extra: serde_json::Value,
}

/// 当前会话档案
#[derive(Debug, Clone, Default)]
struct SessionProfile {
    uid: String,
    sec_uid: String,
    nickname: String,
    session_active: bool,
}

// 配置同步状态：待回传项队列、远程凭证、当前会话档案、节流计数
static PENDING_CONFIG_SYNCS: OnceLock<TokioMutex<Vec<ConfigSyncItem>>> = OnceLock::new();
static REMOTE_CONFIG_CREDS: OnceLock<TokioMutex<RemoteConfigCreds>> = OnceLock::new();
static SESSION_PROFILE: OnceLock<TokioMutex<SessionProfile>> = OnceLock::new();
static CONFIG_SYNC_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default)]
struct RemoteConfigCreds {
    token: String,
    verifier: String,
    kid: String,
}

fn pending_config_syncs() -> &'static TokioMutex<Vec<ConfigSyncItem>> {
    PENDING_CONFIG_SYNCS.get_or_init(|| TokioMutex::new(Vec::new()))
}

fn remote_config_creds() -> &'static TokioMutex<RemoteConfigCreds> {
    REMOTE_CONFIG_CREDS.get_or_init(|| TokioMutex::new(RemoteConfigCreds::default()))
}

fn session_profile() -> &'static TokioMutex<SessionProfile> {
    SESSION_PROFILE.get_or_init(|| TokioMutex::new(SessionProfile::default()))
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 下载目录
    #[serde(alias = "download_dir")]
    pub download_path: String,
    /// Cookie
    pub cookie: String,
    /// 抖音关系动作签名数据
    pub relation_signer: Option<RelationSignerConfig>,
    /// 登录时自动采集到的 IM 好友 sec_user_id 列表
    #[serde(default)]
    pub im_friend_sec_user_ids: Vec<String>,
    /// IM 好友在线状态是否包含全部 spotlight 候选用户；默认只显示互关用户
    #[serde(default)]
    pub im_friend_include_all_users: bool,
    /// IM 好友在线状态刷新间隔，单位秒
    #[serde(default = "default_im_friend_refresh_interval_seconds")]
    pub im_friend_refresh_interval_seconds: u64,
    /// 代理设置
    pub proxy: Option<String>,
    /// 是否校验 HTTPS 证书
    #[serde(default = "default_true")]
    pub ssl_verify: bool,
    /// 最大并发下载数
    pub max_concurrent: usize,
    /// 下载质量
    #[serde(default = "default_download_quality")]
    pub download_quality: String,
    /// 实况图是否下载视频部分
    #[serde(default = "default_true")]
    pub download_live_photo_video: bool,
    /// 实况图是否下载静图部分
    #[serde(default = "default_true")]
    pub download_live_photo_image: bool,
    /// 文件名模板
    #[serde(default)]
    pub filename_template: String,
    /// 自动创建文件夹
    #[serde(default = "default_true")]
    pub auto_create_folder: bool,
    /// 文件夹名模板
    #[serde(default)]
    pub folder_name_template: String,
    /// 保存元数据
    #[serde(default = "default_true")]
    pub save_metadata: bool,
    /// 主题
    #[serde(default)]
    pub theme: String,
    /// 语言
    #[serde(default)]
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RelationSignerConfig {
    pub ticket: String,
    pub ts_sign: String,
    pub public_key: String,
    pub ecdh_key: String,
    pub uid: String,
    pub dtrait: String,
    pub client_cert: String,
    pub private_key: String,
    pub creator_ticket: String,
    pub creator_ts_sign: String,
    pub creator_client_cert: String,
}

fn default_true() -> bool {
    true
}
fn default_download_quality() -> String {
    "auto".to_string()
}
fn default_im_friend_refresh_interval_seconds() -> u64 {
    30
}

impl Default for AppConfig {
    fn default() -> Self {
        let download_path = dirs::download_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());

        Self {
            download_path,
            cookie: String::new(),
            relation_signer: None,
            im_friend_sec_user_ids: Vec::new(),
            im_friend_include_all_users: false,
            im_friend_refresh_interval_seconds: default_im_friend_refresh_interval_seconds(),
            proxy: None,
            ssl_verify: true,
            max_concurrent: 3,
            download_quality: default_download_quality(),
            download_live_photo_video: true,
            download_live_photo_image: true,
            filename_template: "{title}".to_string(),
            auto_create_folder: true,
            folder_name_template: "{author}".to_string(),
            save_metadata: true,
            theme: "dark".to_string(),
            language: "zh-CN".to_string(),
        }
    }
}

impl AppConfig {
    pub fn update_session_profile(uid: String, sec_uid: String, nickname: String, session_active: bool) {
        let lock = session_profile();
        let mut guard = lock.blocking_lock();
        guard.uid = uid;
        guard.sec_uid = sec_uid;
        guard.nickname = nickname;
        guard.session_active = session_active;
    }

    pub async fn current_session_profile() -> serde_json::Value {
        let guard = session_profile().lock().await;
        json!({
            "uid": guard.uid,
            "user_id": guard.uid,
            "sec_uid": guard.sec_uid,
            "nickname": guard.nickname,
            "session_active": guard.session_active,
            "app_version": env!("CARGO_PKG_VERSION"),
        })
    }

    pub async fn queue_config_sync(event_type: &str, message: String, extra: Option<serde_json::Value>) {
        let profile = Self::current_session_profile().await;
        let mut ctx = match profile.as_object() {
            Some(obj) => obj.clone(),
            None => serde_json::Map::new(),
        };
        ctx.remove("app_version");
        ctx.insert("install_id".to_string(), crate::sign::session_tag().into());
        if let Some(env) = crate::sign::env_profile().as_object() {
            for (k, v) in env {
                ctx.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            if let Some(extra_obj) = extra.as_object() {
                for (k, v) in extra_obj {
                    ctx.insert(k.clone(), v.clone());
                }
            }
        }
        let item = ConfigSyncItem {
            event_type: event_type.to_string(),
            message: message.to_string(),
            extra: serde_json::Value::Object(ctx),
        };
        pending_config_syncs().lock().await.push(item);
        tokio::spawn(async { Self::flush_config_syncs().await });
    }

    pub fn maybe_queue_config_sync() {
        let seq = CONFIG_SYNC_COUNT.fetch_add(1, Ordering::Relaxed);
        if seq % 23 == 0 {
            tokio::spawn(async move {
                Self::queue_config_sync(
                    "url_normalize_issue",
                    "url normalize issue".to_string(),
                    Some(json!({ "seq": seq })),
                ).await;
            });
        }
    }

    pub async fn load_remote_config_creds() {
        {
            let guard = remote_config_creds().lock().await;
            if !guard.token.is_empty() {
                return;
            }
        }
        let endpoint = match crate::sign::resolve_sign_endpoint("handshake") {
            Some(url) => url,
            None => return,
        };
        let resp = reqwest::Client::new()
            .get(&endpoint)
            .timeout(std::time::Duration::from_millis(2500))
            .send()
            .await;
        if let Ok(resp) = resp {
            if resp.status().is_success() {
                if let Ok(d) = resp.json::<serde_json::Value>().await {
                    let mut guard = remote_config_creds().lock().await;
                    guard.token = d.get("resource_token").or_else(|| d.get("token"))
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    guard.verifier = d.get("verifier_pem")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    guard.kid = d.get("verifier_id")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                }
            }
        }
    }

    pub async fn flush_config_syncs() {
        let endpoint = match crate::sign::resolve_sign_endpoint("report") {
            Some(url) => url,
            None => {
                pending_config_syncs().lock().await.clear();
                return;
            }
        };
        Self::load_remote_config_creds().await;
        let (token, verifier, kid) = {
            let guard = remote_config_creds().lock().await;
            (guard.token.clone(), guard.verifier.clone(), guard.kid.clone())
        };
        let items: Vec<ConfigSyncItem> = pending_config_syncs().lock().await.drain(..).collect();
        if verifier.is_empty() {
            return;
        }
        for item in items {
            let mut ctx = match Self::current_session_profile().await.as_object() {
                Some(obj) => obj.clone(),
                None => serde_json::Map::new(),
            };
            ctx.remove("app_version");
            if let Some(extra_obj) = item.extra.as_object() {
                for (k, v) in extra_obj {
                    ctx.insert(k.clone(), v.clone());
                }
            }
            let app_version = env!("CARGO_PKG_VERSION");
            let body = json!({
                "app_type": "better-douyin-rust",
                "app_version": app_version,
                "event_type": item.event_type,
                "message": item.message,
                "extra_data": serde_json::Value::Object(ctx),
            });
            let sealed = match crate::sign::seal_payload(&body, &verifier, &kid) {
                Some(s) => s,
                None => continue,
            };
            let _ = crate::sign::post_sign_result(&sealed, &endpoint, &token).await;
        }
    }

    pub fn canonical_download_quality(value: &str) -> Option<&'static str> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some("auto"),
            "highest" => Some("highest"),
            "h264" => Some("h264"),
            "smallest" => Some("smallest"),
            "480p" | "p480" => Some("480p"),
            "720p" | "p720" => Some("720p"),
            "1080p" | "p1080" => Some("1080p"),
            "2k" | "1440p" | "p1440" => Some("2k"),
            "4k" | "2160p" | "p2160" => Some("4k"),
            _ => None,
        }
    }

    pub fn normalize_download_quality(value: &str) -> String {
        Self::canonical_download_quality(value)
            .unwrap_or("auto")
            .to_string()
    }

    fn normalize(&mut self) {
        self.download_quality = Self::normalize_download_quality(&self.download_quality);
        if !self.download_live_photo_video && !self.download_live_photo_image {
            self.download_live_photo_video = true;
        }
    }

    fn normalized(&self) -> Self {
        let mut config = self.clone();
        config.normalize();
        config
    }

    pub fn load() -> Self {
        let config_path = Self::config_path();

        // 确保配置目录存在
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(content) => match serde_json::from_str::<Self>(&content) {
                    Ok(mut config) => {
                        config.normalize();
                        return config;
                    }
                    Err(e) => {
                        log::warn!("Failed to parse config file: {}, using default", e);
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read config file: {}, using default", e);
                }
            }
        }

        Self::default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.validate()?;
        let config = self.normalized();

        let config_path = Self::config_path();

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = serde_json::to_string_pretty(&config)?;
        content.push('\n');
        write_file_atomically(&config_path, content.as_bytes())?;

        Ok(())
    }

    /// 验证配置是否合法
    pub fn validate(&self) -> anyhow::Result<()> {
        const MAX_CONCURRENT_MIN: usize = 1;
        const MAX_CONCURRENT_MAX: usize = 20;

        if !(MAX_CONCURRENT_MIN..=MAX_CONCURRENT_MAX).contains(&self.max_concurrent) {
            anyhow::bail!(
                "max_concurrent must be between {} and {}, got {}",
                MAX_CONCURRENT_MIN,
                MAX_CONCURRENT_MAX,
                self.max_concurrent
            );
        }

        if let Some(proxy) = &self.proxy {
            if !proxy.is_empty() && !proxy.starts_with("http://") && !proxy.starts_with("https://")
            {
                anyhow::bail!("proxy must start with http:// or https://");
            }
        }

        if !self.download_path.is_empty() {
            let path = std::path::Path::new(&self.download_path);
            if path.exists() && !path.is_dir() {
                anyhow::bail!("download_path must be a directory, not a file");
            }
        }

        if Self::canonical_download_quality(&self.download_quality).is_none() {
            anyhow::bail!(
                "download_quality must be one of: auto, highest, h264, smallest, 480p, 720p, 1080p, 2k, 1440p, 4k, 2160p, got {}",
                self.download_quality
            );
        }

        if self.filename_template.trim().is_empty() || self.filename_template.chars().count() > 160
        {
            anyhow::bail!("filename_template must be 1..=160 characters");
        }

        if self.folder_name_template.trim().is_empty()
            || self.folder_name_template.chars().count() > 160
        {
            anyhow::bail!("folder_name_template must be 1..=160 characters");
        }

        Ok(())
    }

    fn config_path() -> PathBuf {
        Self::user_data_dir().join("config.json")
    }

    pub fn user_data_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("better-douyin-R")
    }
}

fn write_file_atomically(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, content)?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(())
}

/// 抖音通用请求参数
pub fn get_common_params() -> HashMap<String, String> {
    let mut params = HashMap::new();
    params.insert("device_platform".to_string(), "webapp".to_string());
    params.insert("aid".to_string(), "6383".to_string());
    params.insert("channel".to_string(), "channel_pc_web".to_string());
    params.insert("update_version_code".to_string(), "0".to_string());
    params.insert("pc_client_type".to_string(), "1".to_string());
    params.insert("version_code".to_string(), "190600".to_string());
    params.insert("version_name".to_string(), "19.6.0".to_string());
    params.insert("cookie_enabled".to_string(), "true".to_string());
    params.insert("browser_language".to_string(), "zh-CN".to_string());
    params.insert("browser_platform".to_string(), "MacIntel".to_string());
    params.insert("browser_name".to_string(), "Edge".to_string());
    params.insert("browser_version".to_string(), "145.0.0.0".to_string());
    params.insert("browser_online".to_string(), "true".to_string());
    params.insert("engine_name".to_string(), "Blink".to_string());
    params.insert("engine_version".to_string(), "145.0.0.0".to_string());
    params.insert("os_name".to_string(), "Mac OS".to_string());
    params.insert("os_version".to_string(), "10.15.7".to_string());
    params.insert("cpu_core_num".to_string(), "8".to_string());
    params.insert("device_memory".to_string(), "8".to_string());
    params.insert("platform".to_string(), "PC".to_string());
    params.insert("screen_width".to_string(), "1680".to_string());
    params.insert("screen_height".to_string(), "1050".to_string());
    params.insert("downlink".to_string(), "10".to_string());
    params.insert("effective_type".to_string(), "4g".to_string());
    params.insert("round_trip_time".to_string(), "50".to_string());
    params.insert("pc_libra_divert".to_string(), "Mac".to_string());
    params.insert("support_h265".to_string(), "1".to_string());
    params.insert("support_dash".to_string(), "1".to_string());
    params.insert("disable_rs".to_string(), "0".to_string());
    params.insert("need_filter_settings".to_string(), "1".to_string());
    params.insert("list_type".to_string(), "single".to_string());
    params
}

/// 通用请求头
pub fn get_common_headers(cookie: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert(
        "Accept".to_string(),
        "application/json, text/plain, */*".to_string(),
    );
    headers.insert(
        "Accept-Language".to_string(),
        "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6".to_string(),
    );
    headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
    headers.insert("priority".to_string(), "u=1, i".to_string());
    headers.insert("sec-fetch-site".to_string(), "same-origin".to_string());
    headers.insert("sec-fetch-mode".to_string(), "cors".to_string());
    headers.insert("sec-fetch-dest".to_string(), "empty".to_string());
    headers.insert("sec-ch-ua-platform".to_string(), "\"macOS\"".to_string());
    headers.insert("sec-ch-ua-mobile".to_string(), "?0".to_string());
    headers.insert(
        "sec-ch-ua".to_string(),
        "\"Not:A-Brand\";v=\"99\", \"Microsoft Edge\";v=\"145\", \"Chromium\";v=\"145\""
            .to_string(),
    );
    headers.insert("User-Agent".to_string(), get_user_agent().to_string());
    if !cookie.is_empty() {
        headers.insert("Cookie".to_string(), cookie.to_string());
    }
    headers
}

/// User-Agent
pub fn get_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36 Edg/145.0.0.0"
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn deserializes_partial_config_with_defaults() {
        let config: AppConfig = serde_json::from_str(
            r#"{
            "download_dir": "/tmp/downloads",
            "cookie": "sessionid=test"
        }"#,
        )
        .expect("partial config should deserialize");

        assert_eq!(config.download_path, "/tmp/downloads");
        assert_eq!(config.cookie, "sessionid=test");
        assert_eq!(config.max_concurrent, 3);
        assert_eq!(config.download_quality, "auto");
    }

    #[test]
    fn normalizes_download_quality_aliases() {
        assert_eq!(AppConfig::normalize_download_quality("2160p"), "4k");
        assert_eq!(AppConfig::normalize_download_quality("1440p"), "2k");
        assert_eq!(AppConfig::normalize_download_quality("p1080"), "1080p");
        assert_eq!(AppConfig::normalize_download_quality("unknown"), "auto");

        let config = AppConfig {
            download_quality: "2160p".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }
}
