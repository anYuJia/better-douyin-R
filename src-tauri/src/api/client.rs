//! API 客户端

use crate::config::{get_user_agent, AppConfig};
use crate::sign;
use anyhow::{anyhow, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use openssl::bn::BigNumContext;
use openssl::ec::{EcKey, PointConversionForm};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use rand::{distributions::Alphanumeric, Rng};
use regex::Regex;
use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::types::*;

static WEBID_PATTERNS: &[&str] = &[
    r#"\\"user_unique_id\\":\\"(\d+)\\""#,
    r#""user_unique_id":"(\d+)""#,
    r#""webid":"(\d+)""#,
    r#"webid=(\d+)"#,
];
static WEBID_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    WEBID_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
});
static SHARE_URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>"']+|www\.[^\s<>"']+"#).unwrap());
static AWEME_ID_DIGIT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+$").unwrap());
static AWEME_ID_PATTERNS: &[&str] = &[
    r"video/(\d+)",
    r"note/(\d+)",
    r"aweme_id=(\d+)",
    r"modal_id=(\d+)",
    r"/(\d{18,21})",
];
static AWEME_ID_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    AWEME_ID_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
});

fn looks_watermarked_media_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("watermark=1") || lower.contains("playwm") || lower.contains("logo_name=")
}

fn is_dash_video_only_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("media-video") || lower.contains("media_video")
}

fn normalize_recommended_feed_type(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "recommended" | "recommend" | "tab" | "home" | "feed" => "recommended",
        _ => "featured",
    }
}

fn is_valid_recommended_video(video: &VideoInfo) -> bool {
    !video.aweme_id.trim().is_empty()
        && !video.video.play_addr.trim().is_empty()
        && !video.video.cover.trim().is_empty()
        && (!video.author.sec_uid.trim().is_empty()
            || !video.author.uid.trim().is_empty()
            || !video.author.nickname.trim().is_empty())
}

fn clean_video_media_url(url: &str) -> String {
    url.trim()
        .replace("watermark=1", "watermark=0")
        .replace("playwm", "play")
}

type HmacSha256 = Hmac<Sha256>;

/// 抖音 API 客户端
#[derive(Clone)]
pub struct DouyinClient {
    pub(super) client: reqwest::Client,
    pub(super) config: AppConfig,
    webid_cache: Arc<Mutex<Option<(String, Instant)>>>,
    cookie_dict: Arc<HashMap<String, String>>,
}

impl DouyinClient {
    pub fn new(config: AppConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(Policy::limited(5))
            .danger_accept_invalid_certs(false);

        if let Some(proxy) = &config.proxy {
            if !proxy.is_empty() {
                builder = builder.proxy(reqwest::Proxy::all(proxy)?);
            }
        }

        let client = builder.build()?;
        let cookie_dict = Arc::new(Self::cookies_to_dict(&config.cookie));

        Ok(Self {
            client,
            config,
            webid_cache: Arc::new(Mutex::new(None)),
            cookie_dict,
        })
    }

    /// 返回当前 cookie 的解析结果（构造时一次性解析并缓存）。
    /// 调用方按需 clone 或通过引用访问。
    pub(super) fn cookie_dict(&self) -> &HashMap<String, String> {
        &self.cookie_dict
    }

    fn cookies_to_dict(cookie_str: &str) -> HashMap<String, String> {
        let mut cookie_dict = HashMap::new();

        for item in cookie_str.split(';') {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some((key, value)) = trimmed.split_once('=') {
                cookie_dict.insert(key.trim().to_string(), value.to_string());
            }
        }

        cookie_dict
    }

    pub fn cookie(&self) -> &str {
        &self.config.cookie
    }

    pub fn im_session_id(&self) -> Option<String> {
        let cookie_dict = self.cookie_dict();
        cookie_dict
            .get("sessionid")
            .or_else(|| cookie_dict.get("sessionid_ss"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub(super) fn ticket_guard_headers_from_cookie(cookie_str: &str) -> HashMap<String, String> {
        let cookie_dict = Self::cookies_to_dict(cookie_str);
        let mut headers = HashMap::new();

        if let Some(raw_legacy_client_data) = cookie_dict.get("bd_ticket_guard_client_data") {
            let legacy_decoded = urlencoding::decode(raw_legacy_client_data)
                .map(|value| value.into_owned())
                .unwrap_or_else(|_| raw_legacy_client_data.clone());
            if let Ok(bytes) =
                base64::engine::general_purpose::STANDARD.decode(legacy_decoded.as_bytes())
            {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(object) = value.as_object() {
                        for (key, value) in object {
                            if key.starts_with("bd-ticket-guard-") {
                                if let Some(value) = value.as_str() {
                                    headers.insert(key.clone(), value.to_string());
                                } else if value.is_number() || value.is_boolean() {
                                    headers.insert(key.clone(), value.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        let raw_client_data_v2 = cookie_dict.get("bd_ticket_guard_client_data_v2");
        let raw_client_data =
            raw_client_data_v2.or_else(|| cookie_dict.get("bd_ticket_guard_client_data"));
        let Some(raw_client_data) = raw_client_data else {
            return headers;
        };

        let decoded = urlencoding::decode(raw_client_data)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| raw_client_data.clone());
        if raw_client_data_v2.is_some() {
            headers.insert("bd-ticket-guard-client-data".to_string(), decoded.clone());
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(decoded.as_bytes()) else {
            return headers;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return headers;
        };

        if let Some(object) = value.as_object() {
            for (key, value) in object {
                if key.starts_with("bd-ticket-guard-") {
                    if let Some(value) = value.as_str() {
                        headers.insert(key.clone(), value.to_string());
                    } else if value.is_number() || value.is_boolean() {
                        headers.insert(key.clone(), value.to_string());
                    }
                }
            }

            if !headers.contains_key("bd-ticket-guard-ree-public-key") {
                if let Some(public_key) =
                    value.get("ree_public_key").and_then(|value| value.as_str())
                {
                    headers.insert(
                        "bd-ticket-guard-ree-public-key".to_string(),
                        public_key.to_string(),
                    );
                }
            }

            headers
                .entry("bd-ticket-guard-web-sign-type".to_string())
                .or_insert_with(|| {
                    if raw_client_data_v2.is_some() {
                        "1"
                    } else {
                        "0"
                    }
                    .to_string()
                });
        }

        headers
    }

    fn cookie_string_with_value(cookie_str: &str, name: &str, value: &str) -> String {
        let mut found = false;
        let mut parts = Vec::new();
        for item in cookie_str.split(';') {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((key, _)) = trimmed.split_once('=') else {
                parts.push(trimmed.to_string());
                continue;
            };
            if key.trim() == name {
                parts.push(format!("{name}={value}"));
                found = true;
            } else {
                parts.push(trimmed.to_string());
            }
        }
        if !found {
            parts.push(format!("{name}={value}"));
        }
        parts.join("; ")
    }

    fn decode_relation_ecdh_key(value: &str) -> Option<Vec<u8>> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
            let mut out = Vec::with_capacity(32);
            for index in (0..trimmed.len()).step_by(2) {
                let byte = u8::from_str_radix(&trimmed[index..index + 2], 16).ok()?;
                out.push(byte);
            }
            return Some(out);
        }

        base64::engine::general_purpose::STANDARD
            .decode(trimmed.as_bytes())
            .ok()
    }

    pub(super) fn relation_ticket_guard_headers(&self, path: &str) -> HashMap<String, String> {
        let Some(signer) = self.config.relation_signer.as_ref() else {
            return Self::ticket_guard_headers_from_cookie(&self.config.cookie);
        };
        let ticket = signer.ticket.trim();
        let ts_sign = signer.ts_sign.trim();
        let public_key = signer.public_key.trim();
        let Some(ecdh_key) = Self::decode_relation_ecdh_key(&signer.ecdh_key) else {
            log::warn!("Douyin relation signer ecdh_key is unavailable or invalid");
            return Self::ticket_guard_headers_from_cookie(&self.config.cookie);
        };
        if ticket.is_empty() || ts_sign.is_empty() || public_key.is_empty() {
            log::warn!("Douyin relation signer is incomplete");
            return Self::ticket_guard_headers_from_cookie(&self.config.cookie);
        }

        let timestamp = chrono::Utc::now().timestamp();
        let sign_data = format!("ticket={ticket}&path={path}&timestamp={timestamp}");
        let mut mac = match HmacSha256::new_from_slice(&ecdh_key) {
            Ok(mac) => mac,
            Err(_) => return Self::ticket_guard_headers_from_cookie(&self.config.cookie),
        };
        mac.update(sign_data.as_bytes());
        let req_sign =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let client_data = serde_json::json!({
            "ts_sign": ts_sign,
            "req_content": "ticket,path,timestamp",
            "req_sign": req_sign,
            "timestamp": timestamp,
        });
        let client_data = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&client_data).unwrap_or_default());

        HashMap::from([
            (
                "bd-ticket-guard-ree-public-key".to_string(),
                public_key.to_string(),
            ),
            ("bd-ticket-guard-web-version".to_string(), "2".to_string()),
            ("bd-ticket-guard-web-sign-type".to_string(), "1".to_string()),
            ("bd-ticket-guard-version".to_string(), "2".to_string()),
            (
                "bd-ticket-guard-iteration-version".to_string(),
                "1".to_string(),
            ),
            ("bd-ticket-guard-client-data".to_string(), client_data),
        ])
    }

    fn spider_ticket_guard_headers(&self, path: &str) -> Result<HashMap<String, String>> {
        let signer = self
            .config
            .relation_signer
            .as_ref()
            .ok_or_else(|| anyhow!("评论发布安全参数缺失，请重新登录获取 Cookie"))?;
        let ticket = signer.ticket.trim();
        let ts_sign = signer.ts_sign.trim();
        let private_key = signer.private_key.trim().replace("\\n", "\n");
        if ticket.is_empty() || ts_sign.is_empty() || private_key.is_empty() {
            return Err(anyhow!("评论发布安全参数不完整，请重新登录获取 Cookie"));
        }

        let timestamp = chrono::Utc::now().timestamp();
        let sign_data = format!("ticket={ticket}&path={path}&timestamp={timestamp}");
        let req_sign = Self::spider_req_sign(&sign_data, &private_key)?;
        let client_data = serde_json::json!({
            "ts_sign": ts_sign,
            "req_content": "ticket,path,timestamp",
            "req_sign": req_sign,
            "timestamp": timestamp,
        });
        let client_data = base64::engine::general_purpose::URL_SAFE
            .encode(serde_json::to_vec(&client_data).unwrap_or_default());
        let ree_public_key = Self::spider_ree_key(&private_key)?;

        Ok(HashMap::from([
            ("bd-ticket-guard-client-data".to_string(), client_data),
            (
                "bd-ticket-guard-iteration-version".to_string(),
                "1".to_string(),
            ),
            ("bd-ticket-guard-ree-public-key".to_string(), ree_public_key),
            ("bd-ticket-guard-version".to_string(), "2".to_string()),
            ("bd-ticket-guard-web-version".to_string(), "1".to_string()),
        ]))
    }

    fn relation_uid_hash(&self) -> Option<String> {
        let cookie_dict = self.cookie_dict();
        let uid = self
            .config
            .relation_signer
            .as_ref()
            .map(|signer| signer.uid.trim().to_string())
            .filter(|uid| !uid.is_empty())
            .or_else(|| cookie_dict.get("uid_tt").cloned())
            .or_else(|| cookie_dict.get("uid_tt_ss").cloned())?;
        if uid.is_empty() {
            return None;
        }
        if uid.len() == 32 && uid.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Some(uid.to_ascii_lowercase());
        }
        Some(format!("{:x}", md5::compute(uid.as_bytes())))
    }

    pub(super) fn relation_dtrait(&self) -> Option<String> {
        let value = self.config.relation_signer.as_ref()?.dtrait.trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }


    pub(super) fn generate_ms_token() -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(107)
            .map(char::from)
            .collect()
    }

    pub(super) fn generate_verify_fp() -> String {
        let random_str: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect::<String>()
            .to_lowercase();
        format!("verify_0{}", random_str)
    }

    fn generate_fake_webid() -> String {
        const DIGITS: &[u8] = b"0123456789";
        let mut rng = rand::thread_rng();
        (0..19)
            .map(|_| DIGITS[rng.gen_range(0..DIGITS.len())] as char)
            .collect()
    }

    fn spider_req_sign(sign_data: &str, private_key: &str) -> Result<String> {
        let key = PKey::private_key_from_pem(private_key.as_bytes())
            .map_err(|error| anyhow!("TicketGuard 私钥解析失败: {}", error))?;
        let mut signer = Signer::new(MessageDigest::sha256(), &key)
            .map_err(|error| anyhow!("TicketGuard 签名初始化失败: {}", error))?;
        signer
            .update(sign_data.as_bytes())
            .map_err(|error| anyhow!("TicketGuard 签名写入失败: {}", error))?;
        let signature = signer
            .sign_to_vec()
            .map_err(|error| anyhow!("TicketGuard 签名生成失败: {}", error))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(signature))
    }

    fn spider_ree_key(private_key: &str) -> Result<String> {
        let key = EcKey::private_key_from_pem(private_key.as_bytes())
            .or_else(|_| {
                PKey::private_key_from_pem(private_key.as_bytes()).and_then(|key| key.ec_key())
            })
            .map_err(|error| anyhow!("TicketGuard EC 私钥解析失败: {}", error))?;
        let mut context = BigNumContext::new()
            .map_err(|error| anyhow!("TicketGuard 公钥导出初始化失败: {}", error))?;
        let public_key = key
            .public_key()
            .to_bytes(key.group(), PointConversionForm::UNCOMPRESSED, &mut context)
            .map_err(|error| anyhow!("TicketGuard 公钥导出失败: {}", error))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(public_key))
    }

    fn sign_spider_a_bogus(query: &str, body: &str) -> Result<String> {
        Ok(sign::sign_spider_publish(query, body))
    }

    fn spider_quote(value: &str) -> String {
        urlencoding::encode(value)
            .replace("%2F", "/")
            .replace("%2f", "/")
    }

    fn spider_splice_params(params: &[(String, String)]) -> String {
        params
            .iter()
            .map(|(key, value)| format!("{key}={}", Self::spider_quote(value)))
            .collect::<Vec<_>>()
            .join("&")
    }

    fn aws_quote(value: &str) -> String {
        urlencoding::encode(value)
            .replace('+', "%20")
            .replace("%7E", "~")
    }

    fn aws_canonical_query(params: &BTreeMap<String, String>) -> String {
        params
            .iter()
            .map(|(key, value)| format!("{}={}", Self::aws_quote(key), Self::aws_quote(value)))
            .collect::<Vec<_>>()
            .join("&")
    }

    fn aws_signing_key(secret_access_key: &str, date_stamp: &str) -> Result<Vec<u8>> {
        let mut mac = HmacSha256::new_from_slice(format!("AWS4{secret_access_key}").as_bytes())?;
        mac.update(date_stamp.as_bytes());
        let k_date = mac.finalize().into_bytes();

        let mut mac = HmacSha256::new_from_slice(&k_date)?;
        mac.update(b"cn-north-1");
        let k_region = mac.finalize().into_bytes();

        let mut mac = HmacSha256::new_from_slice(&k_region)?;
        mac.update(b"vod");
        let k_service = mac.finalize().into_bytes();

        let mut mac = HmacSha256::new_from_slice(&k_service)?;
        mac.update(b"aws4_request");
        Ok(mac.finalize().into_bytes().to_vec())
    }

    pub(super) fn aws_vod_auth_headers(
        method: &str,
        query_params: &BTreeMap<String, String>,
        access_key_id: &str,
        secret_access_key: &str,
        session_token: &str,
        payload_hash: &str,
        extra_signed_headers: BTreeMap<String, String>,
    ) -> Result<(String, BTreeMap<String, String>)> {
        let now = chrono::Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();
        let token = session_token
            .split_once('|')
            .map(|(left, _)| left)
            .unwrap_or(session_token);
        let mut signed_header_values = BTreeMap::from([
            ("x-amz-date".to_string(), amz_date.clone()),
            ("x-amz-security-token".to_string(), token.to_string()),
        ]);
        for (key, value) in extra_signed_headers {
            signed_header_values.insert(key.to_ascii_lowercase(), value);
        }

        let canonical_headers = signed_header_values
            .iter()
            .map(|(key, value)| format!("{}:{}\n", key, value.trim()))
            .collect::<String>();
        let signed_headers = signed_header_values
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(";");
        let canonical_query = Self::aws_canonical_query(query_params);
        let canonical_request = [
            method.to_ascii_uppercase(),
            "/".to_string(),
            canonical_query.clone(),
            canonical_headers,
            signed_headers.clone(),
            payload_hash.to_string(),
        ]
        .join("\n");
        let credential_scope = format!("{date_stamp}/cn-north-1/vod/aws4_request");
        let request_hash = Sha256::digest(canonical_request.as_bytes());
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{:x}",
            request_hash
        );
        let signing_key = Self::aws_signing_key(secret_access_key, &date_stamp)?;
        let mut mac = HmacSha256::new_from_slice(&signing_key)?;
        mac.update(string_to_sign.as_bytes());
        let signature = format!("{:x}", mac.finalize().into_bytes());
        let mut headers = BTreeMap::from([
            (
                "authorization".to_string(),
                format!(
                    "AWS4-HMAC-SHA256 Credential={access_key_id}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
                ),
            ),
            ("x-amz-date".to_string(), amz_date),
            ("x-amz-security-token".to_string(), token.to_string()),
        ]);
        for (key, value) in signed_header_values {
            if key != "x-amz-date" && key != "x-amz-security-token" {
                headers.insert(key, value);
            }
        }
        Ok((canonical_query, headers))
    }

    async fn get_webid(&self, headers: &HashMap<String, String>) -> Option<String> {
        self.get_webid_from_url(headers, "https://www.douyin.com/?recommend=1")
            .await
    }

    async fn get_webid_from_url(
        &self,
        headers: &HashMap<String, String>,
        url: &str,
    ) -> Option<String> {
        {
            let cache = self.webid_cache.lock().await;
            if let Some((webid, cached_at)) = &*cache {
                if cached_at.elapsed() < Duration::from_secs(600) {
                    return Some(webid.clone());
                }
            }
        }

        let mut request_headers = headers.clone();
        request_headers.insert("sec-fetch-dest".to_string(), "document".to_string());
        request_headers.insert("sec-fetch-mode".to_string(), "navigate".to_string());
        request_headers.insert("sec-fetch-site".to_string(), "none".to_string());
        request_headers.insert(
            "Accept".to_string(),
            "text/html,application/xhtml+xml".to_string(),
        );
        request_headers.insert(
            "accept".to_string(),
            "text/html,application/xhtml+xml".to_string(),
        );
        request_headers.insert("upgrade-insecure-requests".to_string(), "1".to_string());
        if !self.config.cookie.trim().is_empty() {
            request_headers.insert("Cookie".to_string(), self.config.cookie.clone());
        }

        let mut req = self.client.get(url);
        for (key, value) in &request_headers {
            req = req.header(key, value);
        }

        let response = req.send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }

        let html = response.text().await.ok()?;

        for re in WEBID_REGEXES.iter() {
            if let Some(caps) = re.captures(&html) {
                if let Some(matched) = caps.get(1) {
                    let webid = matched.as_str().to_string();
                    let mut cache = self.webid_cache.lock().await;
                    *cache = Some((webid.clone(), Instant::now()));
                    return Some(webid);
                }
            }
        }

        None
    }

    async fn get_csrf_token(&self, headers: &HashMap<String, String>) -> Option<String> {
        let mut request_headers = headers.clone();
        request_headers.insert("accept".to_string(), "*/*".to_string());
        request_headers.insert("cache-control".to_string(), "no-cache".to_string());
        request_headers.insert("pragma".to_string(), "no-cache".to_string());
        request_headers.insert(
            "referer".to_string(),
            "https://www.douyin.com/?recommend=1".to_string(),
        );
        request_headers.insert("x-secsdk-csrf-request".to_string(), "1".to_string());
        request_headers.insert("x-secsdk-csrf-version".to_string(), "1.2.22".to_string());
        request_headers.remove("content-type");
        request_headers.remove("Content-Type");

        let mut req = self
            .client
            .head("https://www.douyin.com/service/2/abtest_config/");
        for (key, value) in &request_headers {
            req = req.header(key, value);
        }
        let response = req.send().await.ok()?;
        let raw_token = response
            .headers()
            .get("x-ware-csrf-token")
            .or_else(|| response.headers().get("X-Ware-Csrf-Token"))?
            .to_str()
            .ok()?;
        let parts = raw_token.split(',').map(str::trim).collect::<Vec<_>>();
        parts
            .get(1)
            .filter(|value| !value.is_empty())
            .or_else(|| parts.iter().find(|value| value.len() > 16))
            .map(|value| value.to_string())
    }

    pub(super) async fn enrich_request(
        &self,
        params: &mut HashMap<String, String>,
        headers: &mut HashMap<String, String>,
    ) {
        let cookie = headers
            .get("cookie")
            .or_else(|| headers.get("Cookie"))
            .cloned()
            .unwrap_or_else(|| self.config.cookie.clone());

        if cookie.is_empty() {
            return;
        }

        let mut cookie_dict = Self::cookies_to_dict(&cookie);

        params
            .entry("msToken".to_string())
            .or_insert_with(Self::generate_ms_token);
        if let Some(ms_token) = params.get("msToken").cloned() {
            headers.insert(
                "Cookie".to_string(),
                Self::cookie_string_with_value(&cookie, "msToken", &ms_token),
            );
            cookie_dict.insert("msToken".to_string(), ms_token);
        }
        params.insert(
            "screen_width".to_string(),
            cookie_dict.get("dy_swidth").cloned().unwrap_or_else(|| {
                params
                    .get("screen_width")
                    .cloned()
                    .unwrap_or_else(|| "1680".to_string())
            }),
        );
        params.insert(
            "screen_height".to_string(),
            cookie_dict.get("dy_sheight").cloned().unwrap_or_else(|| {
                params
                    .get("screen_height")
                    .cloned()
                    .unwrap_or_else(|| "1050".to_string())
            }),
        );
        params.insert(
            "cpu_core_num".to_string(),
            cookie_dict
                .get("device_web_cpu_core")
                .cloned()
                .unwrap_or_else(|| {
                    params
                        .get("cpu_core_num")
                        .cloned()
                        .unwrap_or_else(|| "8".to_string())
                }),
        );
        params.insert(
            "device_memory".to_string(),
            cookie_dict
                .get("device_web_memory_size")
                .cloned()
                .unwrap_or_else(|| {
                    params
                        .get("device_memory")
                        .cloned()
                        .unwrap_or_else(|| "8".to_string())
                }),
        );

        let verify_fp = cookie_dict
            .get("s_v_web_id")
            .cloned()
            .unwrap_or_else(Self::generate_verify_fp);
        params.insert("verifyFp".to_string(), verify_fp.clone());
        params.insert("fp".to_string(), verify_fp);

        if let Some(uifid) = cookie_dict.get("UIFID") {
            headers.insert("uifid".to_string(), uifid.clone());
            params.insert("uifid".to_string(), uifid.clone());
        }

        let webid = self
            .get_webid(headers)
            .await
            .unwrap_or_else(Self::generate_fake_webid);
        params.insert("webid".to_string(), webid);
    }

    fn set_param_part(params: &mut Vec<(String, String)>, key: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some((_, existing)) = params.iter_mut().find(|(name, _)| name == key) {
            *existing = value;
        } else {
            params.push((key.to_string(), value));
        }
    }

    fn get_param_part(params: &[(String, String)], key: &str) -> Option<String> {
        params
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }

    async fn enrich_request_parts(
        &self,
        params: &mut Vec<(String, String)>,
        headers: &mut HashMap<String, String>,
    ) {
        let cookie = headers
            .get("cookie")
            .or_else(|| headers.get("Cookie"))
            .cloned()
            .unwrap_or_else(|| self.config.cookie.clone());

        if cookie.is_empty() {
            return;
        }

        let mut cookie_dict = Self::cookies_to_dict(&cookie);
        let ms_token = Self::generate_ms_token();
        Self::set_param_part(params, "msToken", ms_token.clone());
        headers.insert(
            "Cookie".to_string(),
            Self::cookie_string_with_value(&cookie, "msToken", &ms_token),
        );
        cookie_dict.insert("msToken".to_string(), ms_token);

        let screen_width = cookie_dict
            .get("dy_swidth")
            .cloned()
            .or_else(|| Self::get_param_part(params, "screen_width"))
            .unwrap_or_else(|| "1680".to_string());
        Self::set_param_part(params, "screen_width", screen_width);

        let screen_height = cookie_dict
            .get("dy_sheight")
            .cloned()
            .or_else(|| Self::get_param_part(params, "screen_height"))
            .unwrap_or_else(|| "1050".to_string());
        Self::set_param_part(params, "screen_height", screen_height);

        let cpu_core_num = cookie_dict
            .get("device_web_cpu_core")
            .cloned()
            .or_else(|| Self::get_param_part(params, "cpu_core_num"))
            .unwrap_or_else(|| "8".to_string());
        Self::set_param_part(params, "cpu_core_num", cpu_core_num);

        let device_memory = cookie_dict
            .get("device_web_memory_size")
            .cloned()
            .or_else(|| Self::get_param_part(params, "device_memory"))
            .unwrap_or_else(|| "8".to_string());
        Self::set_param_part(params, "device_memory", device_memory);

        let verify_fp = cookie_dict
            .get("s_v_web_id")
            .cloned()
            .unwrap_or_else(Self::generate_verify_fp);
        Self::set_param_part(params, "verifyFp", verify_fp.clone());
        Self::set_param_part(params, "fp", verify_fp);

        if let Some(uifid) = cookie_dict.get("UIFID") {
            headers.insert("uifid".to_string(), uifid.clone());
            Self::set_param_part(params, "uifid", uifid.clone());
        }

        let webid = self
            .get_webid(headers)
            .await
            .unwrap_or_else(Self::generate_fake_webid);
        Self::set_param_part(params, "webid", webid);
    }

    async fn request_with_options<T: DeserializeOwned>(
        &self,
        url: &str,
        params: Option<HashMap<&str, String>>,
        method: &str,
        extra_headers: Option<HashMap<String, String>>,
        skip_sign: bool,
    ) -> Result<T> {
        let started_at = Instant::now();
        let mut all_params = crate::config::get_common_params();

        if let Some(p) = params {
            for (key, value) in p {
                all_params.insert(key.to_string(), value);
            }
        }

        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.extend(Self::ticket_guard_headers_from_cookie(&self.config.cookie));
        if let Some(extra) = extra_headers {
            headers.extend(extra);
        }

        self.enrich_request(&mut all_params, &mut headers).await;

        if !skip_sign {
            let params_str = serde_urlencoded::to_string(&all_params)?;
            let user_agent = headers
                .get("User-Agent")
                .map(String::as_str)
                .unwrap_or_else(|| get_user_agent());
            let a_bogus = if url.contains("reply") {
                sign::sign_reply(&params_str, user_agent)
            } else {
                sign::sign_detail(&params_str, user_agent)
            };
            all_params.insert("a_bogus".to_string(), a_bogus);
        }

        log::debug!(
            "API request started: method={} url={} skip_sign={}",
            method,
            url,
            skip_sign,
        );

        // 打印关键参数用于调试
        let params_str: String = all_params
            .iter()
            .map(|(k, v)| {
                let preview = if v.chars().count() > 20 {
                    format!("{}...", v.chars().take(20).collect::<String>())
                } else {
                    v.clone()
                };
                format!("{}={}", k, preview)
            })
            .collect::<Vec<_>>()
            .join(", ");
        log::debug!("Request params: {}", params_str);

        let mut req = match method {
            "GET" => self.client.get(url).query(&all_params),
            "POST" => self.client.post(url).form(&all_params),
            _ => return Err(anyhow!("Unsupported HTTP method: {}", method)),
        };

        for (key, value) in headers {
            req = req.header(&key, value);
        }

        let response = req.send().await.map_err(|e| {
            log::error!(
                "API request failed: method={} url={} elapsed_ms={} error={}",
                method,
                url,
                started_at.elapsed().as_millis(),
                e
            );
            e
        })?;

        if !response.status().is_success() {
            log::warn!(
                "API request returned non-success status: method={} url={} status={} elapsed_ms={}",
                method,
                url,
                response.status(),
                started_at.elapsed().as_millis()
            );
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let json = response.json::<T>().await.map_err(|e| {
            log::error!(
                "API response decode failed: method={} url={} elapsed_ms={} error={}",
                method,
                url,
                started_at.elapsed().as_millis(),
                e
            );
            e
        })?;
        log::debug!(
            "API request completed: method={} url={} elapsed_ms={}",
            method,
            url,
            started_at.elapsed().as_millis()
        );
        Ok(json)
    }

    /// 通用请求方法
    pub async fn request<T: DeserializeOwned>(
        &self,
        url: &str,
        params: Option<HashMap<&str, String>>,
        method: &str,
    ) -> Result<ApiResponse<T>> {
        self.request_with_options(url, params, method, None, false)
            .await
    }

    pub async fn request_raw_json(
        &self,
        url: &str,
        params: Option<HashMap<&str, String>>,
        method: &str,
    ) -> Result<serde_json::Value> {
        self.request_with_options(url, params, method, None, false)
            .await
    }

    pub async fn request_raw_json_with_options(
        &self,
        url: &str,
        params: Option<HashMap<&str, String>>,
        method: &str,
        extra_headers: Option<HashMap<String, String>>,
        skip_sign: bool,
    ) -> Result<serde_json::Value> {
        self.request_with_options(url, params, method, extra_headers, skip_sign)
            .await
    }

    /// 从分享文本中提取第一个可请求链接。
    fn normalize_share_url_token(value: &str) -> String {
        let trimmed = value.trim();
        let end = trimmed
            .char_indices()
            .find_map(|(index, ch)| {
                if "，。！？；、,!;".contains(ch) {
                    Some(index)
                } else {
                    None
                }
            })
            .unwrap_or(trimmed.len());

        trimmed[..end]
            .trim()
            .trim_end_matches(|ch: char| "，。！？；、,.!;".contains(ch))
            .to_string()
    }

    fn extract_share_url(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        let value = SHARE_URL_REGEX
            .find(trimmed)
            .map(|matched| matched.as_str().to_string())
            .unwrap_or_else(|| trimmed.to_string());
        let value = Self::normalize_share_url_token(&value);

        if value.is_empty() {
            None
        } else if value.starts_with("www.") {
            Some(format!("https://{}", value))
        } else {
            Some(value)
        }
    }

    /// 从 URL 提取视频 ID
    pub fn extract_aweme_id(url: &str) -> Option<String> {
        let url = url.trim();

        // 直接是 aweme_id
        if AWEME_ID_DIGIT_REGEX.is_match(url) {
            return Some(url.to_string());
        }

        // 从分享链接提取
        for re in AWEME_ID_REGEXES.iter() {
            if let Some(caps) = re.captures(url) {
                if let Some(id) = caps.get(1) {
                    return Some(id.as_str().to_string());
                }
            }
        }

        None
    }

    /// 获取视频详情
    pub async fn get_video_detail(&self, aweme_id: &str) -> Result<VideoInfo> {
        let primary_result = self.get_single_video_detail(aweme_id).await;
        let mut video_info = match primary_result {
            Ok(video_info) => video_info,
            Err(primary_error) => {
                log::warn!(
                    "single video detail request failed, trying multi detail fallback: aweme_id={} error={}",
                    aweme_id,
                    primary_error
                );
                return self
                    .get_multi_video_detail(aweme_id)
                    .await
                    .map_err(|fallback_error| {
                        anyhow!(
                            "{}; fallback multi detail failed: {}",
                            primary_error,
                            fallback_error
                        )
                    });
            }
        };

        if !Self::video_info_has_media(&video_info) {
            match self.get_multi_video_detail(aweme_id).await {
                Ok(fallback) if Self::video_info_has_media(&fallback) => {
                    log::info!(
                        "using multi detail fallback because single detail had no media: aweme_id={}",
                        aweme_id
                    );
                    video_info = fallback;
                }
                Ok(_) => {
                    log::warn!(
                        "multi detail fallback also had no media: aweme_id={}",
                        aweme_id
                    );
                }
                Err(error) => {
                    log::warn!(
                        "multi detail fallback failed after empty single detail: aweme_id={} error={}",
                        aweme_id,
                        error
                    );
                }
            }
        }

        if video_info.aweme_id.trim().is_empty() {
            video_info.aweme_id = aweme_id.to_string();
        }

        Ok(video_info)
    }

    async fn get_single_video_detail(&self, aweme_id: &str) -> Result<VideoInfo> {
        let mut params = HashMap::new();
        params.insert("aweme_id", aweme_id.to_string());
        params.insert("aid", "1128".to_string());
        params.insert("version_name", "23.5.0".to_string());
        params.insert("device_platform", "webapp".to_string());
        params.insert("os", "windows".to_string());

        let response = match self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/aweme/detail/",
                Some(params.clone()),
                "GET",
                None,
                true,
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                log::warn!(
                    "video detail unsigned request failed, retrying with signature: aweme_id={} error={}",
                    aweme_id,
                    error
                );
                self.request_raw_json_with_options(
                    "https://www.douyin.com/aweme/v1/web/aweme/detail/",
                    Some(params),
                    "GET",
                    None,
                    false,
                )
                .await?
            }
        };

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            log::warn!(
                "Douyin video detail rejected: status_code={} status_msg={} aweme_id={}",
                status_code,
                status_msg,
                aweme_id
            );
            return Err(anyhow!("API error: {}", status_msg));
        }

        let data = response
            .get("aweme_detail")
            .ok_or_else(|| anyhow!("No aweme_detail in response"))?;
        let mut video_info = self.parse_video_info(data)?;
        if video_info.aweme_id.trim().is_empty() {
            video_info.aweme_id = aweme_id.to_string();
        }

        Ok(video_info)
    }

    async fn get_multi_video_detail(&self, aweme_id: &str) -> Result<VideoInfo> {
        let normalized_aweme_id = aweme_id.trim();
        if normalized_aweme_id.is_empty() {
            return Err(anyhow!("aweme_id is empty"));
        }

        let mut params = HashMap::new();
        params.insert("aweme_ids", format!("[{}]", normalized_aweme_id));
        params.insert("request_source", "200".to_string());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/multi/aweme/detail/",
                Some(params),
                "GET",
                None,
                true,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            log::warn!(
                "Douyin multi video detail rejected: status_code={} status_msg={} aweme_id={}",
                status_code,
                status_msg,
                normalized_aweme_id
            );
            return Err(anyhow!("API error: {}", status_msg));
        }

        let data = response
            .get("aweme_details")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["aweme_id"].as_str() == Some(normalized_aweme_id))
                    .or_else(|| items.first())
            })
            .ok_or_else(|| anyhow!("No aweme_details in response"))?;

        let mut video_info = self.parse_video_info(data)?;
        if video_info.aweme_id.trim().is_empty() {
            video_info.aweme_id = normalized_aweme_id.to_string();
        }
        Ok(video_info)
    }

    fn video_info_has_media(video_info: &VideoInfo) -> bool {
        video_info
            .image_urls
            .as_ref()
            .map(|urls| urls.iter().any(|url| !url.trim().is_empty()))
            .unwrap_or(false)
            || video_info
                .live_photo_urls
                .as_ref()
                .map(|urls| urls.iter().any(|url| !url.trim().is_empty()))
                .unwrap_or(false)
            || !video_info.video.play_addr.trim().is_empty()
            || video_info
                .video
                .download_addr
                .as_ref()
                .map(|url| !url.trim().is_empty())
                .unwrap_or(false)
            || video_info
                .video
                .dash_addr
                .as_ref()
                .map(|url| !url.trim().is_empty())
                .unwrap_or(false)
    }

    /// 解析视频信息
    fn parse_video_info(&self, data: &serde_json::Value) -> Result<VideoInfo> {
        let aweme_id = data["aweme_id"].as_str().unwrap_or_default().to_string();
        let desc = data["desc"].as_str().unwrap_or_default().to_string();
        let create_time = data["create_time"].as_i64().unwrap_or(0);

        // 作者信息
        let author_data = &data["author"];
        let author = AuthorInfo {
            uid: author_data["uid"].as_str().unwrap_or_default().to_string(),
            sec_uid: author_data["sec_uid"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            nickname: author_data["nickname"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            avatar_thumb: self.get_first_url(&author_data["avatar_thumb"]["url_list"]),
            avatar_medium: self.get_first_url(&author_data["avatar_medium"]["url_list"]),
            signature: author_data["signature"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            follower_count: author_data["follower_count"].as_i64().unwrap_or(0),
            following_count: author_data["following_count"].as_i64().unwrap_or(0),
            aweme_count: author_data["aweme_count"].as_i64().unwrap_or(0),
            favoriting_count: author_data["favoriting_count"].as_i64().unwrap_or(0),
            is_follow: author_data["is_follow"].as_bool().unwrap_or(false)
                || author_data["follow_status"].as_i64().unwrap_or(0) > 0,
            follow_status: author_data["follow_status"].as_i64().unwrap_or(0) as i32,
            verify_status: author_data["verify_status"].as_i64().unwrap_or(0) as i32,
            unique_id: author_data["unique_id"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        };

        // 视频数据 - 参考 Python 版本从 bit_rate[0]["play_addr"] 获取视频 URL
        let video_data = &data["video"];

        let dash_addr = Self::select_dash_video_url(video_data);
        let audio_addr = Self::select_dash_audio_url(video_data);
        let bit_rate_play_addr = video_data["bit_rate"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|br| self.get_first_url_opt(&br["play_addr"]));
        let fallback_play_addr = self.get_first_url(&video_data["play_addr"]);
        let download_addr = self.get_first_url_opt(&video_data["download_addr"]);
        let primary_no_watermark = [
            bit_rate_play_addr.clone(),
            self.get_first_url_opt(&video_data["play_addr_h264"]),
            Some(fallback_play_addr.clone()),
            self.get_first_url_opt(&video_data["play_addr_lowbr"]),
            download_addr.clone(),
        ]
        .into_iter()
        .flatten()
        .find(|url| {
            !url.is_empty() && !looks_watermarked_media_url(url) && !is_dash_video_only_url(url)
        });
        let play_addr = primary_no_watermark
            .or(bit_rate_play_addr.filter(|url| !is_dash_video_only_url(url)))
            .or({
                if fallback_play_addr.is_empty() || is_dash_video_only_url(&fallback_play_addr) {
                    None
                } else {
                    Some(fallback_play_addr)
                }
            })
            .unwrap_or_default();

        let video = VideoData {
            preview_addr: Some(play_addr.clone()),
            play_addr: play_addr.clone(),
            dash_addr,
            audio_addr,
            play_addr_h264: self.get_first_url_opt(&video_data["play_addr_h264"]),
            play_addr_lowbr: self.get_first_url_opt(&video_data["play_addr_lowbr"]),
            download_addr: self.get_first_url_opt(&video_data["download_addr"]),
            cover: self.get_first_url(&video_data["cover"]["url_list"]),
            dynamic_cover: self.get_first_url(&video_data["dynamic_cover"]["url_list"]),
            origin_cover: self.get_first_url(&video_data["origin_cover"]["url_list"]),
            width: video_data["width"].as_i64().unwrap_or(0) as i32,
            height: video_data["height"].as_i64().unwrap_or(0) as i32,
            duration: video_data["duration"].as_i64().unwrap_or(0),
            ratio: video_data["ratio"].as_str().unwrap_or_default().to_string(),
            bit_rate: video_data["bit_rate"].as_array().map(|arr| {
                arr.iter()
                    .map(|b| BitRateInfo {
                        gear_name: b["gear_name"].as_str().unwrap_or_default().to_string(),
                        format: b["format"].as_str().unwrap_or_default().to_string(),
                        bit_rate: b["bit_rate"].as_i64().unwrap_or(0),
                        quality_type: b["quality_type"].as_i64().unwrap_or(0) as i32,
                        is_h265: b["is_h265"].as_bool().unwrap_or(false),
                        data_size: b["data_size"].as_i64().unwrap_or(0),
                        width: b["width"].as_i64().unwrap_or(0) as i32,
                        height: b["height"].as_i64().unwrap_or(0) as i32,
                        play_addr: self.get_first_url_opt(&b["play_addr"]),
                        play_addr_h264: self.get_first_url_opt(&b["play_addr_h264"]),
                    })
                    .collect()
            }),
        };

        // 统计
        let stats = &data["statistics"];
        let statistics = Statistics {
            play_count: stats["play_count"].as_i64().unwrap_or(0),
            digg_count: stats["digg_count"].as_i64().unwrap_or(0),
            comment_count: stats["comment_count"].as_i64().unwrap_or(0),
            share_count: stats["share_count"].as_i64().unwrap_or(0),
            collect_count: stats["collect_count"].as_i64().unwrap_or(0),
            forward_count: stats["forward_count"].as_i64().unwrap_or(0),
        };

        // 状态
        let status_data = &data["status"];
        let status = Status {
            is_delete: status_data["is_delete"].as_bool().unwrap_or(false),
            private_status: status_data["private_status"].as_i64().unwrap_or(0) as i32,
            review_status: status_data["review_status"].as_i64().unwrap_or(0) as i32,
            with_goods: status_data["with_goods"].as_bool().unwrap_or(false),
            is_prohibited: status_data["is_prohibited"].as_bool().unwrap_or(false),
        };

        // 判断媒体类型 - 参考 Python 版本
        // Python: 如果 images 字段存在且不为 null，就是图集(awemeType=1)
        // 否则是视频(awemeType=0)
        let images_data = data
            .get("images")
            .and_then(|v| v.as_array())
            .filter(|arr| !arr.is_empty());

        let is_image = images_data.is_some();
        let mut image_urls_list = Vec::new();
        let mut live_photo_urls_list = Vec::new();

        if let Some(images) = images_data {
            for image in images {
                if let Some(url) = image
                    .get("video")
                    .and_then(|value| value.get("play_addr"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| value.as_array())
                    .and_then(|urls| urls.first())
                    .and_then(|value| value.as_str())
                {
                    live_photo_urls_list.push(url.to_string());
                } else if let Some(url) = image
                    .get("url_list")
                    .and_then(|value| value.as_array())
                    .and_then(|urls| urls.last())
                    .and_then(|value| value.as_str())
                {
                    image_urls_list.push(url.to_string());
                }
            }
        }

        let has_live_photo = !live_photo_urls_list.is_empty();
        let has_static_image = !image_urls_list.is_empty();
        let image_urls = if image_urls_list.is_empty() {
            None
        } else {
            Some(image_urls_list)
        };
        let live_photo_urls = if live_photo_urls_list.is_empty() {
            None
        } else {
            Some(live_photo_urls_list)
        };

        // 确定媒体类型
        // 参考 Python 版本: awemeType=0 视频, awemeType=1 图集
        // 实况照片是图集的特殊形式，有视频URL
        let media_type = if has_live_photo && has_static_image {
            MediaType::Mixed
        } else if has_live_photo {
            MediaType::LivePhoto
        } else if is_image {
            MediaType::Image
        } else {
            MediaType::Video
        };

        log::debug!(
            "parse_video_info: aweme_id={} is_image={} has_live_photo={} media_type={:?}",
            aweme_id,
            is_image,
            has_live_photo,
            media_type
        );

        // 音乐信息
        let music = if data["music"].is_object() {
            let m = &data["music"];
            Some(MusicInfo {
                id: m["id"].as_str().unwrap_or_default().to_string(),
                title: m["title"].as_str().unwrap_or_default().to_string(),
                author: m["author"]
                    .as_str()
                    .or_else(|| m["owner_nickname"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                play_url: self.extract_music_play_url_value(m),
                cover_thumb: self
                    .get_first_url_opt(&m["cover_thumb"]["url_list"])
                    .or_else(|| self.get_first_url_opt(&m["cover_large"]["url_list"]))
                    .unwrap_or_default(),
                duration: m["duration"].as_i64().unwrap_or(0),
            })
        } else {
            None
        };

        // 文本额外信息
        let text_extra = data["text_extra"].as_array().map(|arr| {
            arr.iter()
                .map(|t| TextExtra {
                    text: t["text"].as_str().unwrap_or_default().to_string(),
                    r#type: t["type"].as_i64().unwrap_or(0) as i32,
                    hashtag_name: t["hashtag_name"].as_str().map(|s| s.to_string()),
                    aweme_id: t["aweme_id"].as_str().map(|s| s.to_string()),
                    sec_uid: t["sec_uid"].as_str().map(|s| s.to_string()),
                    user_id: t["user_id"].as_str().map(|s| s.to_string()),
                })
                .collect()
        });

        // 判断媒体类型
        let raw_media_type = data["raw_media_type"].as_i64().map(|v| v as i32);
        let is_liked = Self::json_boolish_any(data, &["user_digged", "is_liked", "digg_status"]);
        let is_collected = Self::json_boolish_any(
            data,
            &[
                "is_collected",
                "is_collect",
                "collect_status",
                "collect_stat",
            ],
        );

        Ok(VideoInfo {
            aweme_id,
            desc,
            create_time,
            author,
            video,
            statistics,
            status,
            image_urls,
            is_image,
            media_type,
            has_live_photo,
            is_liked,
            is_collected,
            live_photo_urls,
            music,
            raw_media_type,
            text_extra,
        })
    }

    fn get_first_url(&self, data: &serde_json::Value) -> String {
        self.get_first_url_opt(data).unwrap_or_default()
    }

    fn get_avatar_url(&self, data: &serde_json::Value, keys: &[&str]) -> String {
        keys.iter()
            .filter_map(|key| data.get(*key))
            .find_map(|value| self.get_first_url_opt(value))
            .unwrap_or_default()
    }

    fn json_boolish_any(data: &serde_json::Value, keys: &[&str]) -> bool {
        keys.iter()
            .filter_map(|key| data.get(*key))
            .any(Self::json_boolish)
    }

    fn json_boolish(value: &serde_json::Value) -> bool {
        if let Some(value) = value.as_bool() {
            return value;
        }
        if let Some(value) = value.as_i64() {
            return value > 0;
        }
        if let Some(value) = value.as_str() {
            return matches!(value.trim(), "1" | "true" | "True" | "TRUE");
        }
        false
    }

    fn get_first_url_opt(&self, data: &serde_json::Value) -> Option<String> {
        if let Some(value) = data
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }

        if let Some(arr) = data.as_array() {
            return arr.iter().find_map(|value| self.get_first_url_opt(value));
        }

        if let Some(obj) = data.as_object() {
            for key in [
                "url_list",
                "url",
                "main_url",
                "backup_url",
                "fallback_url",
                "play_addr",
                "play_url",
                "download_addr",
                "download_url",
                "display_url",
                "uri",
            ] {
                if let Some(url) = obj.get(key).and_then(|value| self.get_first_url_opt(value)) {
                    if key == "uri" && !url.starts_with("http://") && !url.starts_with("https://") {
                        continue;
                    }
                    return Some(url);
                }
            }
        }

        None
    }

    fn select_dash_video_url(video_data: &serde_json::Value) -> Option<String> {
        let bit_rates = video_data["bit_rate"].as_array()?;

        bit_rates
            .iter()
            .filter(|bit_rate| bit_rate["format"].as_str() == Some("dash"))
            .filter(|bit_rate| !bit_rate["is_h265"].as_bool().unwrap_or(false))
            .find_map(|bit_rate| {
                let urls = bit_rate["play_addr"]["url_list"].as_array()?;
                urls.iter()
                    .filter_map(|value| value.as_str().map(str::trim))
                    .find(|url| !url.is_empty() && url.contains("media-video-avc1"))
                    .or_else(|| {
                        urls.iter()
                            .filter_map(|value| value.as_str().map(str::trim))
                            .find(|url| !url.is_empty())
                    })
                    .map(str::to_string)
            })
    }

    fn select_dash_audio_url(video_data: &serde_json::Value) -> Option<String> {
        let audio_rates = video_data["bit_rate_audio"].as_array()?;

        for audio_rate in audio_rates {
            let audio_meta = &audio_rate["audio_meta"];
            if let Some(url) = Self::first_media_url_value(&audio_meta["url_list"]) {
                return Some(url);
            }
            if let Some(url) = Self::first_media_url_value(audio_meta) {
                return Some(url);
            }
        }

        None
    }

    fn first_media_url_value(data: &serde_json::Value) -> Option<String> {
        if let Some(value) = data
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }

        if let Some(values) = data.as_array() {
            return values.iter().find_map(Self::first_media_url_value);
        }

        if let Some(object) = data.as_object() {
            for key in [
                "main_url",
                "backup_url",
                "fallback_url",
                "url_list",
                "url",
                "play_url",
                "download_url",
                "uri",
            ] {
                if let Some(url) = object.get(key).and_then(Self::first_media_url_value) {
                    if key == "uri" && !url.starts_with("http://") && !url.starts_with("https://") {
                        continue;
                    }
                    return Some(url);
                }
            }
        }

        None
    }

    fn get_last_url_opt(&self, data: &serde_json::Value) -> Option<String> {
        data.as_array()
            .and_then(|arr| arr.last())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn extract_music_play_url_value(&self, music: &serde_json::Value) -> Option<String> {
        if let Some(play_url) = music.get("play_url") {
            if play_url.is_object() {
                if let Some(url) = self.get_first_url_opt(&play_url["url_list"]) {
                    if !url.is_empty() {
                        return Some(url);
                    }
                }
                if let Some(uri) = play_url.get("uri").and_then(|value| value.as_str()) {
                    if uri.starts_with("http") {
                        return Some(uri.to_string());
                    }
                }
            } else if let Some(url) = play_url.as_str() {
                if url.starts_with("http") {
                    return Some(url.to_string());
                }
            }
        }

        if let Some(music_file) = music.get("music_file") {
            if music_file.is_object() {
                if let Some(url) = self.get_first_url_opt(&music_file["url_list"]) {
                    if !url.is_empty() {
                        return Some(url);
                    }
                }
            } else if let Some(url) = music_file.as_str() {
                if url.starts_with("http") {
                    return Some(url.to_string());
                }
            }
        }

        for key in ["src_url", "mp3_url"] {
            if let Some(url) = music.get(key).and_then(|value| value.as_str()) {
                if url.starts_with("http") {
                    return Some(url.to_string());
                }
            }
        }

        None
    }

    fn extract_liked_media_info(
        &self,
        post: &serde_json::Value,
    ) -> (String, Vec<LikedVideoMediaUrl>) {
        let mut urls = Vec::new();
        let mut media_type = "unknown".to_string();

        if let Some(images) = post.get("images").and_then(|value| value.as_array()) {
            let mut has_live = false;
            let mut has_image = false;

            for image in images {
                if let Some(video_urls) = image
                    .get("video")
                    .and_then(|value| value.get("play_addr"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| value.as_array())
                {
                    has_live = true;
                    if let Some(url) = video_urls.first().and_then(|value| value.as_str()) {
                        urls.push(LikedVideoMediaUrl {
                            r#type: "live_photo".to_string(),
                            url: url.to_string(),
                        });
                    }
                } else if let Some(image_urls) =
                    image.get("url_list").and_then(|value| value.as_array())
                {
                    if let Some(url) = image_urls.last().and_then(|value| value.as_str()) {
                        has_image = true;
                        urls.push(LikedVideoMediaUrl {
                            r#type: "image".to_string(),
                            url: url.to_string(),
                        });
                    }
                }
            }

            media_type = if has_live && has_image {
                "mixed".to_string()
            } else if has_live {
                "live_photo".to_string()
            } else if has_image {
                "image".to_string()
            } else {
                "unknown".to_string()
            };
        } else if let Some(video_urls) = post
            .get("video")
            .and_then(|value| value.get("play_addr"))
            .and_then(|value| value.get("url_list"))
            .and_then(|value| value.as_array())
        {
            if let Some(url) = video_urls.first().and_then(|value| value.as_str()) {
                let clean_url = clean_video_media_url(url);
                if !clean_url.is_empty() && !is_dash_video_only_url(&clean_url) {
                    media_type = "video".to_string();
                    urls.push(LikedVideoMediaUrl {
                        r#type: "video".to_string(),
                        url: clean_url,
                    });
                }
            }
        }

        (media_type, urls)
    }

    fn extract_liked_bgm_url(&self, post: &serde_json::Value) -> Option<String> {
        let music = post.get("music")?;
        let mut bgm_url = self.extract_music_play_url_value(music);

        if bgm_url
            .as_ref()
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            let h5_url = music
                .get("h5_url")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let web_url = music
                .get("web_url")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            bgm_url = Some(if !h5_url.is_empty() {
                h5_url.to_string()
            } else {
                web_url.to_string()
            });
        }

        if bgm_url
            .as_ref()
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            if let Some(music_file) = music.get("music_file") {
                if music_file.is_object() {
                    bgm_url = self.get_first_url_opt(&music_file["url_list"]);
                } else if let Some(url) = music_file.as_str() {
                    bgm_url = Some(url.to_string());
                }
            }
        }

        bgm_url
    }

    fn build_liked_video_item(
        &self,
        post: &serde_json::Value,
        default_liked: bool,
        default_collected: bool,
    ) -> Option<LikedVideoItem> {
        let aweme_id = post.get("aweme_id")?.as_str()?.to_string();
        let (media_type, media_urls) = self.extract_liked_media_info(post);
        let video_data = &post["video"];
        let dash_addr = Self::select_dash_video_url(video_data);
        let audio_addr = Self::select_dash_audio_url(video_data);
        let raw_play_addr = self.get_first_url(&video_data["play_addr"]);
        let selected_play_addr = clean_video_media_url(&raw_play_addr);
        let selected_play_addr = if is_dash_video_only_url(&selected_play_addr) {
            String::new()
        } else {
            selected_play_addr
        };

        let cover_url = post
            .get("video")
            .and_then(|value| value.get("cover"))
            .and_then(|value| value.get("url_list"))
            .and_then(|value| self.get_first_url_opt(value))
            .or_else(|| {
                post.get("images")
                    .and_then(|value| value.as_array())
                    .and_then(|images| images.first())
                    .and_then(|image| image.get("url_list"))
                    .and_then(|value| self.get_last_url_opt(value))
            })
            .unwrap_or_default();
        let fallback_media_url = media_urls
            .first()
            .map(|media| media.url.clone())
            .unwrap_or_default();
        let preview_addr = if selected_play_addr.is_empty() {
            fallback_media_url.clone()
        } else {
            selected_play_addr.clone()
        };
        let duration = video_data["duration"].as_i64().unwrap_or(0);
        let bit_rate = video_data["bit_rate"].as_array().and_then(|arr| {
            let items = arr
                .iter()
                .filter_map(|b| {
                    let play_addr = self.get_first_url_opt(&b["play_addr"]);
                    let play_addr_h264 = self.get_first_url_opt(&b["play_addr_h264"]);
                    if play_addr.is_none() && play_addr_h264.is_none() {
                        return None;
                    }
                    Some(BitRateInfo {
                        gear_name: b["gear_name"].as_str().unwrap_or_default().to_string(),
                        format: b["format"].as_str().unwrap_or_default().to_string(),
                        bit_rate: b["bit_rate"].as_i64().unwrap_or(0),
                        quality_type: b["quality_type"].as_i64().unwrap_or(0) as i32,
                        is_h265: b["is_h265"].as_bool().unwrap_or(false),
                        data_size: b["data_size"].as_i64().unwrap_or(0),
                        width: b["width"].as_i64().unwrap_or(0) as i32,
                        height: b["height"].as_i64().unwrap_or(0) as i32,
                        play_addr,
                        play_addr_h264,
                    })
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                None
            } else {
                Some(items)
            }
        });

        Some(LikedVideoItem {
            aweme_id,
            desc: post["desc"].as_str().unwrap_or_default().to_string(),
            create_time: post["create_time"].as_i64().unwrap_or(0),
            digg_count: post["statistics"]["digg_count"].as_i64().unwrap_or(0),
            comment_count: post["statistics"]["comment_count"].as_i64().unwrap_or(0),
            share_count: post["statistics"]["share_count"].as_i64().unwrap_or(0),
            cover_url: cover_url.clone(),
            duration,
            media_type: media_type.clone(),
            raw_media_type: media_type,
            media_urls,
            bgm_url: self.extract_liked_bgm_url(post),
            is_liked: Self::json_boolish_any(post, &["user_digged", "is_liked", "digg_status"])
                || default_liked,
            is_collected: Self::json_boolish_any(
                post,
                &[
                    "is_collected",
                    "is_collect",
                    "collect_status",
                    "collect_stat",
                ],
            ) || default_collected,
            statistics: Statistics {
                digg_count: post["statistics"]["digg_count"].as_i64().unwrap_or(0),
                comment_count: post["statistics"]["comment_count"].as_i64().unwrap_or(0),
                share_count: post["statistics"]["share_count"].as_i64().unwrap_or(0),
                play_count: post["statistics"]["play_count"].as_i64().unwrap_or(0),
                collect_count: post["statistics"]["collect_count"].as_i64().unwrap_or(0),
                ..Default::default()
            },
            video: VideoData {
                preview_addr: if preview_addr.is_empty() {
                    None
                } else {
                    Some(preview_addr.clone())
                },
                play_addr: if selected_play_addr.is_empty() {
                    fallback_media_url.clone()
                } else {
                    selected_play_addr
                },
                dash_addr,
                audio_addr,
                play_addr_h264: self.get_first_url_opt(&video_data["play_addr_h264"]),
                play_addr_lowbr: self.get_first_url_opt(&video_data["play_addr_lowbr"]),
                download_addr: self.get_first_url_opt(&video_data["download_addr"]),
                cover: cover_url.clone(),
                dynamic_cover: self
                    .get_first_url_opt(&video_data["dynamic_cover"]["url_list"])
                    .unwrap_or_else(|| cover_url.clone()),
                origin_cover: self
                    .get_first_url_opt(&video_data["origin_cover"]["url_list"])
                    .unwrap_or_else(|| cover_url.clone()),
                width: video_data["width"].as_i64().unwrap_or(0) as i32,
                height: video_data["height"].as_i64().unwrap_or(0) as i32,
                duration,
                ratio: video_data["ratio"].as_str().unwrap_or_default().to_string(),
                bit_rate,
            },
            author: LikedVideoAuthor {
                nickname: post["author"]["nickname"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                sec_uid: post["author"]["sec_uid"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                avatar_thumb: post
                    .get("author")
                    .and_then(|value| value.get("avatar_thumb"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| self.get_first_url_opt(value))
                    .unwrap_or_default(),
            },
        })
    }

    async fn request_liked_videos_response(
        &self,
        sec_uid: &str,
        max_cursor: i64,
        count: u32,
    ) -> Result<serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("max_cursor", max_cursor.to_string());
        params.insert("count", count.to_string());
        if !sec_uid.is_empty() {
            params.insert("sec_user_id", sec_uid.to_string());
        }

        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/aweme/favorite/",
                Some(params),
                "GET",
                Some(headers),
                true,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        Ok(response)
    }

    pub async fn get_liked_videos_python_style(
        &self,
        sec_uid: &str,
        max_cursor: i64,
        count: u32,
    ) -> Result<(Vec<LikedVideoItem>, i64, bool)> {
        let response = self
            .request_liked_videos_response(sec_uid, max_cursor, count)
            .await?;

        let cursor = response["max_cursor"]
            .as_i64()
            .or_else(|| response["cursor"].as_i64())
            .or_else(|| response["min_cursor"].as_i64())
            .unwrap_or(0);
        let has_more = response["has_more"].as_i64().unwrap_or(0) == 1
            || response["has_more"].as_bool().unwrap_or(false);
        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.build_liked_video_item(post, true, false))
                    .collect()
            })
            .unwrap_or_default();

        Ok((videos, cursor, has_more))
    }

    async fn request_collected_videos_response(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<serde_json::Value> {
        let url = "https://www.douyin.com/aweme/v1/web/aweme/listcollection/";
        let mut query_params = crate::config::get_common_params();
        query_params.insert("count".to_string(), count.to_string());
        query_params.insert("cursor".to_string(), cursor.to_string());

        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/user/self?from_tab_name=main&showTab=favorite_collection"
                .to_string(),
        );
        headers.insert("Origin".to_string(), "https://www.douyin.com".to_string());
        headers.insert(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
        );

        self.enrich_request(&mut query_params, &mut headers).await;

        let params_str = serde_urlencoded::to_string(&query_params)?;
        let user_agent = headers
            .get("User-Agent")
            .map(String::as_str)
            .unwrap_or_else(|| get_user_agent());
        query_params.insert(
            "a_bogus".to_string(),
            sign::sign_detail(&params_str, user_agent),
        );

        let mut body_params = HashMap::new();
        body_params.insert("count", count.to_string());
        body_params.insert("cursor", cursor.to_string());

        let mut req = self
            .client
            .post(url)
            .query(&query_params)
            .form(&body_params);
        for (key, value) in &headers {
            req = req.header(key, value);
        }

        let response = req
            .send()
            .await
            .map_err(|error| anyhow!("HTTP request failed: {}", error))?;
        if !response.status().is_success() {
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let json = response.json::<serde_json::Value>().await?;
        Self::ensure_status_ok(&json)?;
        Ok(json)
    }

    pub async fn get_collected_videos_python_style(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<LikedVideoItem>, i64, bool)> {
        let response = self
            .request_collected_videos_response(cursor, count)
            .await?;

        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.build_liked_video_item(post, false, true))
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            videos,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    /// 获取收藏视频列表（返回 VideoInfo，用于批量下载）
    pub async fn get_collected_videos(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let response = self
            .request_collected_videos_response(cursor, count)
            .await?;

        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.parse_video_info(post).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            videos,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    /// 获取收藏合集列表
    pub async fn get_collected_mixes(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<CollectionMixItem>, i64, bool)> {
        let mut params = HashMap::new();
        params.insert("count", count.to_string());
        params.insert("cursor", cursor.to_string());

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/user/self?from_tab_name=main&showTab=favorite_collection"
                .to_string(),
        );

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/mix/listcollection/",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;
        Self::ensure_status_ok(&response)?;

        let mixes = response["mix_infos"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| self.build_collection_mix_item(item))
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            mixes,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    fn build_collection_mix_item(&self, item: &serde_json::Value) -> Option<CollectionMixItem> {
        let mix_id = item["mix_id"].as_str().unwrap_or_default().to_string();
        if mix_id.is_empty() {
            return None;
        }

        let author = item.get("author");
        let statis = item.get("statis");

        Some(CollectionMixItem {
            mix_id,
            mix_name: item["mix_name"].as_str().unwrap_or_default().to_string(),
            desc: item["desc"].as_str().unwrap_or_default().to_string(),
            cover_url: item
                .get("cover_url")
                .and_then(|value| value.get("url_list"))
                .and_then(|value| self.get_first_url_opt(value))
                .unwrap_or_default(),
            author: CollectionMixAuthor {
                nickname: author
                    .and_then(|value| value["nickname"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                sec_uid: author
                    .and_then(|value| value["sec_uid"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                avatar_thumb: author
                    .and_then(|value| value.get("avatar_thumb"))
                    .and_then(|value| value.get("url_list"))
                    .and_then(|value| self.get_first_url_opt(value))
                    .unwrap_or_default(),
            },
            statis: CollectionMixStats {
                collect_vv: statis
                    .and_then(|value| value["collect_vv"].as_i64())
                    .unwrap_or(0),
                play_vv: statis
                    .and_then(|value| value["play_vv"].as_i64())
                    .unwrap_or(0),
                updated_to_episode: statis
                    .and_then(|value| value["updated_to_episode"].as_i64())
                    .unwrap_or(0),
            },
            create_time: item["create_time"].as_i64().unwrap_or(0),
            update_time: item["update_time"].as_i64().unwrap_or(0),
            mix_type: item["mix_type"].as_i64().unwrap_or(0) as i32,
        })
    }

    /// 获取合集内的视频列表
    pub async fn get_mix_videos(
        &self,
        series_id: &str,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let mut params = HashMap::new();
        params.insert("series_id", series_id.to_string());
        params.insert("pull_type", "2".to_string());
        params.insert("cursor", cursor.to_string());
        params.insert("count", count.to_string());

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/user/self?from_tab_name=main&showTab=favorite_collection"
                .to_string(),
        );

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/series/aweme/",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;
        Self::ensure_status_ok(&response)?;

        let videos = response["aweme_list"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|post| self.parse_video_info(post).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok((
            videos,
            Self::json_cursor(&response),
            Self::json_has_more(&response),
        ))
    }

    /// 获取无水印视频 URL
    pub fn get_no_watermark_url(video: &VideoInfo) -> Option<String> {
        for url in [
            video.video.play_addr_h264.as_deref(),
            Some(video.video.play_addr.as_str()),
            video.video.download_addr.as_deref(),
            video.video.play_addr_lowbr.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let clean_url = url
                .trim()
                .replace("watermark=1", "watermark=0")
                .replace("playwm", "play");
            let normalized = clean_url.to_ascii_lowercase();
            if !clean_url.is_empty()
                && !normalized.contains("playwm")
                && !normalized.contains("watermark=1")
                && !normalized.contains("media-video")
                && !normalized.contains("media_video")
            {
                return Some(clean_url);
            }
        }
        None
    }

    fn json_count_value(value: &serde_json::Value, keys: &[&str]) -> i64 {
        for key in keys {
            let item = &value[*key];
            if let Some(number) = item.as_i64() {
                return number;
            }
            if let Some(text) = item.as_str() {
                let normalized = text.trim().replace(',', "");
                if let Ok(number) = normalized.parse::<i64>() {
                    return number;
                }
            }
        }
        0
    }

    fn json_has_more(value: &serde_json::Value) -> bool {
        value["has_more"].as_i64().unwrap_or(0) == 1
            || value["has_more"].as_bool().unwrap_or(false)
            || matches!(value["has_more"].as_str(), Some("1" | "true" | "True"))
    }

    fn json_cursor(value: &serde_json::Value) -> i64 {
        value["cursor"]
            .as_i64()
            .or_else(|| value["max_cursor"].as_i64())
            .or_else(|| value["min_cursor"].as_i64())
            .unwrap_or(0)
    }

    fn ensure_status_ok(value: &serde_json::Value) -> Result<()> {
        let status_code = value["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = value["status_msg"].as_str().unwrap_or("unknown error");
            log::warn!(
                "Douyin API status rejected: status_code={} status_msg={}",
                status_code,
                status_msg
            );
            return Err(anyhow!("API error: {} (code={})", status_msg, status_code));
        }
        Ok(())
    }

    /// 搜索用户
    pub async fn search_user(&self, keyword: &str) -> Result<SearchUserResult> {
        let keyword = keyword.trim();

        if keyword.contains("https") {
            let user_id = keyword
                .split('/')
                .next_back()
                .unwrap_or_default()
                .split('?')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();

            if user_id.is_empty() {
                return Ok(SearchUserResult::NotFound);
            }

            return Ok(SearchUserResult::Single(Box::new(UserInfo {
                sec_uid: user_id,
                ..Default::default()
            })));
        }

        let precise_search =
            keyword.starts_with('@') || keyword.chars().any(|ch| ch.is_ascii_digit());
        let mut params = HashMap::new();
        params.insert("keyword", keyword.to_string());
        params.insert("search_channel", "aweme_user_web".to_string());
        params.insert("search_source", "normal_search".to_string());
        params.insert("query_correct_type", "1".to_string());
        params.insert("is_filter_search", "0".to_string());
        params.insert("from_group_id", "".to_string());
        params.insert("offset", "0".to_string());
        params.insert("count", if precise_search { "1" } else { "10" }.to_string());
        params.insert(
            "pc_search_top_1_params",
            "{\"enable_ai_search_top_1\":1}".to_string(),
        );

        let encoded_keyword: String =
            url::form_urlencoded::byte_serialize(keyword.as_bytes()).collect();
        let verify_url = format!(
            "https://www.douyin.com/jingxuan/search/{}?type=user",
            encoded_keyword
        );
        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), verify_url.clone());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/discover/search/",
                Some(params),
                "GET",
                Some(headers),
                true,
            )
            .await?;

        let need_verify = response["search_nil_info"]["search_nil_type"]
            .as_str()
            .map(|value| value == "verify_check")
            .unwrap_or(false)
            && response["user_list"]
                .as_array()
                .map(|items| items.is_empty())
                .unwrap_or(true);
        if need_verify {
            return Ok(SearchUserResult::NeedVerify { verify_url });
        }

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let users: Vec<UserInfo> = response["user_list"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let user = if item["user_info"].is_object() {
                            &item["user_info"]
                        } else {
                            item
                        };
                        Some(UserInfo {
                            uid: user["uid"].as_str()?.to_string(),
                            nickname: user["nickname"].as_str()?.to_string(),
                            avatar_thumb: self.get_first_url(&user["avatar_thumb"]["url_list"]),
                            avatar_medium: self.get_first_url(&user["avatar_medium"]["url_list"]),
                            avatar_larger: self.get_first_url(&user["avatar_larger"]["url_list"]),
                            signature: user["signature"].as_str().unwrap_or_default().to_string(),
                            follower_count: user["follower_count"].as_i64().unwrap_or(0),
                            following_count: user["following_count"].as_i64().unwrap_or(0),
                            total_favorited: user["total_favorited"].as_i64().unwrap_or(0),
                            aweme_count: Self::json_count_value(
                                user,
                                &[
                                    "aweme_count",
                                    "aweme_count_str",
                                    "aweme_count_text",
                                    "work_count",
                                ],
                            ),
                            favoriting_count: user["favoriting_count"].as_i64().unwrap_or(0),
                            is_follow: user["is_follow"].as_bool().unwrap_or(false)
                                || user["follow_status"].as_i64().unwrap_or(0) > 0,
                            follow_status: user["follow_status"].as_i64().unwrap_or(0) as i32,
                            sec_uid: user["sec_uid"].as_str().unwrap_or_default().to_string(),
                            unique_id: user["unique_id"].as_str().unwrap_or_default().to_string(),
                            verify_status: user["verify_status"].as_i64().unwrap_or(0) as i32,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if users.is_empty() {
            return Ok(SearchUserResult::NotFound);
        }

        if precise_search {
            Ok(SearchUserResult::Single(Box::new(
                users.into_iter().next().unwrap_or_default(),
            )))
        } else {
            Ok(SearchUserResult::Multiple(users))
        }
    }

    /// 获取用户详情
    pub async fn get_user_detail(&self, sec_uid: &str) -> Result<UserDetail> {
        let mut params = HashMap::new();
        params.insert("sec_user_id", sec_uid.to_string());
        params.insert("personal_center_strategy", "1".to_string());
        params.insert("source", "channel_pc_web".to_string());

        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/user/profile/other/",
                Some(params),
                "GET",
                Some(headers),
                true,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let user_data = &response["user"];

        let info = UserInfo {
            uid: user_data["uid"].as_str().unwrap_or_default().to_string(),
            nickname: user_data["nickname"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            avatar_thumb: self.get_first_url(&user_data["avatar_thumb"]["url_list"]),
            avatar_medium: self.get_first_url(&user_data["avatar_medium"]["url_list"]),
            avatar_larger: self.get_first_url(&user_data["avatar_larger"]["url_list"]),
            signature: user_data["signature"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            follower_count: user_data["follower_count"].as_i64().unwrap_or(0),
            following_count: user_data["following_count"].as_i64().unwrap_or(0),
            total_favorited: user_data["total_favorited"].as_i64().unwrap_or(0),
            aweme_count: user_data["aweme_count"].as_i64().unwrap_or(0),
            favoriting_count: user_data["favoriting_count"].as_i64().unwrap_or(0),
            is_follow: user_data["is_follow"].as_bool().unwrap_or(false)
                || user_data["follow_status"].as_i64().unwrap_or(0) > 0,
            follow_status: user_data["follow_status"].as_i64().unwrap_or(0) as i32,
            sec_uid: user_data["sec_uid"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            unique_id: user_data["unique_id"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            verify_status: user_data["verify_status"].as_i64().unwrap_or(0) as i32,
        };

        Ok(UserDetail {
            info,
            is_favorite: user_data["is_favorite"].as_bool().unwrap_or(false),
            follow_status: user_data["follow_status"].as_i64().unwrap_or(0) as i32,
            story_count: user_data["story_count"].as_i64().unwrap_or(0),
            friend_status: user_data["friend_status"].as_i64().unwrap_or(0) as i32,
        })
    }

    /// 获取用户发布的视频列表
    pub async fn get_user_videos(
        &self,
        sec_uid: &str,
        max_cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let mut params = HashMap::new();
        params.insert("publish_video_strategy_type", "2".to_string());
        params.insert("sec_user_id", sec_uid.to_string());
        params.insert("max_cursor", max_cursor.to_string());
        params.insert("locate_query", "false".to_string());
        params.insert("show_live_replay_strategy", "1".to_string());
        params.insert("need_time_list", "0".to_string());
        params.insert("time_list_query", "0".to_string());
        params.insert("whale_cut_token", "".to_string());
        params.insert("count", count.to_string());

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/aweme/post/",
                Some(params),
                "GET",
                None,
                true,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let aweme_list = response["aweme_list"].as_array();
        let has_more = response["has_more"].as_i64().unwrap_or(0) == 1
            || response["has_more"].as_bool().unwrap_or(false);
        let cursor = response["max_cursor"].as_i64().unwrap_or(0);

        let videos = if let Some(list) = aweme_list {
            list.iter()
                .filter_map(|v| self.parse_video_info(v).ok())
                .filter(is_valid_recommended_video)
                .collect()
        } else {
            vec![]
        };

        Ok((videos, cursor, has_more))
    }

    /// 获取点赞视频列表
    pub async fn get_liked_videos(
        &self,
        sec_uid: &str,
        max_cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let response = self
            .request_liked_videos_response(sec_uid, max_cursor, count)
            .await?;

        let aweme_list = response["aweme_list"].as_array();
        let has_more = response["has_more"].as_i64().unwrap_or(0) == 1
            || response["has_more"].as_bool().unwrap_or(false);
        let cursor = response["max_cursor"].as_i64().unwrap_or(0);

        let videos = if let Some(list) = aweme_list {
            list.iter()
                .filter_map(|v| self.parse_video_info(v).ok())
                .collect()
        } else {
            vec![]
        };

        Ok((videos, cursor, has_more))
    }

    /// 获取推荐视频
    pub async fn get_recommended_feed(
        &self,
        cursor: i64,
        count: u32,
        feed_type: &str,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        if normalize_recommended_feed_type(feed_type) == "recommended" {
            return self.get_home_recommended_feed(cursor, count).await;
        }

        let mut params = HashMap::new();
        params.insert("module_id", "3003101".to_string());
        params.insert("count", count.to_string());
        params.insert("pull_type", "0".to_string());
        params.insert("refresh_index", "1".to_string());
        params.insert("refer_type", "10".to_string());
        params.insert("filterGids", "".to_string());
        params.insert("presented_ids", "".to_string());
        params.insert("refer_id", "".to_string());
        params.insert("tag_id", "".to_string());
        params.insert("use_lite_type", "2".to_string());
        params.insert("Seo-Flag", "0".to_string());
        params.insert("pre_log_id", "".to_string());
        params.insert("pre_item_ids", "".to_string());
        params.insert("pre_room_ids", "".to_string());
        params.insert("pre_item_from", "sati".to_string());
        params.insert("xigua_user", "0".to_string());
        params.insert(
            "awemePcRecRawData",
            "{\"is_xigua_user\":0,\"danmaku_switch_status\":0,\"is_client\":false}".to_string(),
        );
        if cursor > 0 {
            params.insert("cursor", cursor.to_string());
        }

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/?recommend=1".to_string(),
        );

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v2/web/module/feed/",
                Some(params),
                "POST",
                Some(headers),
                false, // 需要签名
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let aweme_list = response["aweme_list"].as_array();
        // has_more 可能是布尔值或整数
        let has_more = response["has_more"]
            .as_bool()
            .or_else(|| response["has_more"].as_i64().map(|v| v == 1))
            .unwrap_or(false);
        let next_cursor = response["cursor"]
            .as_i64()
            .or_else(|| response["max_cursor"].as_i64())
            .or_else(|| response["min_cursor"].as_i64())
            .unwrap_or_else(|| if has_more { cursor + 1 } else { cursor });

        let videos = if let Some(list) = aweme_list {
            list.iter()
                .filter_map(|v| self.parse_video_info(v).ok())
                .collect()
        } else {
            vec![]
        };

        Ok((videos, next_cursor, has_more))
    }

    async fn get_home_recommended_feed(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let refresh_index = std::cmp::max(1, cursor + 1);
        let raw_data = serde_json::json!({
            "is_client": false,
            "ff_danmaku_status": 1,
            "danmaku_switch_status": 0,
            "is_dash_user": 1,
            "related_recommend": 1,
            "is_xigua_user": 0,
        })
        .to_string();

        let mut params = HashMap::new();
        params.insert("filterGids", String::new());
        params.insert("tag_id", String::new());
        params.insert("live_insert_type", String::new());
        params.insert("count", count.to_string());
        params.insert("refresh_index", refresh_index.to_string());
        params.insert("video_type_select", "1".to_string());
        params.insert("aweme_pc_rec_raw_data", raw_data);
        params.insert("globalwid", String::new());
        params.insert("pull_type", if cursor <= 0 { "0" } else { "2" }.to_string());
        params.insert("min_window", "0".to_string());
        params.insert("free_right", "0".to_string());
        params.insert("view_count", cursor.max(0).to_string());
        params.insert("plug_block", "0".to_string());
        params.insert("ug_source", String::new());
        params.insert("creative_id", String::new());
        params.insert("webcast_sdk_version", "170400".to_string());
        params.insert("webcast_version_code", "170400".to_string());

        let headers = HashMap::from([(
            "Referer".to_string(),
            "https://www.douyin.com/?recommend=1".to_string(),
        )]);

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/tab/feed/",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let has_more = response["has_more"]
            .as_bool()
            .or_else(|| response["has_more"].as_i64().map(|v| v == 1))
            .unwrap_or(false);
        let next_cursor = response["cursor"]
            .as_i64()
            .or_else(|| response["max_cursor"].as_i64())
            .or_else(|| response["min_cursor"].as_i64())
            .unwrap_or_else(|| if has_more { refresh_index } else { cursor });

        let videos = if let Some(list) = response["aweme_list"].as_array() {
            // 预解析 fallback 与 aweme_id，避免在并发任务里持有 list 借用。
            // tab/feed 返回的视频字段不完整（缺少可播放地址），需要逐个请求 detail 刷新。
            // 用 buffered(4) 并发请求，保持原列表顺序，与 Python 版行为一致。
            let prepared: Vec<(String, Option<VideoInfo>)> = list
                .iter()
                .map(|value| {
                    let fallback = self.parse_video_info(value).ok();
                    let aweme_id = value["aweme_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .or_else(|| fallback.as_ref().map(|video| video.aweme_id.clone()))
                        .unwrap_or_default();
                    (aweme_id, fallback)
                })
                .collect();

            use futures::stream::{self, StreamExt};
            const HYDRATE_CONCURRENCY: usize = 4;
            let aweme_ids: Vec<String> = prepared
                .iter()
                .map(|(aweme_id, _)| aweme_id.clone())
                .collect();
            let hydrated: Vec<Option<VideoInfo>> = stream::iter(aweme_ids)
                .map(|aweme_id| async move {
                    if aweme_id.is_empty() {
                        return None;
                    }
                    match self.get_video_detail(&aweme_id).await {
                        Ok(detail) if is_valid_recommended_video(&detail) => Some(detail),
                        Ok(_) => {
                            log::warn!(
                                "home recommended detail had no playable media: aweme_id={}",
                                aweme_id
                            );
                            None
                        }
                        Err(error) => {
                            log::warn!(
                                "home recommended detail hydration failed: aweme_id={} error={}",
                                aweme_id,
                                error
                            );
                            None
                        }
                    }
                })
                .buffered(HYDRATE_CONCURRENCY)
                .collect()
                .await;

            let mut videos = Vec::new();
            let mut seen_ids = HashSet::new();
            let mut hydrated_count = 0usize;
            for ((_, fallback), maybe_detail) in prepared.into_iter().zip(hydrated) {
                if maybe_detail.is_some() {
                    hydrated_count += 1;
                }
                let video = maybe_detail.or(fallback.filter(is_valid_recommended_video));
                let Some(video) = video else { continue };
                if !video.aweme_id.trim().is_empty() && !seen_ids.insert(video.aweme_id.clone()) {
                    continue;
                }
                videos.push(video);
            }

            if hydrated_count > 0 {
                log::debug!(
                    "home recommended detail hydration completed: {}/{}",
                    hydrated_count,
                    videos.len()
                );
            }
            videos
        } else {
            Vec::new()
        };

        Ok((videos, next_cursor, has_more))
    }

    /// 获取评论列表
    pub async fn get_comments(
        &self,
        aweme_id: &str,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<CommentInfo>, i64, bool, i64)> {
        let mut params = HashMap::new();
        params.insert("aweme_id", aweme_id.to_string());
        params.insert("cursor", cursor.to_string());
        params.insert("count", count.to_string());
        params.insert("pc_img_format", "webp".to_string());
        params.insert("item_type", "0".to_string());
        params.insert("insert_ids", String::new());
        params.insert("whale_cut_token", String::new());
        params.insert("cut_version", "1".to_string());
        params.insert("rcFT", String::new());

        let extra_headers = HashMap::from([
            ("Origin".to_string(), "https://www.douyin.com".to_string()),
            (
                "Referer".to_string(),
                format!("https://www.douyin.com/video/{}", aweme_id),
            ),
            ("sec-fetch-site".to_string(), "same-site".to_string()),
        ]);

        let data = self
            .request_raw_json_with_options(
                "https://www-hj.douyin.com/aweme/v1/web/comment/list/",
                Some(params),
                "GET",
                Some(extra_headers),
                false,
            )
            .await?;

        if data["status_code"].as_i64().unwrap_or(0) != 0 {
            return Err(anyhow!(
                "API error: {}",
                data["status_msg"]
                    .as_str()
                    .or_else(|| data["message"].as_str())
                    .unwrap_or("获取评论失败")
            ));
        }

        let comments_data = data["comments"].as_array();
        let has_more = Self::json_has_more(&data);
        let cursor = data["cursor"].as_i64().unwrap_or(0);
        let total = data["total"].as_i64().unwrap_or(0);

        let comments = if let Some(list) = comments_data {
            list.iter().filter_map(|c| self.parse_comment(c)).collect()
        } else {
            vec![]
        };

        Ok((comments, cursor, has_more, total))
    }

    /// 获取评论的二级回复列表
    pub async fn get_comment_replies(
        &self,
        aweme_id: &str,
        comment_id: &str,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<CommentInfo>, i64, bool, i64)> {
        let mut params = HashMap::new();
        params.insert("item_id", aweme_id.to_string());
        params.insert("aweme_id", aweme_id.to_string());
        params.insert("comment_id", comment_id.to_string());
        params.insert("cursor", cursor.to_string());
        params.insert("count", count.to_string());
        params.insert("pc_img_format", "webp".to_string());
        params.insert("item_type", "0".to_string());

        let extra_headers = HashMap::from([
            ("Origin".to_string(), "https://www.douyin.com".to_string()),
            (
                "Referer".to_string(),
                format!("https://www.douyin.com/video/{}", aweme_id),
            ),
            ("sec-fetch-site".to_string(), "same-site".to_string()),
        ]);

        let data = self
            .request_raw_json_with_options(
                "https://www-hj.douyin.com/aweme/v1/web/comment/list/reply/",
                Some(params),
                "GET",
                Some(extra_headers),
                false,
            )
            .await?;

        if data["status_code"].as_i64().unwrap_or(0) != 0 {
            return Err(anyhow!(
                "API error: {}",
                data["status_msg"]
                    .as_str()
                    .or_else(|| data["message"].as_str())
                    .unwrap_or("获取评论回复失败")
            ));
        }

        let comments_data = data["comments"]
            .as_array()
            .or_else(|| data["reply_comments"].as_array());
        let has_more = Self::json_has_more(&data);
        let cursor = data["cursor"].as_i64().unwrap_or(0);
        let total = data["total"].as_i64().unwrap_or(0);
        let comments = if let Some(list) = comments_data {
            list.iter().filter_map(|c| self.parse_comment(c)).collect()
        } else {
            vec![]
        };

        Ok((comments, cursor, has_more, total))
    }

    fn parse_comment(&self, data: &serde_json::Value) -> Option<CommentInfo> {
        let user = &data["user"];
        let sticker_url = {
            let static_url = self.get_first_url(&data["sticker"]["static_url"]["url_list"]);
            if static_url.is_empty() {
                self.get_first_url(&data["sticker"]["animate_url"]["url_list"])
            } else {
                static_url
            }
        };
        Some(CommentInfo {
            cid: data["cid"].as_str()?.to_string(),
            text: data["text"].as_str().unwrap_or_default().to_string(),
            create_time: data["create_time"].as_i64().unwrap_or(0),
            user: CommentUser {
                uid: user["uid"].as_str().unwrap_or_default().to_string(),
                nickname: user["nickname"].as_str().unwrap_or_default().to_string(),
                avatar_thumb: self.get_first_url(&user["avatar_thumb"]["url_list"]),
                sec_uid: user["sec_uid"].as_str().unwrap_or_default().to_string(),
            },
            digg_count: data["digg_count"].as_i64().unwrap_or(0),
            user_digged: data["user_digged"].as_i64().unwrap_or(0) as i32,
            reply_comment_total: data["reply_comment_total"].as_i64().unwrap_or(0),
            sub_comments: None,
            status: data["status"].as_i64().unwrap_or(0) as i32,
            ip_label: data["ip_label"].as_str().unwrap_or_default().to_string(),
            sticker_url,
        })
    }

    async fn post_form_parts(
        &self,
        url: &str,
        query_params: &[(String, String)],
        body_params: &[(String, String)],
        headers: &HashMap<String, String>,
    ) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Vec<u8>)> {
        let mut req = self.client.post(url).query(query_params).form(body_params);
        for (key, value) in headers {
            req = req.header(key, value);
        }
        let response = req
            .send()
            .await
            .map_err(|error| anyhow!("HTTP request failed: {}", error))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.bytes().await?.to_vec();
        Ok((status, headers, body))
    }

    fn header_value(headers: &reqwest::header::HeaderMap, key: &str) -> String {
        headers
            .get(key)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    pub async fn set_comment_liked(
        &self,
        aweme_id: &str,
        comment_id: &str,
        liked: bool,
        level: u32,
    ) -> Result<serde_json::Value> {
        let aweme_id = aweme_id.trim();
        let comment_id = comment_id.trim();
        if aweme_id.is_empty() {
            return Err(anyhow!("作品ID不能为空"));
        }
        if comment_id.is_empty() {
            return Err(anyhow!("评论ID不能为空"));
        }

        let url = "https://www-hj.douyin.com/aweme/v1/web/comment/digg";
        let mut query_params = crate::config::get_common_params();
        query_params.insert("cid".to_string(), comment_id.to_string());
        query_params.insert("aweme_id".to_string(), aweme_id.to_string());
        query_params.insert(
            "digg_type".to_string(),
            if liked { "1" } else { "2" }.to_string(),
        );
        query_params.insert("channel_id".to_string(), "0".to_string());
        query_params.insert("app_name".to_string(), "aweme".to_string());
        query_params.insert("item_type".to_string(), "0".to_string());
        query_params.insert("level".to_string(), level.max(1).to_string());
        query_params.insert("enter_from".to_string(), "discover".to_string());
        query_params.insert("previous_page".to_string(), "discover".to_string());
        query_params.insert("update_version_code".to_string(), "170400".to_string());
        query_params.insert("version_code".to_string(), "170400".to_string());
        query_params.insert("version_name".to_string(), "17.4.0".to_string());
        query_params.insert("browser_name".to_string(), "Chrome".to_string());
        query_params.insert("browser_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("engine_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("device_memory".to_string(), "16".to_string());

        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.extend(self.relation_ticket_guard_headers("/aweme/v1/web/comment/digg"));
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
        headers.insert("Origin".to_string(), "https://www.douyin.com".to_string());
        headers.insert("sec-fetch-site".to_string(), "same-site".to_string());
        headers.insert("sec-fetch-mode".to_string(), "cors".to_string());
        headers.insert("sec-fetch-dest".to_string(), "empty".to_string());
        headers.insert("priority".to_string(), "u=1, i".to_string());
        headers.insert("x-secsdk-csrf-token".to_string(), "DOWNGRADE".to_string());
        headers.insert(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
        );
        headers.insert(
            "User-Agent".to_string(),
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".to_string(),
        );
        headers.insert(
            "sec-ch-ua".to_string(),
            "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\""
                .to_string(),
        );
        if let Some(dtrait) = self.relation_dtrait() {
            headers.insert("x-tt-session-dtrait".to_string(), dtrait);
        }

        self.enrich_request(&mut query_params, &mut headers).await;
        let params_str = serde_urlencoded::to_string(&query_params)?;
        let user_agent = headers
            .get("User-Agent")
            .map(String::as_str)
            .unwrap_or_else(|| get_user_agent());
        query_params.insert(
            "a_bogus".to_string(),
            sign::sign_detail(&params_str, user_agent),
        );

        let mut req = self
            .client
            .post(url)
            .query(&query_params)
            .body(String::new());
        for (key, value) in &headers {
            req = req.header(key, value);
        }
        let response = req
            .send()
            .await
            .map_err(|error| anyhow!("HTTP request failed: {}", error))?;
        if !response.status().is_success() {
            return Err(anyhow!("HTTP error: {}", response.status()));
        }
        let response = response.json::<serde_json::Value>().await?;
        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("评论点赞失败");
            return Err(anyhow!("评论点赞失败: {}", status_msg));
        }
        Ok(response)
    }

    pub async fn publish_comment(
        &self,
        aweme_id: &str,
        text: &str,
        reply_id: &str,
        reply_to_reply_id: &str,
    ) -> Result<(serde_json::Value, Option<CommentInfo>)> {
        let aweme_id = aweme_id.trim();
        let text = text.trim();
        if aweme_id.is_empty() {
            return Err(anyhow!("作品ID不能为空"));
        }
        if text.is_empty() {
            return Err(anyhow!("评论内容不能为空"));
        }
        self.get_current_user_strict_profile()
            .await
            .map_err(|error| anyhow!("用户未登录，请在设置中重新登录并刷新 Cookie: {}", error))?;

        let path = "/aweme/v1/web/comment/publish";
        let url = "https://www.douyin.com/aweme/v1/web/comment/publish";
        let reply_id = reply_id.trim();
        let reply_to_reply_id = reply_to_reply_id.trim();

        let cookie_dict = self.cookie_dict();
        let ms_token = cookie_dict
            .get("msToken")
            .cloned()
            .unwrap_or_else(Self::generate_ms_token);
        let cookie_with_ms_token =
            Self::cookie_string_with_value(&self.config.cookie, "msToken", &ms_token);
        let verify_fp = cookie_dict
            .get("s_v_web_id")
            .cloned()
            .unwrap_or_else(Self::generate_verify_fp);

        let mut spider_headers = HashMap::from([
            (
                "user-agent".to_string(),
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/117.0"
                    .to_string(),
            ),
            ("cache-control".to_string(), "no-cache".to_string()),
            ("pragma".to_string(), "no-cache".to_string()),
            (
                "sec-ch-ua".to_string(),
                "\"Microsoft Edge\";v=\"125\", \"Chromium\";v=\"125\", \"Not.A/Brand\";v=\"24\""
                    .to_string(),
            ),
            ("sec-ch-ua-mobile".to_string(), "?0".to_string()),
            ("sec-ch-ua-platform".to_string(), "\"Windows\"".to_string()),
            ("sec-fetch-dest".to_string(), "empty".to_string()),
            ("sec-fetch-mode".to_string(), "cors".to_string()),
            ("sec-fetch-site".to_string(), "same-origin".to_string()),
            ("priority".to_string(), "u=1, i".to_string()),
            (
                "accept".to_string(),
                "application/json, text/plain, */*".to_string(),
            ),
            (
                "accept-language".to_string(),
                "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6".to_string(),
            ),
            (
                "content-type".to_string(),
                "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
            ),
            ("Origin".to_string(), "https://www.douyin.com".to_string()),
            (
                "referer".to_string(),
                format!("https://www.douyin.com/discover?modal_id={aweme_id}"),
            ),
            ("Cookie".to_string(), cookie_with_ms_token.clone()),
        ]);
        if let Ok(ticket_headers) = self.spider_ticket_guard_headers(path) {
            spider_headers.extend(ticket_headers);
        }

        let mut spider_query = vec![
            ("app_name".to_string(), "aweme".to_string()),
            ("enter_from".to_string(), "discover".to_string()),
            ("previous_page".to_string(), "discover".to_string()),
            ("device_platform".to_string(), "webapp".to_string()),
            ("aid".to_string(), "6383".to_string()),
            ("channel".to_string(), "channel_pc_web".to_string()),
            ("pc_client_type".to_string(), "1".to_string()),
            ("update_version_code".to_string(), "170400".to_string()),
            ("version_code".to_string(), "170400".to_string()),
            ("version_name".to_string(), "17.4.0".to_string()),
            ("cookie_enabled".to_string(), "true".to_string()),
            ("screen_width".to_string(), "1707".to_string()),
            ("screen_height".to_string(), "960".to_string()),
            ("browser_language".to_string(), "zh-CN".to_string()),
            ("browser_platform".to_string(), "Win32".to_string()),
            ("browser_name".to_string(), "Edge".to_string()),
            ("browser_version".to_string(), "125.0.0.0".to_string()),
            ("browser_online".to_string(), "true".to_string()),
            ("engine_name".to_string(), "Blink".to_string()),
            ("engine_version".to_string(), "125.0.0.0".to_string()),
            ("os_name".to_string(), "Windows".to_string()),
            ("os_version".to_string(), "10".to_string()),
            ("cpu_core_num".to_string(), "32".to_string()),
            ("device_memory".to_string(), "8".to_string()),
            ("platform".to_string(), "PC".to_string()),
            ("downlink".to_string(), "10".to_string()),
            ("effective_type".to_string(), "4g".to_string()),
            ("round_trip_time".to_string(), "100".to_string()),
        ];
        let webid_url = format!("https://www.douyin.com/discover?modal_id={aweme_id}");
        let webid = self
            .get_webid_from_url(&spider_headers, &webid_url)
            .await
            .unwrap_or_else(Self::generate_fake_webid);
        spider_query.push(("webid".to_string(), webid));
        spider_query.push(("msToken".to_string(), ms_token));
        if let Some(csrf_token) = self.get_csrf_token(&spider_headers).await {
            spider_headers.insert("x-secsdk-csrf-token".to_string(), csrf_token);
        }

        let mut spider_body_for_sign = vec![
            ("aweme_id".to_string(), aweme_id.to_string()),
            (
                "comment_send_celltime".to_string(),
                rand::thread_rng().gen_range(1000..20000).to_string(),
            ),
            (
                "comment_video_celltime".to_string(),
                rand::thread_rng().gen_range(1000..20000).to_string(),
            ),
        ];
        if !reply_id.is_empty() {
            spider_body_for_sign.push(("reply_id".to_string(), reply_id.to_string()));
        }
        spider_body_for_sign.push(("text".to_string(), text.to_string()));
        let spider_body = spider_body_for_sign.clone();
        spider_body_for_sign.push(("text_extra".to_string(), "[]".to_string()));

        let spider_query_str = Self::spider_splice_params(&spider_query);
        let spider_body_str = Self::spider_splice_params(&spider_body_for_sign);
        let a_bogus = Self::sign_spider_a_bogus(&spider_query_str, &spider_body_str)?;
        spider_query.push(("a_bogus".to_string(), a_bogus));
        spider_query.push(("verifyFp".to_string(), verify_fp.clone()));
        spider_query.push(("fp".to_string(), verify_fp));
        let spider_query_parts = spider_query;
        log::debug!(
            "Douyin comment publish spider request shape: query_keys={} body_sign_keys={} body_send_keys={} csrf_present={} ticket_guard_present={} cookie_len={} signer_present={}",
            spider_query_parts
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>()
                .join(","),
            spider_body_for_sign
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>()
                .join(","),
            spider_body
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>()
                .join(","),
            spider_headers.contains_key("x-secsdk-csrf-token"),
            spider_headers.contains_key("bd-ticket-guard-client-data"),
            cookie_with_ms_token.len(),
            self.config.relation_signer.is_some(),
        );

        let (mut status, mut response_headers, mut body) = self
            .post_form_parts(url, &spider_query_parts, &spider_body, &spider_headers)
            .await?;
        let first_ticket_guard_result =
            Self::header_value(&response_headers, "bd-ticket-guard-result");
        log::debug!(
            "Douyin comment publish spider response: status={} len={} ticket_guard_result={} logid={}",
            status,
            body.len(),
            first_ticket_guard_result,
            Self::header_value(&response_headers, "x-tt-logid"),
        );

        if status.is_success() && body.is_empty() && first_ticket_guard_result == "1002" {
            let cookie_ticket_headers = Self::ticket_guard_headers_from_cookie(&self.config.cookie);
            if !cookie_ticket_headers.is_empty() {
                let mut retry_headers = spider_headers.clone();
                retry_headers
                    .retain(|key, _| !key.to_ascii_lowercase().starts_with("bd-ticket-guard-"));
                retry_headers.extend(cookie_ticket_headers);
                if let Some(csrf_token) = self.get_csrf_token(&retry_headers).await {
                    retry_headers.insert("x-secsdk-csrf-token".to_string(), csrf_token);
                }
                (status, response_headers, body) = self
                    .post_form_parts(url, &spider_query_parts, &spider_body, &retry_headers)
                    .await?;
                log::debug!(
                    "Douyin comment publish cookie-ticket response: status={} len={} ticket_guard_result={} logid={}",
                    status,
                    body.len(),
                    Self::header_value(&response_headers, "bd-ticket-guard-result"),
                    Self::header_value(&response_headers, "x-tt-logid"),
                );
            }
        }

        if status.is_success() && body.is_empty() {
            let mut query_parts = vec![
                ("device_platform".to_string(), "webapp".to_string()),
                ("aid".to_string(), "6383".to_string()),
                ("channel".to_string(), "channel_pc_web".to_string()),
                ("update_version_code".to_string(), "0".to_string()),
                ("pc_client_type".to_string(), "1".to_string()),
                ("version_code".to_string(), "190600".to_string()),
                ("version_name".to_string(), "19.6.0".to_string()),
                ("cookie_enabled".to_string(), "true".to_string()),
                ("screen_width".to_string(), "1680".to_string()),
                ("screen_height".to_string(), "1050".to_string()),
                ("browser_language".to_string(), "zh-CN".to_string()),
                ("browser_platform".to_string(), "MacIntel".to_string()),
                ("browser_name".to_string(), "Edge".to_string()),
                ("browser_version".to_string(), "145.0.0.0".to_string()),
                ("browser_online".to_string(), "true".to_string()),
                ("engine_name".to_string(), "Blink".to_string()),
                ("engine_version".to_string(), "145.0.0.0".to_string()),
                ("os_name".to_string(), "Mac OS".to_string()),
                ("os_version".to_string(), "10.15.7".to_string()),
                ("cpu_core_num".to_string(), "8".to_string()),
                ("device_memory".to_string(), "8".to_string()),
                ("platform".to_string(), "PC".to_string()),
                ("downlink".to_string(), "10".to_string()),
                ("effective_type".to_string(), "4g".to_string()),
                ("round_trip_time".to_string(), "50".to_string()),
            ];
            Self::set_param_part(&mut query_parts, "app_name", "aweme");
            Self::set_param_part(&mut query_parts, "enter_from", "discover");
            Self::set_param_part(&mut query_parts, "previous_page", "discover");
            Self::set_param_part(&mut query_parts, "update_version_code", "170400");
            Self::set_param_part(&mut query_parts, "version_code", "170400");
            Self::set_param_part(&mut query_parts, "version_name", "17.4.0");
            Self::set_param_part(&mut query_parts, "browser_name", "Chrome");
            Self::set_param_part(&mut query_parts, "browser_version", "148.0.0.0");
            Self::set_param_part(&mut query_parts, "engine_version", "148.0.0.0");
            Self::set_param_part(&mut query_parts, "device_memory", "16");

            let mut body_params = vec![
                ("aweme_id".to_string(), aweme_id.to_string()),
                ("text".to_string(), text.to_string()),
                ("text_extra".to_string(), "[]".to_string()),
                ("paste_edit_method".to_string(), "non_paste".to_string()),
                ("comment_send_celltime".to_string(), "3000".to_string()),
                ("comment_video_celltime".to_string(), "2000".to_string()),
                ("one_level_comment_rank".to_string(), "1".to_string()),
            ];
            if !reply_id.is_empty() {
                body_params.push(("reply_id".to_string(), reply_id.to_string()));
                body_params.push((
                    "reply_to_reply_id".to_string(),
                    if reply_to_reply_id.is_empty() {
                        "0"
                    } else {
                        reply_to_reply_id
                    }
                    .to_string(),
                ));
            }

            let mut headers = crate::config::get_common_headers(&self.config.cookie);
            headers.extend(self.relation_ticket_guard_headers(path));
            headers.insert(
                "Referer".to_string(),
                format!("https://www.douyin.com/video/{aweme_id}"),
            );
            headers.insert("Origin".to_string(), "https://www.douyin.com".to_string());
            headers.insert("sec-fetch-site".to_string(), "same-origin".to_string());
            headers.insert("sec-fetch-mode".to_string(), "cors".to_string());
            headers.insert("sec-fetch-dest".to_string(), "empty".to_string());
            headers.insert("priority".to_string(), "u=1, i".to_string());
            headers.insert("x-secsdk-csrf-token".to_string(), "DOWNGRADE".to_string());
            headers.insert(
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
            );
            headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".to_string());
            headers.insert(
                "sec-ch-ua".to_string(),
                "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\""
                    .to_string(),
            );
            if let Some(dtrait) = self.relation_dtrait() {
                headers.insert("x-tt-session-dtrait".to_string(), dtrait);
            }
            self.enrich_request_parts(&mut query_parts, &mut headers)
                .await;
            let params_str = serde_urlencoded::to_string(&query_parts)?;
            let user_agent = headers
                .get("User-Agent")
                .map(String::as_str)
                .unwrap_or_else(|| get_user_agent());
            query_parts.push((
                "a_bogus".to_string(),
                sign::sign_detail(&params_str, user_agent),
            ));
            (status, response_headers, body) = self
                .post_form_parts(url, &query_parts, &body_params, &headers)
                .await?;
            log::debug!(
                "Douyin comment publish relation-v2 response: status={} len={} ticket_guard_result={} logid={}",
                status,
                body.len(),
                Self::header_value(&response_headers, "bd-ticket-guard-result"),
                Self::header_value(&response_headers, "x-tt-logid"),
            );

            let relation_ticket_guard_result =
                Self::header_value(&response_headers, "bd-ticket-guard-result");
            if (!status.is_success() || body.is_empty())
                && (status == reqwest::StatusCode::FORBIDDEN
                    || matches!(relation_ticket_guard_result.as_str(), "1002" | "1205"))
            {
                let cookie_ticket_headers =
                    Self::ticket_guard_headers_from_cookie(&self.config.cookie);
                if !cookie_ticket_headers.is_empty() {
                    let mut cookie_headers = headers.clone();
                    cookie_headers
                        .retain(|key, _| !key.to_ascii_lowercase().starts_with("bd-ticket-guard-"));
                    cookie_headers.extend(cookie_ticket_headers);
                    (status, response_headers, body) = self
                        .post_form_parts(url, &query_parts, &body_params, &cookie_headers)
                        .await?;
                    log::debug!(
                        "Douyin comment publish relation-v2 cookie-ticket response: status={} len={} ticket_guard_result={} logid={}",
                        status,
                        body.len(),
                        Self::header_value(&response_headers, "bd-ticket-guard-result"),
                        Self::header_value(&response_headers, "x-tt-logid"),
                    );
                }
            }
        }

        if !status.is_success() || body.is_empty() {
            let ticket_guard_result =
                Self::header_value(&response_headers, "bd-ticket-guard-result");
            let logid = Self::header_value(&response_headers, "x-tt-logid");
            if status == reqwest::StatusCode::FORBIDDEN && !ticket_guard_result.is_empty() {
                return Err(anyhow!(
                    "评论安全参数未通过（HTTP 403, TicketGuard {}{}），请在设置中重新登录并等待安全参数采集完成后重试",
                    ticket_guard_result,
                    if logid.is_empty() {
                        "".to_string()
                    } else {
                        format!(", logid {}", logid)
                    }
                ));
            }
            return Err(anyhow!(
                "发表评论失败: HTTP {}{}{}",
                status,
                if ticket_guard_result.is_empty() {
                    "".to_string()
                } else {
                    format!(", TicketGuard {}", ticket_guard_result)
                },
                if logid.is_empty() {
                    "".to_string()
                } else {
                    format!(", logid {}", logid)
                }
            ));
        }

        let response = serde_json::from_slice::<serde_json::Value>(&body)?;
        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let raw_status_msg = response["message"]
                .as_str()
                .or_else(|| response["status_msg"].as_str())
                .or_else(|| response["prompts"].as_str())
                .or_else(|| response["status_msg_extra"].as_str())
                .map(str::trim)
                .filter(|message| !message.is_empty());
            log::warn!(
                "Douyin comment publish API rejected: status_code={} status_msg={} response={}",
                status_code,
                raw_status_msg.unwrap_or(""),
                response
            );
            let status_msg = if status_code == 8 {
                "抖音评论动作未接受当前登录态或安全参数（状态码 8），请先在抖音网页/客户端手动发一次评论后再重试".to_string()
            } else {
                match raw_status_msg {
                    Some(message) if message != "发表评论失败" => {
                        format!("{}（抖音状态码 {}）", message, status_code)
                    }
                    _ => format!("抖音评论接口返回状态码 {}", status_code),
                }
            };
            return Err(anyhow!("{}", status_msg));
        }
        let comment = response
            .get("comment")
            .and_then(|value| self.parse_comment(value));
        Ok((response, comment))
    }

    /// 解析分享链接
    pub async fn parse_share_link(&self, url: &str) -> Result<VideoInfo> {
        let share_url =
            Self::extract_share_url(url).ok_or_else(|| anyhow!("Share link is empty"))?;

        if let Some(aweme_id) = Self::extract_aweme_id(&share_url) {
            return self.get_video_detail(&aweme_id).await;
        }

        // 先请求获取重定向后的 URL
        let response = self
            .client
            .get(&share_url)
            .header("User-Agent", get_user_agent())
            .send()
            .await?;

        let final_url = response.url().to_string();

        // 提取视频 ID
        let aweme_id = Self::extract_aweme_id(&final_url)
            .ok_or_else(|| anyhow!("Cannot extract video ID from URL"))?;

        self.get_video_detail(&aweme_id).await
    }

    pub async fn set_video_liked(&self, aweme_id: &str, liked: bool) -> Result<serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("aweme_id", aweme_id.trim().to_string());
        params.insert("item_type", "0".to_string());
        // Douyin web uses type=1 for digg and type=0 for cancel. The response
        // field `is_digg` is not reliable for confirming persistence.
        params.insert("type", if liked { "1" } else { "0" }.to_string());

        self.request_relation_update(
            "https://www-hj.douyin.com/aweme/v1/web/commit/item/digg/",
            params,
            "点赞",
        )
        .await
    }

    pub async fn set_video_collected(
        &self,
        aweme_id: &str,
        collected: bool,
    ) -> Result<serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("aweme_id", aweme_id.trim().to_string());
        params.insert("action", if collected { "1" } else { "0" }.to_string());
        params.insert("aweme_type", "0".to_string());

        self.request_relation_update(
            "https://www-hj.douyin.com/aweme/v1/web/aweme/collect/",
            params,
            "收藏",
        )
        .await
    }

    pub async fn set_user_followed(&self, user_id: &str, follow: bool) -> Result<serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("user_id", user_id.trim().to_string());
        params.insert("type", if follow { "1" } else { "0" }.to_string());

        self.request_relation_update(
            "https://www-hj.douyin.com/aweme/v1/web/commit/follow/user/",
            params,
            "关注",
        )
        .await
    }


    pub async fn get_following_sec_user_ids(
        &self,
        user_id: &str,
        sec_uid: &str,
        limit: usize,
        mutual_only: bool,
    ) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut max_time = "0".to_string();
        for _ in 0..20 {
            let mut params = HashMap::new();
            params.insert("user_id", user_id.to_string());
            params.insert("sec_user_id", sec_uid.to_string());
            params.insert("count", "100".to_string());
            params.insert("max_time", max_time.clone());
            params.insert("min_time", "0".to_string());
            params.insert("source_type", "1".to_string());

            let response = self
                .request_raw_json(
                    "https://www.douyin.com/aweme/v1/web/user/following/list/",
                    Some(params),
                    "GET",
                )
                .await?;

            let status_code = response["status_code"].as_i64().unwrap_or(0);
            if status_code != 0 {
                let status_msg = response["status_msg"]
                    .as_str()
                    .or_else(|| response["message"].as_str())
                    .unwrap_or("请求失败");
                return Err(anyhow!("获取关注列表失败: {}", status_msg));
            }

            let users = response["followings"]
                .as_array()
                .or_else(|| response["user_list"].as_array())
                .or_else(|| response["data"].as_array())
                .cloned()
                .unwrap_or_default();
            for user in users {
                if mutual_only && user["follower_status"].as_i64().unwrap_or_default() <= 0 {
                    continue;
                }
                let id = user["sec_uid"]
                    .as_str()
                    .or_else(|| user["sec_user_id"].as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !id.is_empty() && seen.insert(id.clone()) {
                    ids.push(id);
                    if ids.len() >= limit {
                        return Ok(ids);
                    }
                }
            }

            let has_more = response["has_more"]
                .as_bool()
                .or_else(|| response["has_more"].as_i64().map(|value| value == 1))
                .unwrap_or(false);
            let next_max_time = response["max_time"]
                .as_i64()
                .map(|value| value.to_string())
                .or_else(|| response["max_time"].as_str().map(str::to_string))
                .unwrap_or_default();

            if next_max_time.is_empty() || next_max_time == max_time {
                break;
            }
            if !has_more {
                break;
            }
            max_time = next_max_time;
        }

        Ok(ids)
    }

    async fn request_relation_update(
        &self,
        url: &str,
        body_params: HashMap<&str, String>,
        action_name: &str,
    ) -> Result<serde_json::Value> {
        if body_params
            .get("aweme_id")
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
            && body_params
                .get("user_id")
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(anyhow!("目标ID不能为空"));
        }

        let mut query_params = crate::config::get_common_params();
        query_params.insert("update_version_code".to_string(), "170400".to_string());
        query_params.insert("version_code".to_string(), "170400".to_string());
        query_params.insert("version_name".to_string(), "17.4.0".to_string());
        query_params.insert("browser_name".to_string(), "Chrome".to_string());
        query_params.insert("browser_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("engine_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("device_memory".to_string(), "16".to_string());
        if action_name == "点赞" {
            if let Some(uid) = self.relation_uid_hash() {
                query_params.insert("uid".to_string(), uid);
            }
        }
        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        let relation_path = url::Url::parse(url)
            .map(|parsed| parsed.path().to_string())
            .unwrap_or_default();
        headers.extend(self.relation_ticket_guard_headers(&relation_path));
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
        headers.insert("Origin".to_string(), "https://www.douyin.com".to_string());
        // www.douyin.com → www-hj.douyin.com 是同站跨源，浏览器发送 same-site
        headers.insert("sec-fetch-site".to_string(), "same-site".to_string());
        headers.insert(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
        );
        headers.insert(
            "User-Agent".to_string(),
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".to_string(),
        );
        headers.insert(
            "sec-ch-ua".to_string(),
            "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\""
                .to_string(),
        );
        if let Some(dtrait) = self.relation_dtrait() {
            headers.insert("x-tt-session-dtrait".to_string(), dtrait);
        }

        self.enrich_request(&mut query_params, &mut headers).await;
        query_params.insert("browser_name".to_string(), "Chrome".to_string());
        query_params.insert("browser_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("engine_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("device_memory".to_string(), "16".to_string());
        headers.insert("x-secsdk-csrf-token".to_string(), "DOWNGRADE".to_string());
        log::debug!(
            "Douyin {} relation update request: host={} path={} uid_present={} signer_present={}",
            action_name,
            url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .unwrap_or_default(),
            relation_path,
            query_params.contains_key("uid"),
            self.config.relation_signer.is_some(),
        );
        let params_str = serde_urlencoded::to_string(&query_params)?;
        let user_agent = headers
            .get("User-Agent")
            .map(String::as_str)
            .unwrap_or_else(|| get_user_agent());
        query_params.insert(
            "a_bogus".to_string(),
            sign::sign_detail(&params_str, user_agent),
        );

        let mut req = self
            .client
            .post(url)
            .query(&query_params)
            .form(&body_params);
        for (key, value) in &headers {
            req = req.header(key, value);
        }

        let response = req
            .send()
            .await
            .map_err(|error| anyhow!("HTTP request failed: {}", error))?;
        if !response.status().is_success() {
            let status = response.status();
            let ticket_guard_result = response
                .headers()
                .get("bd-ticket-guard-result")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let passport_security_gateway = response
                .headers()
                .get("bd_passport_security_gateway")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            log::warn!(
                "Douyin {} relation update rejected: http_status={} ticket_guard_result={:?} passport_gateway={:?}",
                action_name,
                status,
                ticket_guard_result,
                passport_security_gateway,
            );
            if status.as_u16() == 403
                && (ticket_guard_result.is_some()
                    || passport_security_gateway.as_deref() == Some("1"))
            {
                return Err(anyhow!(
                    "RELATION_SECURITY_GATEWAY: 抖音安全校验拒绝了{}操作（HTTP 403{}），当前 Cookie 仍会保留，请稍后重试，或先在抖音网页/客户端完成一次同类操作。",
                    action_name,
                    ticket_guard_result
                        .as_deref()
                        .map(|value| format!(", TicketGuard {}", value))
                        .unwrap_or_default()
                ));
            }
            return Err(anyhow!("HTTP error: {}", status));
        }

        let response = response.json::<serde_json::Value>().await?;
        log::debug!(
            "Douyin {} relation update response: status_code={} status_msg={}",
            action_name,
            response["status_code"].as_i64().unwrap_or(0),
            response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or(""),
        );

        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("请求失败");
            log::warn!(
                "Douyin {} relation update failed: code={} message={}",
                action_name,
                status_code,
                status_msg,
            );
            if status_code == 8
                || status_msg.contains("用户未登录")
                || status_msg.contains("未登录")
            {
                return Err(anyhow!(
                    "RELATION_SECURITY_GATEWAY: 抖音{}动作接口未接受当前网页登录凭据（{}），当前 Cookie 仍会保留。请稍后重试，或先在抖音网页/客户端完成一次同类操作。",
                    action_name,
                    status_msg
                ));
            }
            return Err(anyhow!("{}失败: {}", action_name, status_msg));
        }

        Ok(response)
    }

    /// 验证 Cookie 是否有效
    pub async fn verify_cookie(&self) -> Result<CookieStatus> {
        match self.get_current_user_from_profile_self().await {
            Ok(user) => Ok(CookieStatus {
                valid: true,
                user_name: Some(user.nickname),
                user_id: Some(if user.uid.is_empty() {
                    user.sec_uid.clone()
                } else {
                    user.uid.clone()
                }),
                sec_uid: if user.sec_uid.is_empty() {
                    None
                } else {
                    Some(user.sec_uid)
                },
                avatar_thumb: if user.avatar_thumb.is_empty() {
                    None
                } else {
                    Some(user.avatar_thumb)
                },
                avatar_medium: if user.avatar_medium.is_empty() {
                    None
                } else {
                    Some(user.avatar_medium)
                },
                avatar_larger: if user.avatar_larger.is_empty() {
                    None
                } else {
                    Some(user.avatar_larger)
                },
                expires_at: None,
                message: "Cookie 有效".to_string(),
            }),
            Err(e) => {
                let cookies = crate::cookie::parse_cookie_string(&self.config.cookie);
                if crate::cookie::has_douyin_session_cookie(&cookies) {
                    if looks_like_logged_out_error(&e.to_string()) {
                        log::warn!("Douyin profile cookie check reports logged out: {}", e);
                        return Ok(CookieStatus {
                            valid: false,
                            user_name: None,
                            user_id: None,
                            sec_uid: None,
                            avatar_thumb: None,
                            avatar_medium: None,
                            avatar_larger: None,
                            expires_at: None,
                            message: "用户未登录，请在设置中重新登录并刷新 Cookie".to_string(),
                        });
                    }
                    match self.check_passport_account_expired().await {
                        Ok(Some(message)) => {
                            log::warn!("Douyin passport reports saved cookie expired: {}", message);
                            return Ok(CookieStatus {
                                valid: false,
                                user_name: None,
                                user_id: None,
                                sec_uid: None,
                                avatar_thumb: None,
                                avatar_medium: None,
                                avatar_larger: None,
                                expires_at: None,
                                message: format!("Cookie 会话已过期，请重新登录: {}", message),
                            });
                        }
                        Ok(None) => {}
                        Err(error) => {
                            log::warn!("Douyin passport account check failed: {}", error);
                        }
                    }
                    log::warn!(
                        "Douyin profile cookie check failed; treating saved cookie as unavailable for action APIs: {}",
                        e
                    );
                    return Ok(CookieStatus {
                        valid: false,
                        user_name: None,
                        user_id: None,
                        sec_uid: None,
                        avatar_thumb: None,
                        avatar_medium: None,
                        avatar_larger: None,
                        expires_at: None,
                        message: format!("用户未登录，请在设置中重新登录并刷新 Cookie: {}", e),
                    });
                }

                Ok(CookieStatus {
                    valid: false,
                    user_name: None,
                    user_id: None,
                    sec_uid: None,
                    avatar_thumb: None,
                    avatar_medium: None,
                    avatar_larger: None,
                    expires_at: None,
                    message: if looks_like_logged_out_error(&e.to_string()) {
                        "用户未登录，请在设置中重新登录并刷新 Cookie".to_string()
                    } else {
                        format!("Cookie 无效: {}", e)
                    },
                })
            }
        }
    }

    async fn check_passport_account_expired(&self) -> Result<Option<String>> {
        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());

        let mut req = self
            .client
            .get("https://www.douyin.com/passport/web/account/info/");
        for (key, value) in &headers {
            req = req.header(key, value);
        }

        let response = req.send().await?;
        if !response.status().is_success() {
            return Ok(None);
        }

        let json = response.json::<serde_json::Value>().await?;
        let message = json["message"].as_str().unwrap_or_default();
        let error_code = json["data"]["error_code"].as_i64().unwrap_or(0);
        let description = json["data"]["description"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        if message == "error"
            && error_code == 1
            && (description.contains("会话过期")
                || description.contains("重新登录")
                || description.contains("登录"))
        {
            return Ok(Some(if description.is_empty() {
                "会话过期".to_string()
            } else {
                description
            }));
        }

        Ok(None)
    }

    /// 获取当前用户信息 (需要登录)
    pub async fn get_current_user(&self) -> Result<UserInfo> {
        match self.get_current_user_from_profile_self().await {
            Ok(user) => Ok(user),
            Err(profile_error) => {
                log::warn!(
                    "Douyin profile/self current user lookup failed: {}",
                    profile_error
                );
                self.get_current_user_from_query_user().await
            }
        }
    }

    /// 获取当前用户信息，不使用 query/user 兜底。动作接口必须通过该强校验。
    pub async fn get_current_user_strict_profile(&self) -> Result<UserInfo> {
        self.get_current_user_from_profile_self().await
    }

    async fn get_current_user_from_profile_self(&self) -> Result<UserInfo> {
        let headers = HashMap::from([(
            "Accept-Encoding".to_string(),
            "identity;q=1, *;q=0".to_string(),
        )]);
        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/user/profile/self/",
                None,
                "GET",
                Some(headers),
                true,
            )
            .await
            .map_err(|error| {
                anyhow!("当前登录态可访问 IM 接口，但个人资料接口不可用: {}", error)
            })?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let data = response
            .get("user")
            .or_else(|| response.pointer("/data/user"))
            .or_else(|| {
                response.get("data").filter(|value| {
                    value.get("uid").is_some()
                        || value.get("sec_uid").is_some()
                        || value.get("user_id").is_some()
                })
            })
            .ok_or_else(|| anyhow!("No user in response"))?;

        let avatar_thumb = self.get_avatar_url(
            data,
            &[
                "avatar_thumb",
                "avatar_100x100",
                "avatar_168x168",
                "avatar_medium",
                "avatar_300x300",
                "avatar_larger",
            ],
        );
        let avatar_medium = self.get_avatar_url(
            data,
            &[
                "avatar_medium",
                "avatar_168x168",
                "avatar_300x300",
                "avatar_larger",
                "avatar_thumb",
                "avatar_100x100",
            ],
        );
        let avatar_larger = self.get_avatar_url(
            data,
            &[
                "avatar_larger",
                "avatar_300x300",
                "avatar_medium",
                "avatar_168x168",
                "avatar_thumb",
                "avatar_100x100",
            ],
        );
        log::debug!(
            "Douyin profile/self current user parsed: uid_present={} sec_uid_present={} avatar_thumb_present={} avatar_medium_present={} avatar_larger_present={}",
            !data["uid"].as_str().unwrap_or_default().is_empty(),
            !data["sec_uid"].as_str().unwrap_or_default().is_empty(),
            !avatar_thumb.is_empty(),
            !avatar_medium.is_empty(),
            !avatar_larger.is_empty(),
        );

        Ok(UserInfo {
            uid: data["uid"].as_str().unwrap_or_default().to_string(),
            nickname: data["nickname"].as_str().unwrap_or_default().to_string(),
            avatar_thumb,
            avatar_medium,
            avatar_larger,
            signature: data["signature"].as_str().unwrap_or_default().to_string(),
            follower_count: data["follower_count"].as_i64().unwrap_or(0),
            following_count: data["following_count"].as_i64().unwrap_or(0),
            total_favorited: data["total_favorited"].as_i64().unwrap_or(0),
            aweme_count: data["aweme_count"].as_i64().unwrap_or(0),
            favoriting_count: data["favoriting_count"].as_i64().unwrap_or(0),
            is_follow: false,
            follow_status: data["follow_status"].as_i64().unwrap_or(0) as i32,
            sec_uid: data["sec_uid"].as_str().unwrap_or_default().to_string(),
            unique_id: data["unique_id"].as_str().unwrap_or_default().to_string(),
            verify_status: data["verify_status"].as_i64().unwrap_or(0) as i32,
        })
    }

    async fn get_current_user_from_query_user(&self) -> Result<UserInfo> {
        let mut params = HashMap::new();
        params.insert("publish_video_strategy_type", "2".to_string());
        let headers = HashMap::from([
            (
                "Referer".to_string(),
                "https://www.douyin.com/discover".to_string(),
            ),
            (
                "Accept-Encoding".to_string(),
                "identity;q=1, *;q=0".to_string(),
            ),
        ]);
        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/query/user",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;
        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }
        let uid = response
            .get("user_uid")
            .and_then(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .or_else(|| value.as_i64().map(|number| number.to_string()))
            })
            .unwrap_or_default()
            .trim()
            .to_string();
        if uid.is_empty() {
            return Err(anyhow!("query/user 未返回 user_uid"));
        }
        Ok(UserInfo {
            uid: uid.clone(),
            nickname: "抖音用户".to_string(),
            avatar_thumb: String::new(),
            avatar_medium: String::new(),
            avatar_larger: String::new(),
            signature: String::new(),
            follower_count: 0,
            following_count: 0,
            total_favorited: 0,
            aweme_count: 0,
            favoriting_count: 0,
            is_follow: false,
            follow_status: 0,
            sec_uid: String::new(),
            unique_id: uid,
            verify_status: 0,
        })
    }
}

fn looks_like_logged_out_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    message.contains("用户未登录")
        || message.contains("未登录")
        || message.contains("登录态")
        || message.contains("重新登录")
        || lower.contains("not login")
        || lower.contains("not logged in")
        || lower.contains("login required")
        || lower.contains("session expired")
}

#[cfg(test)]
mod tests {
    use super::DouyinClient;
    use crate::api::MediaType;
    use crate::config::AppConfig;
    use serde_json::json;

    #[test]
    fn extracts_share_url_from_copied_text() {
        let text = "1.23 复制打开抖音 https://v.douyin.com/iRNBho6/，看TA的作品";
        assert_eq!(
            DouyinClient::extract_share_url(text).as_deref(),
            Some("https://v.douyin.com/iRNBho6/")
        );
    }

    #[test]
    fn normalizes_www_share_url() {
        assert_eq!(
            DouyinClient::extract_share_url("www.douyin.com/video/7341234567890123456。")
                .as_deref(),
            Some("https://www.douyin.com/video/7341234567890123456")
        );
    }

    #[test]
    fn extracts_aweme_id_from_common_link_shapes() {
        assert_eq!(
            DouyinClient::extract_aweme_id("https://www.douyin.com/video/7341234567890123456"),
            Some("7341234567890123456".to_string())
        );
        assert_eq!(
            DouyinClient::extract_aweme_id("https://www.douyin.com/note/7341234567890123456"),
            Some("7341234567890123456".to_string())
        );
        assert_eq!(
            DouyinClient::extract_aweme_id("https://www.douyin.com/?modal_id=7341234567890123456"),
            Some("7341234567890123456".to_string())
        );
        assert_eq!(
            DouyinClient::extract_aweme_id("7341234567890123456"),
            Some("7341234567890123456".to_string())
        );
    }

    #[test]
    fn selects_dash_audio_url_from_object_and_array_shapes() {
        let object_shape = json!({
            "bit_rate_audio": [{
                "audio_meta": {
                    "url_list": {
                        "main_url": "",
                        "backup_url": "https://example.com/audio-backup.mp4",
                        "fallback_url": "https://example.com/audio-fallback.mp4"
                    }
                }
            }]
        });
        assert_eq!(
            DouyinClient::select_dash_audio_url(&object_shape).as_deref(),
            Some("https://example.com/audio-backup.mp4")
        );

        let array_shape = json!({
            "bit_rate_audio": [{
                "audio_meta": {
                    "url_list": [
                        "",
                        "https://example.com/audio-array.mp4"
                    ]
                }
            }]
        });
        assert_eq!(
            DouyinClient::select_dash_audio_url(&array_shape).as_deref(),
            Some("https://example.com/audio-array.mp4")
        );
    }

    #[test]
    fn live_photo_post_does_not_add_static_cover_as_extra_media() {
        let client = DouyinClient::new(AppConfig::default()).expect("client");
        let post = json!({
            "aweme_id": "7341234567890123456",
            "desc": "live photo post",
            "author": {},
            "statistics": {},
            "status": {},
            "video": {},
            "images": [{
                "url_list": [
                    "https://example.com/image-small.webp",
                    "https://example.com/image-large.jpeg"
                ],
                "video": {
                    "play_addr": {
                        "url_list": ["https://example.com/live-photo.mp4"]
                    }
                }
            }]
        });

        let video = client.parse_video_info(&post).expect("video info");

        assert_eq!(
            video.live_photo_urls.as_ref().expect("live photos"),
            &vec!["https://example.com/live-photo.mp4".to_string()]
        );
        assert!(video.image_urls.is_none());
        assert_eq!(video.media_type, MediaType::LivePhoto);
    }
}
