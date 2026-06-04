//! 配置模块

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    /// 最大并发下载数
    pub max_concurrent: usize,
    /// 下载质量
    #[serde(default = "default_download_quality")]
    pub download_quality: String,
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
    5
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
            max_concurrent: 3,
            download_quality: default_download_quality(),
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
    pub fn load() -> Self {
        let config_path = Self::config_path();

        // 确保配置目录存在
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(config) => return config,
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

        let config_path = Self::config_path();

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = serde_json::to_string_pretty(self)?;
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

        if !matches!(
            self.download_quality.as_str(),
            "auto" | "highest" | "h264" | "smallest"
        ) {
            anyhow::bail!(
                "download_quality must be one of: auto, highest, h264, smallest, got {}",
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
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("douyin-downloader")
            .join("config.json")
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
}
