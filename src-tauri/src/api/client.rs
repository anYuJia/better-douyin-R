//! API 客户端

use crate::config::{get_user_agent, AppConfig};
use crate::http_client::{apply_douyin_api_client_config, apply_tls_config};
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
use std::collections::{BTreeMap, HashMap};
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
        let mut builder = apply_tls_config(
            apply_douyin_api_client_config(reqwest::Client::builder()).redirect(Policy::limited(5)),
            &config,
        );

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

    pub(super) fn cookie_string_with_value(cookie_str: &str, name: &str, value: &str) -> String {
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

    pub(super) fn spider_ticket_guard_headers(
        &self,
        path: &str,
    ) -> Result<HashMap<String, String>> {
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

    pub(super) fn relation_uid_hash(&self) -> Option<String> {
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

    pub(super) fn generate_fake_webid() -> String {
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

    pub(super) fn sign_spider_a_bogus(query: &str, body: &str) -> Result<String> {
        Ok(sign::sign_spider_publish(query, body))
    }

    fn spider_quote(value: &str) -> String {
        urlencoding::encode(value)
            .replace("%2F", "/")
            .replace("%2f", "/")
    }

    pub(super) fn spider_splice_params(params: &[(String, String)]) -> String {
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

    pub(super) async fn get_webid_from_url(
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

    pub(super) async fn get_csrf_token(&self, headers: &HashMap<String, String>) -> Option<String> {
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

    pub(super) fn set_param_part(
        params: &mut Vec<(String, String)>,
        key: &str,
        value: impl Into<String>,
    ) {
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

    pub(super) async fn enrich_request_parts(
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

        let normalized_url = crate::http_client::normalize_request_url(url);
        let mut req = match method {
            "GET" => self.client.get(&normalized_url).query(&all_params),
            "POST" => self.client.post(&normalized_url).form(&all_params),
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
            tokio::spawn(async { crate::config::AppConfig::flush_config_syncs().await });
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
}
