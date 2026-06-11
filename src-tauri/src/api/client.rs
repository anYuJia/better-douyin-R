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
use reqwest::header::SET_COOKIE;
use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::im_proto;
use super::types::*;

const PUBLISH_COOKIE_MERGE_DENYLIST: &[&str] = &[
    "sessionid",
    "sessionid_ss",
    "sid_guard",
    "uid_tt",
    "uid_tt_ss",
    "sid_tt",
    "passport_auth_status",
    "passport_auth_status_ss",
    "passport_csrf_token",
    "passport_csrf_token_default",
    "odin_tt",
];

fn looks_watermarked_media_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("watermark=1") || lower.contains("playwm") || lower.contains("logo_name=")
}

fn clean_video_media_url(url: &str) -> String {
    url.trim()
        .replace("watermark=1", "watermark=0")
        .replace("playwm", "play")
}

type HmacSha256 = Hmac<Sha256>;
const IM_HISTORY_PAGE_SIZE: i64 = 20;

fn crc32_hex(bytes: &[u8]) -> String {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    format!("{:08x}", !crc)
}

/// 抖音 API 客户端
#[derive(Clone)]
pub struct DouyinClient {
    client: reqwest::Client,
    config: AppConfig,
    webid_cache: Arc<Mutex<Option<(String, Instant)>>>,
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

        Ok(Self {
            client,
            config,
            webid_cache: Arc::new(Mutex::new(None)),
        })
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
        let cookie_dict = Self::cookies_to_dict(&self.config.cookie);
        cookie_dict
            .get("sessionid")
            .or_else(|| cookie_dict.get("sessionid_ss"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn ticket_guard_headers_from_cookie(cookie_str: &str) -> HashMap<String, String> {
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

    fn merge_response_cookies(cookie_str: &str, headers: &reqwest::header::HeaderMap) -> Option<String> {
        let mut updates = HashMap::new();
        for value in headers.get_all(SET_COOKIE).iter() {
            let Ok(header) = value.to_str() else {
                continue;
            };
            let Some(first_part) = header.split(';').next() else {
                continue;
            };
            let Some((name, cookie_value)) = first_part.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || cookie_value.is_empty() {
                continue;
            }
            updates.insert(name.to_string(), cookie_value.to_string());
        }

        if updates.is_empty() {
            return None;
        }

        let mut cookie_dict = Self::cookies_to_dict(cookie_str);
        let mut changed = false;
        for (name, value) in updates {
            if PUBLISH_COOKIE_MERGE_DENYLIST.contains(&name.as_str()) {
                continue;
            }
            if cookie_dict.get(&name) != Some(&value) {
                cookie_dict.insert(name, value);
                changed = true;
            }
        }
        if !changed {
            return None;
        }

        Some(
            cookie_dict
                .into_iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
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

    fn relation_ticket_guard_headers(&self, path: &str) -> HashMap<String, String> {
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
        let cookie_dict = Self::cookies_to_dict(&self.config.cookie);
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

    fn relation_dtrait(&self) -> Option<String> {
        let value = self.config.relation_signer.as_ref()?.dtrait.trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }

    fn im_proto_signer(&self) -> Result<&crate::config::RelationSignerConfig> {
        let signer =
            self.config.relation_signer.as_ref().ok_or_else(|| {
                anyhow!("私信安全参数未采集完整，请在设置中重新登录 Cookie 后重试")
            })?;
        if signer.ticket.trim().is_empty()
            || signer.ts_sign.trim().is_empty()
            || signer.client_cert.trim().is_empty()
            || signer.private_key.trim().is_empty()
        {
            return Err(anyhow!(
                "私信安全参数未采集完整，请在设置中重新登录 Cookie 后重试"
            ));
        }
        Ok(signer)
    }

    fn ecdsa_request_sign(value: &str, private_key: &str) -> Result<String> {
        let pem = private_key.trim().replace("\\n", "\n");
        let key = PKey::private_key_from_pem(pem.as_bytes())
            .map_err(|error| anyhow!("私信签名生成失败: {}", error))?;
        let mut signer = Signer::new(MessageDigest::sha256(), &key)
            .map_err(|error| anyhow!("私信签名生成失败: {}", error))?;
        signer
            .update(value.as_bytes())
            .map_err(|error| anyhow!("私信签名生成失败: {}", error))?;
        let signature = signer
            .sign_to_vec()
            .map_err(|error| anyhow!("私信签名生成失败: {}", error))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(signature))
    }

    fn build_im_request_common_headers(
        &self,
        extra_headers: Option<&HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let cookie_dict = Self::cookies_to_dict(&self.config.cookie);
        let user_agent = get_user_agent();
        let mut headers = HashMap::from([
            ("session_aid".to_string(), "6383".to_string()),
            ("session_did".to_string(), "0".to_string()),
            ("app_name".to_string(), "douyin_pc".to_string()),
            ("priority_region".to_string(), "cn".to_string()),
            ("user_agent".to_string(), user_agent.to_string()),
            ("cookie_enabled".to_string(), "true".to_string()),
            ("browser_language".to_string(), "zh-CN".to_string()),
            ("browser_platform".to_string(), "Win32".to_string()),
            ("browser_name".to_string(), "Mozilla".to_string()),
            (
                "browser_version".to_string(),
                user_agent
                    .split_once("Mozilla/")
                    .map(|(_, value)| value.to_string())
                    .unwrap_or_else(|| user_agent.to_string()),
            ),
            ("browser_online".to_string(), "true".to_string()),
            ("screen_width".to_string(), "1680".to_string()),
            ("screen_height".to_string(), "1050".to_string()),
            ("referer".to_string(), "".to_string()),
            ("timezone_name".to_string(), "Etc/GMT-8".to_string()),
            ("deviceId".to_string(), "0".to_string()),
            (
                "webid".to_string(),
                cookie_dict
                    .get("webid")
                    .or_else(|| cookie_dict.get("ttwid"))
                    .cloned()
                    .unwrap_or_default(),
            ),
            (
                "fp".to_string(),
                cookie_dict
                    .get("s_v_web_id")
                    .cloned()
                    .unwrap_or_else(Self::generate_verify_fp),
            ),
            ("is-retry".to_string(), "0".to_string()),
        ]);
        if let Some(extra_headers) = extra_headers {
            for (key, value) in extra_headers {
                let value = value.trim();
                if !key.trim().is_empty() && !value.is_empty() {
                    headers.insert(key.trim().to_string(), value.to_string());
                }
            }
        }
        headers
    }

    fn build_im_proto_request(
        &self,
        cmd: i64,
        body: &[u8],
        request_sign: &str,
        sdk_version: &str,
        build_number: &str,
        extra_headers: Option<&HashMap<String, String>>,
    ) -> Result<Vec<u8>> {
        let signer = self.im_proto_signer()?;
        let sdk_cert =
            base64::engine::general_purpose::STANDARD.encode(signer.client_cert.as_bytes());
        Ok(im_proto::build_request(
            cmd,
            signer.ticket.trim(),
            signer.ts_sign.trim(),
            &sdk_cert,
            request_sign,
            body,
            &self.build_im_request_common_headers(extra_headers),
            rand::thread_rng().gen_range(10000..=11000),
            sdk_version,
            build_number,
        ))
    }

    fn build_im_pc_proto_request(&self, cmd: i64, body: &[u8]) -> Result<Vec<u8>> {
        self.build_im_pc_proto_request_with_headers(cmd, body, None)
    }

    fn build_im_pc_proto_request_with_headers(
        &self,
        cmd: i64,
        body: &[u8],
        extra_headers: Option<&HashMap<String, String>>,
    ) -> Result<Vec<u8>> {
        self.build_im_proto_request(cmd, body, "", "0.1.6", "fef1a80:p/lzg/store", extra_headers)
    }

    async fn post_im_proto(
        &self,
        url: &str,
        payload: Vec<u8>,
        with_signed_query: bool,
    ) -> Result<serde_json::Value> {
        let headers = HashMap::from([
            ("User-Agent".to_string(), get_user_agent().to_string()),
            ("Cookie".to_string(), self.config.cookie.clone()),
            ("accept".to_string(), "application/x-protobuf".to_string()),
            (
                "content-type".to_string(),
                "application/x-protobuf".to_string(),
            ),
            ("referer".to_string(), "https://www.douyin.com/".to_string()),
            ("origin".to_string(), "https://www.douyin.com".to_string()),
        ]);
        let mut req = self.client.post(url);
        if with_signed_query {
            let cookie_dict = Self::cookies_to_dict(&self.config.cookie);
            let fp = cookie_dict
                .get("s_v_web_id")
                .cloned()
                .unwrap_or_else(Self::generate_verify_fp);
            let mut query_params = HashMap::from([
                ("verifyFp".to_string(), fp.clone()),
                ("fp".to_string(), fp),
                ("msToken".to_string(), Self::generate_ms_token()),
            ]);
            let params_str = serde_urlencoded::to_string(&query_params)?;
            let user_agent = headers
                .get("User-Agent")
                .map(String::as_str)
                .unwrap_or_else(|| get_user_agent());
            query_params.insert(
                "a_bogus".to_string(),
                sign::sign_detail(&params_str, user_agent),
            );
            req = req.query(&query_params);
        }
        for (key, value) in &headers {
            req = req.header(key, value);
        }
        let response = req.body(payload).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "IM protobuf 接口失败（HTTP {}）",
                response.status()
            ));
        }
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            return Err(anyhow!("IM protobuf 接口失败（响应为空）"));
        }
        let parsed = im_proto::parse_response(&bytes);
        let response_message = parsed
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        let message_is_error = !response_message.is_empty()
            && !matches!(
                response_message.to_ascii_lowercase().as_str(),
                "ok" | "success"
            );
        if parsed
            .get("error_desc")
            .and_then(|value| value.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            || message_is_error
        {
            let message = parsed
                .get("error_desc")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .or_else(|| parsed.get("message").and_then(|value| value.as_str()))
                .unwrap_or("IM protobuf 接口返回错误");
            return Err(anyhow!("{}", message));
        }
        Ok(parsed)
    }

    fn generate_ms_token() -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(107)
            .map(char::from)
            .collect()
    }

    fn generate_verify_fp() -> String {
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

    fn aws_vod_auth_headers(
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
        let patterns = [
            r#"\\"user_unique_id\\":\\"(\d+)\\""#,
            r#""user_unique_id":"(\d+)""#,
            r#""webid":"(\d+)""#,
            r#"webid=(\d+)"#,
        ];

        for pattern in patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(caps) = re.captures(&html) {
                    if let Some(matched) = caps.get(1) {
                        let webid = matched.as_str().to_string();
                        let mut cache = self.webid_cache.lock().await;
                        *cache = Some((webid.clone(), Instant::now()));
                        return Some(webid);
                    }
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

    async fn enrich_request(
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

        log::info!(
            "API request started: method={} url={} skip_sign={} cookie_present={} cookie_len={} sessionid_present={} csrf_cookie_present={}",
            method,
            url,
            skip_sign,
            headers
                .get("Cookie")
                .or_else(|| headers.get("cookie"))
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            headers
                .get("Cookie")
                .or_else(|| headers.get("cookie"))
                .map(|value| value.len())
                .unwrap_or(0),
            headers
                .get("Cookie")
                .or_else(|| headers.get("cookie"))
                .map(|value| value.contains("sessionid="))
                .unwrap_or(false),
            headers
                .get("Cookie")
                .or_else(|| headers.get("cookie"))
                .map(|value| value.contains("passport_csrf_token="))
                .unwrap_or(false)
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
        log::info!(
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

        let value = Regex::new(r#"https?://[^\s<>"']+|www\.[^\s<>"']+"#)
            .ok()
            .and_then(|re| re.find(trimmed).map(|matched| matched.as_str().to_string()))
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
        if Regex::new(r"^\d+$").unwrap().is_match(url) {
            return Some(url.to_string());
        }

        // 从分享链接提取
        let patterns = [
            r"video/(\d+)",
            r"note/(\d+)",
            r"aweme_id=(\d+)",
            r"modal_id=(\d+)",
            r"/(\d{18,21})",
        ];

        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(caps) = re.captures(url) {
                    if let Some(id) = caps.get(1) {
                        return Some(id.as_str().to_string());
                    }
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
            is_follow: author_data["is_follow"].as_bool().unwrap_or(false),
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
            .and_then(|br| br["play_addr"]["url_list"].as_array())
            .and_then(|urls| urls.first())
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());
        let fallback_play_addr = self.get_first_url(&video_data["play_addr"]["url_list"]);
        let download_addr = self.get_first_url_opt(&video_data["download_addr"]["url_list"]);
        let primary_no_watermark = [
            bit_rate_play_addr.clone(),
            self.get_first_url_opt(&video_data["play_addr_h264"]["url_list"]),
            Some(fallback_play_addr.clone()),
            self.get_first_url_opt(&video_data["play_addr_lowbr"]["url_list"]),
            download_addr.clone(),
        ]
        .into_iter()
        .flatten()
        .find(|url| !url.is_empty() && !looks_watermarked_media_url(url));
        let play_addr = primary_no_watermark
            .or(bit_rate_play_addr)
            .or({
                if fallback_play_addr.is_empty() {
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
            play_addr_h264: self.get_first_url_opt(&video_data["play_addr_h264"]["url_list"]),
            play_addr_lowbr: self.get_first_url_opt(&video_data["play_addr_lowbr"]["url_list"]),
            download_addr: self.get_first_url_opt(&video_data["download_addr"]["url_list"]),
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
                        play_addr: self.get_first_url_opt(&b["play_addr"]["url_list"]),
                        play_addr_h264: self.get_first_url_opt(&b["play_addr_h264"]["url_list"]),
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
                }

                if let Some(url) = image
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

        log::info!(
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
            for key in ["url_list", "url", "download_url", "play_url", "display_url"] {
                if let Some(url) = obj.get(key).and_then(|value| self.get_first_url_opt(value)) {
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
            let urls = &audio_rate["audio_meta"]["url_list"];
            for key in ["main_url", "backup_url", "fallback_url"] {
                if let Some(url) = urls[key]
                    .as_str()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                {
                    return Some(url.to_string());
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
                media_type = "video".to_string();
                urls.push(LikedVideoMediaUrl {
                    r#type: "video".to_string(),
                    url: url.to_string(),
                });
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
        let raw_play_addr = self.get_first_url(&video_data["play_addr"]["url_list"]);
        let selected_play_addr = clean_video_media_url(&raw_play_addr);

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
            if raw_play_addr.is_empty() {
                fallback_media_url.clone()
            } else {
                clean_video_media_url(&raw_play_addr)
            }
        } else {
            selected_play_addr.clone()
        };
        let duration = video_data["duration"].as_i64().unwrap_or(0);
        let bit_rate = video_data["bit_rate"].as_array().and_then(|arr| {
            let items = arr
                .iter()
                .filter_map(|b| {
                    let play_addr = self.get_first_url_opt(&b["play_addr"]["url_list"]);
                    let play_addr_h264 = self.get_first_url_opt(&b["play_addr_h264"]["url_list"]);
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
                play_addr_h264: self.get_first_url_opt(&video_data["play_addr_h264"]["url_list"]),
                play_addr_lowbr: self.get_first_url_opt(&video_data["play_addr_lowbr"]["url_list"]),
                download_addr: self.get_first_url_opt(&video_data["download_addr"]["url_list"]),
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
                            is_follow: user["is_follow"].as_bool().unwrap_or(false),
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
            is_follow: user_data["is_follow"].as_bool().unwrap_or(false),
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
            is_favorite: response["is_favorite"].as_bool().unwrap_or(false),
            follow_status: response["follow_status"].as_i64().unwrap_or(0) as i32,
            story_count: response["story_count"].as_i64().unwrap_or(0),
            friend_status: response["friend_status"].as_i64().unwrap_or(0) as i32,
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
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
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
    ) -> Result<(serde_json::Value, Option<CommentInfo>, Option<String>)> {
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

        let cookie_dict = Self::cookies_to_dict(&self.config.cookie);
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
        spider_body_for_sign.push(("text_extra".to_string(), "[]".to_string()));
        let spider_body = spider_body_for_sign.clone();

        let spider_query_str = Self::spider_splice_params(&spider_query);
        let spider_body_str = Self::spider_splice_params(&spider_body_for_sign);
        let a_bogus = Self::sign_spider_a_bogus(&spider_query_str, &spider_body_str)?;
        spider_query.push(("a_bogus".to_string(), a_bogus));
        spider_query.push(("verifyFp".to_string(), verify_fp.clone()));
        spider_query.push(("fp".to_string(), verify_fp));
        let spider_query_parts = spider_query;
        log::info!(
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

        let mut updated_cookie: Option<String> = None;
        let (mut status, mut response_headers, mut body) = self
            .post_form_parts(url, &spider_query_parts, &spider_body, &spider_headers)
            .await?;
        if let Some(next_cookie) =
            Self::merge_response_cookies(updated_cookie.as_deref().unwrap_or(&self.config.cookie), &response_headers)
        {
            log::info!("Douyin comment publish merged response cookies from spider response");
            updated_cookie = Some(next_cookie);
        }
        let first_ticket_guard_result =
            Self::header_value(&response_headers, "bd-ticket-guard-result");
        log::info!(
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
                if let Some(next_cookie) = Self::merge_response_cookies(
                    updated_cookie.as_deref().unwrap_or(&self.config.cookie),
                    &response_headers,
                ) {
                    log::info!("Douyin comment publish merged response cookies from cookie-ticket response");
                    updated_cookie = Some(next_cookie);
                }
                log::info!(
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
            if let Some(next_cookie) = Self::merge_response_cookies(
                updated_cookie.as_deref().unwrap_or(&self.config.cookie),
                &response_headers,
            ) {
                log::info!("Douyin comment publish merged response cookies from relation-v2 response");
                updated_cookie = Some(next_cookie);
            }
            log::info!(
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
                    if let Some(next_cookie) = Self::merge_response_cookies(
                        updated_cookie.as_deref().unwrap_or(&self.config.cookie),
                        &response_headers,
                    ) {
                        log::info!("Douyin comment publish merged response cookies from relation-v2 cookie-ticket response");
                        updated_cookie = Some(next_cookie);
                    }
                    log::info!(
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
        Ok((response, comment, updated_cookie))
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

    fn im_common_headers(&self, path: &str) -> HashMap<String, String> {
        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.extend(Self::ticket_guard_headers_from_cookie(&self.config.cookie));
        headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
        headers.insert("Origin".to_string(), "https://www.douyin.com".to_string());
        headers.insert("sec-fetch-site".to_string(), "same-site".to_string());
        headers.insert(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded; charset=UTF-8".to_string(),
        );
        headers.insert("x-secsdk-csrf-token".to_string(), "DOWNGRADE".to_string());
        if let Some(dtrait) = self.relation_dtrait() {
            headers.insert("x-tt-session-dtrait".to_string(), dtrait);
        }
        if path.contains("/im/") {
            headers.insert(
                "sec-ch-ua".to_string(),
                "\"Chromium\";v=\"148\", \"Microsoft Edge\";v=\"148\", \"Not/A)Brand\";v=\"99\""
                    .to_string(),
            );
            headers.insert(
                "User-Agent".to_string(),
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36 Edg/148.0.0.0".to_string(),
            );
        }
        headers
    }

    async fn request_im_post(
        &self,
        url: &str,
        body_params: HashMap<&str, String>,
    ) -> Result<serde_json::Value> {
        let relation_path = url::Url::parse(url)
            .map(|parsed| parsed.path().to_string())
            .unwrap_or_default();
        let mut query_params = crate::config::get_common_params();
        query_params.insert("update_version_code".to_string(), "170400".to_string());
        query_params.insert("version_code".to_string(), "170400".to_string());
        query_params.insert("version_name".to_string(), "17.4.0".to_string());
        query_params.insert("browser_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("engine_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("round_trip_time".to_string(), "0".to_string());

        let mut headers = self.im_common_headers(&relation_path);
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

        let mut body_keys = body_params
            .keys()
            .map(|key| key.to_string())
            .collect::<Vec<_>>();
        body_keys.sort();
        log::info!(
            "Douyin IM request: path={} body_keys={} sec_user_ids_len={}",
            relation_path,
            body_keys.join(","),
            body_params
                .get("sec_user_ids")
                .map(|value| value.len())
                .unwrap_or_default()
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
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let response = response.json::<serde_json::Value>().await?;
        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("请求失败");
            return Err(anyhow!("IM接口请求失败: {}", status_msg));
        }

        Ok(response)
    }

    async fn request_im_get(
        &self,
        url: &str,
        endpoint_params: HashMap<&str, String>,
    ) -> Result<serde_json::Value> {
        let relation_path = url::Url::parse(url)
            .map(|parsed| parsed.path().to_string())
            .unwrap_or_default();
        let mut query_params = crate::config::get_common_params();
        query_params.insert("update_version_code".to_string(), "170400".to_string());
        query_params.insert("version_code".to_string(), "170400".to_string());
        query_params.insert("version_name".to_string(), "17.4.0".to_string());
        query_params.insert("browser_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("engine_version".to_string(), "148.0.0.0".to_string());
        query_params.insert("round_trip_time".to_string(), "0".to_string());
        for (key, value) in endpoint_params {
            query_params.insert(key.to_string(), value);
        }

        let mut headers = self.im_common_headers(&relation_path);
        headers.remove("Content-Type");
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

        log::info!("Douyin IM GET request: path={}", relation_path);

        let mut req = self.client.get(url).query(&query_params);
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
                .unwrap_or("请求失败");
            return Err(anyhow!("IM接口请求失败: {}", status_msg));
        }

        Ok(response)
    }

    fn collect_spotlight_sec_user_ids(
        response: &serde_json::Value,
        include_all_users: bool,
        ids: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        fn push_id(user: &serde_json::Value, ids: &mut Vec<String>, seen: &mut HashSet<String>) {
            for key in ["sec_uid", "sec_user_id"] {
                if let Some(id) = user.get(key).and_then(|value| value.as_str()) {
                    let id = id.trim().to_string();
                    if !id.is_empty() && seen.insert(id.clone()) {
                        ids.push(id);
                        break;
                    }
                }
            }
        }

        if let Some(items) = response["followings"].as_array() {
            for item in items {
                let is_mutual = item["follow_status"].as_i64().unwrap_or_default() > 0
                    && item["follower_status"].as_i64().unwrap_or_default() > 0;
                if include_all_users || is_mutual {
                    push_id(item, ids, seen);
                }
            }
        }

        if let Some(items) = response["sorted_info"].as_array() {
            for item in items {
                if item["conv_type"].as_i64().unwrap_or_default() == 0 {
                    push_id(item, ids, seen);
                }
            }
        }

        if include_all_users {
            for key in [
                "mix_recent_share_day_sort",
                "mix_recent_share_users",
                "single_recent_share_users",
            ] {
                if let Some(items) = response[key].as_array() {
                    for item in items {
                        push_id(item, ids, seen);
                    }
                }
            }

            if let Some(items) = response["recent_share_users"]["data"].as_array() {
                for item in items {
                    push_id(item, ids, seen);
                }
            }
        }
    }

    fn collect_sec_uid_records(value: &serde_json::Value) -> Vec<serde_json::Value> {
        fn visit(
            item: &serde_json::Value,
            records: &mut Vec<serde_json::Value>,
            seen: &mut HashSet<String>,
        ) {
            match item {
                serde_json::Value::Array(items) => {
                    for child in items {
                        visit(child, records, seen);
                    }
                }
                serde_json::Value::Object(object) => {
                    let sec_uid = object
                        .get("sec_uid")
                        .or_else(|| object.get("sec_user_id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if !sec_uid.is_empty() && seen.insert(sec_uid) {
                        records.push(item.clone());
                    }
                    for child in object.values() {
                        if child.is_object() || child.is_array() {
                            visit(child, records, seen);
                        }
                    }
                }
                _ => {}
            }
        }

        let mut records = Vec::new();
        let mut seen = HashSet::new();
        visit(value, &mut records, &mut seen);
        records
    }

    fn share_sorted_sec_uids(response: &serde_json::Value, limit: usize) -> Vec<String> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        if let Some(items) = response["sorted_info"].as_array() {
            for item in items {
                if item["conv_type"].as_i64().unwrap_or_default() != 0 {
                    continue;
                }
                let sec_uid = Self::share_friend_sec_uid(item);
                if !sec_uid.is_empty() && seen.insert(sec_uid.clone()) {
                    ids.push(sec_uid);
                }
                if ids.len() >= limit {
                    break;
                }
            }
        }
        ids
    }

    fn first_url_value(value: Option<&serde_json::Value>) -> String {
        let Some(value) = value else {
            return String::new();
        };
        if let Some(text) = value.as_str() {
            return text.trim().to_string();
        }
        if let Some(items) = value.as_array() {
            for item in items {
                let url = Self::first_url_value(Some(item));
                if !url.is_empty() {
                    return url;
                }
            }
        }
        if let Some(object) = value.as_object() {
            if let Some(url_list) = object.get("url_list") {
                let url = Self::first_url_value(Some(url_list));
                if !url.is_empty() {
                    return url;
                }
            }
            for key in ["url", "uri", "src", "download_url"] {
                let url = Self::first_url_value(object.get(key));
                if !url.is_empty() {
                    return url;
                }
            }
        }
        String::new()
    }

    fn media_uri_from_url(url: &str) -> String {
        let text = url.trim();
        if text.is_empty() {
            return String::new();
        }
        let mut path = url::Url::parse(text)
            .ok()
            .map(|parsed| parsed.path().trim_start_matches('/').to_string())
            .unwrap_or_else(|| {
                text.split('?')
                    .next()
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_string()
            });
        if let Ok(decoded) = urlencoding::decode(&path) {
            path = decoded.into_owned();
        }
        if let Some(stripped) = path.strip_prefix("aweme/") {
            path = stripped.to_string();
        }
        if let Some(stripped) = path.strip_prefix("img/") {
            path = stripped.to_string();
        }
        if let Some((prefix, _)) = path.split_once('~') {
            path = prefix.to_string();
        }
        for suffix in [".webp", ".jpeg", ".jpg", ".png"] {
            if let Some(stripped) = path.strip_suffix(suffix) {
                path = stripped.to_string();
                break;
            }
        }
        path
    }

    fn share_friend_sec_uid(item: &serde_json::Value) -> String {
        item.get("sec_uid")
            .and_then(|value| value.as_str())
            .or_else(|| item.get("sec_user_id").and_then(|value| value.as_str()))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn normalize_share_friends(
        response: &serde_json::Value,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        let mut users_by_sec_uid: HashMap<String, serde_json::Value> = HashMap::new();
        let mut recent_meta: HashMap<String, serde_json::Map<String, serde_json::Value>> =
            HashMap::new();
        let mut order = Vec::new();
        let mut seen_order = HashSet::new();

        let mut remember_order = |sec_uid: &str| {
            let sec_uid = sec_uid.trim();
            if !sec_uid.is_empty() && seen_order.insert(sec_uid.to_string()) {
                order.push(sec_uid.to_string());
            }
        };

        if let Some(items) = response["followings"].as_array() {
            for item in items {
                let sec_uid = Self::share_friend_sec_uid(item);
                if sec_uid.is_empty() {
                    continue;
                }
                users_by_sec_uid.insert(sec_uid.clone(), item.clone());
                remember_order(&sec_uid);
            }
        }

        for key in [
            "mix_recent_share_day_sort",
            "mix_recent_share_users",
            "single_recent_share_users",
        ] {
            if let Some(items) = response[key].as_array() {
                for item in items {
                    let sec_uid = Self::share_friend_sec_uid(item);
                    if sec_uid.is_empty() {
                        continue;
                    }
                    let meta = recent_meta.entry(sec_uid).or_default();
                    meta.insert("is_recent_share".to_string(), serde_json::json!(true));
                    if let Some(value) = item.get("conv_id").and_then(|value| value.as_str()) {
                        meta.insert("conv_id".to_string(), serde_json::json!(value));
                    }
                    if let Some(value) = item.get("conv_type").and_then(|value| value.as_i64()) {
                        meta.insert("conv_type".to_string(), serde_json::json!(value));
                    }
                    if let Some(value) = item.get("share_day_cnt").and_then(|value| value.as_i64())
                    {
                        meta.insert("share_day_count".to_string(), serde_json::json!(value));
                    }
                    let timestamp = item
                        .get("last_share_timestamp")
                        .and_then(|value| value.as_i64())
                        .or_else(|| item.get("timestamp").and_then(|value| value.as_i64()));
                    if let Some(value) = timestamp {
                        meta.insert("last_share_timestamp".to_string(), serde_json::json!(value));
                    }
                }
            }
        }

        let mut sorted_order = Vec::new();
        let mut sorted_seen = HashSet::new();
        if let Some(items) = response["sorted_info"].as_array() {
            for item in items {
                if item["conv_type"].as_i64().unwrap_or_default() != 0 {
                    continue;
                }
                let sec_uid = Self::share_friend_sec_uid(item);
                if !sec_uid.is_empty() && sorted_seen.insert(sec_uid.clone()) {
                    sorted_order.push(sec_uid);
                }
            }
        }

        let mut ordered_ids: Vec<String> = sorted_order
            .into_iter()
            .filter(|sec_uid| users_by_sec_uid.contains_key(sec_uid))
            .collect();
        let ordered_seen: HashSet<String> = ordered_ids.iter().cloned().collect();
        ordered_ids.extend(order.into_iter().filter(|sec_uid| {
            users_by_sec_uid.contains_key(sec_uid) && !ordered_seen.contains(sec_uid)
        }));

        let mut friends = Vec::new();
        let mut seen = HashSet::new();
        for sec_uid in ordered_ids {
            if !seen.insert(sec_uid.clone()) {
                continue;
            }
            let Some(user) = users_by_sec_uid.get(&sec_uid) else {
                continue;
            };
            let nickname = user
                .get("nickname")
                .and_then(|value| value.as_str())
                .or_else(|| user.get("remark_name").and_then(|value| value.as_str()))
                .or_else(|| user.get("unique_id").and_then(|value| value.as_str()))
                .or_else(|| user.get("short_id").and_then(|value| value.as_str()))
                .unwrap_or_default()
                .trim()
                .to_string();
            if nickname.is_empty() {
                continue;
            }

            let mut friend = serde_json::Map::new();
            let avatar_thumb = {
                let primary = Self::first_url_value(user.get("avatar_thumb"));
                if primary.is_empty() {
                    Self::first_url_value(user.get("avatar_small"))
                } else {
                    primary
                }
            };
            let avatar_medium = {
                let primary = Self::first_url_value(user.get("avatar_medium"));
                if !primary.is_empty() {
                    primary
                } else {
                    let secondary = Self::first_url_value(user.get("avatar_168x168"));
                    if secondary.is_empty() {
                        Self::first_url_value(user.get("avatar_small"))
                    } else {
                        secondary
                    }
                }
            };
            friend.insert(
                "uid".to_string(),
                serde_json::json!(user
                    .get("uid")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()),
            );
            friend.insert("sec_uid".to_string(), serde_json::json!(sec_uid.clone()));
            friend.insert("nickname".to_string(), serde_json::json!(nickname));
            friend.insert("avatar_thumb".to_string(), serde_json::json!(avatar_thumb));
            friend.insert(
                "avatar_medium".to_string(),
                serde_json::json!(avatar_medium),
            );
            friend.insert(
                "unique_id".to_string(),
                serde_json::json!(user
                    .get("unique_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()),
            );
            friend.insert(
                "short_id".to_string(),
                serde_json::json!(user
                    .get("short_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()),
            );
            friend.insert(
                "follow_status".to_string(),
                serde_json::json!(user
                    .get("follow_status")
                    .and_then(|value| value.as_i64())
                    .unwrap_or_default()),
            );
            friend.insert(
                "follower_status".to_string(),
                serde_json::json!(user
                    .get("follower_status")
                    .and_then(|value| value.as_i64())
                    .unwrap_or_default()),
            );
            if let Some(meta) = recent_meta.get(&sec_uid) {
                for (key, value) in meta {
                    friend.insert(key.clone(), value.clone());
                }
            }
            friends.push(serde_json::Value::Object(friend));
            if friends.len() >= limit {
                break;
            }
        }

        friends
    }

    pub async fn get_im_share_friends(&self, limit: usize) -> Result<serde_json::Value> {
        let safe_limit = limit.clamp(1, 100);
        let mut params = HashMap::new();
        params.insert("count", safe_limit.to_string());
        params.insert("source", "coldup".to_string());
        params.insert(
            "max_time",
            chrono::Utc::now().timestamp_millis().to_string(),
        );
        params.insert("min_time", "0".to_string());
        params.insert("need_remove_share_panel", "true".to_string());
        params.insert("need_sorted_info", "true".to_string());
        params.insert("with_fstatus", "1".to_string());

        let mut response = self
            .request_im_get(
                "https://www-hj.douyin.com/aweme/v1/web/im/spotlight/relation/",
                params,
            )
            .await?;
        let mut known_sec_uids = HashSet::new();
        if let Some(items) = response["followings"].as_array() {
            for item in items {
                let sec_uid = Self::share_friend_sec_uid(item);
                if !sec_uid.is_empty() {
                    known_sec_uids.insert(sec_uid);
                }
            }
        }
        let missing_sec_uids = Self::share_sorted_sec_uids(&response, safe_limit)
            .into_iter()
            .filter(|sec_uid| !known_sec_uids.contains(sec_uid))
            .collect::<Vec<_>>();
        if !missing_sec_uids.is_empty() {
            for chunk in missing_sec_uids.chunks(20) {
                let chunk = chunk.to_vec();
                let Ok(user_info) = self.get_im_user_info(&chunk).await else {
                    continue;
                };
                let records = Self::collect_sec_uid_records(&user_info);
                let Some(object) = response.as_object_mut() else {
                    continue;
                };
                let followings = object
                    .entry("followings")
                    .or_insert_with(|| serde_json::json!([]));
                let Some(followings) = followings.as_array_mut() else {
                    continue;
                };
                for record in records {
                    let sec_uid = Self::share_friend_sec_uid(&record);
                    if !sec_uid.is_empty() && known_sec_uids.insert(sec_uid) {
                        followings.push(record);
                    }
                }
            }
        }
        let friends = Self::normalize_share_friends(&response, safe_limit);
        Ok(serde_json::json!({
            "success": true,
            "message": "获取分享好友成功",
            "friends": friends,
            "count": friends.len(),
            "has_more": response.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false)
        }))
    }

    async fn get_im_identity_security_token(&self) -> Result<(String, String)> {
        let path = "/passport/safe/get_identity_security_token/";
        let trace_id = Uuid::new_v4()
            .to_string()
            .replace('-', "")
            .chars()
            .take(8)
            .collect::<String>();
        let mut query_params = crate::config::get_common_params();
        query_params.insert("passport_jssdk_version".to_string(), "4.2.3".to_string());
        query_params.insert("passport_jssdk_type".to_string(), "lite".to_string());
        query_params.insert("is_from_ttaccountsdk".to_string(), "1".to_string());
        query_params.insert("aid".to_string(), "6383".to_string());
        query_params.insert("language".to_string(), "zh".to_string());
        query_params.insert("scene".to_string(), "web_im".to_string());
        query_params.insert("auto_retry_req".to_string(), "0".to_string());
        query_params.insert("skip_verify".to_string(), "false".to_string());
        query_params.insert("identity_token_force_get_tag".to_string(), "0".to_string());
        query_params.insert("biz_trace_id".to_string(), trace_id.clone());
        query_params.insert("id_token_version".to_string(), "1.2.10".to_string());

        let mut headers = crate::config::get_common_headers(&self.config.cookie);
        headers.insert(
            "accept".to_string(),
            "application/json, text/javascript".to_string(),
        );
        headers.insert("referer".to_string(), "https://www.douyin.com/".to_string());
        headers.insert("priority".to_string(), "u=1, i".to_string());
        headers.insert("sec-fetch-dest".to_string(), "empty".to_string());
        headers.insert("sec-fetch-mode".to_string(), "cors".to_string());
        headers.insert("sec-fetch-site".to_string(), "same-origin".to_string());
        headers.insert("x-tt-passport-trace-id".to_string(), trace_id);
        let cookie_dict = Self::cookies_to_dict(&self.config.cookie);
        if let Some(csrf) = cookie_dict
            .get("passport_csrf_token")
            .or_else(|| cookie_dict.get("passport_csrf_token_default"))
        {
            headers.insert("x-tt-passport-csrf-token".to_string(), csrf.clone());
        }
        headers.extend(self.relation_ticket_guard_headers(path));
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
            .get(format!("https://www.douyin.com{path}"))
            .query(&query_params);
        for (key, value) in &headers {
            req = req.header(key, value);
        }
        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "获取分享安全凭证失败（HTTP {}）",
                response.status()
            ));
        }
        let payload = response.json::<serde_json::Value>().await?;
        let message = payload
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !message.is_empty() && message != "success" && message != "ok" {
            return Err(anyhow!(
                "{}",
                payload
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("获取分享安全凭证失败")
            ));
        }
        let data = payload
            .get("data")
            .and_then(|value| value.as_object())
            .ok_or_else(|| anyhow!("获取分享安全凭证失败：响应缺少 data"))?;
        let token = data
            .get("identity_security_token")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        let device_id = data
            .get("device_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if token.is_empty() || device_id.is_empty() {
            return Err(anyhow!("获取分享安全凭证失败：缺少 token 或 device_id"));
        }
        Ok((token, device_id))
    }

    pub async fn get_im_user_info(&self, sec_user_ids: &[String]) -> Result<serde_json::Value> {
        let ids = sec_user_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(anyhow!("好友ID不能为空"));
        }

        let mut body_params = HashMap::new();
        body_params.insert("sec_user_ids", serde_json::to_string(&ids)?);

        self.request_im_post(
            "https://www-hj.douyin.com/aweme/v1/web/im/user/info/",
            body_params,
        )
        .await
    }

    pub async fn get_im_user_active_status(
        &self,
        sec_user_ids: &[String],
        conv_ids: &[String],
    ) -> Result<serde_json::Value> {
        let ids = sec_user_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(anyhow!("好友ID不能为空"));
        }
        let conv_ids = conv_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();

        let mut body_params = HashMap::new();
        body_params.insert("conv_ids", serde_json::to_string(&conv_ids)?);
        body_params.insert("sec_user_ids", serde_json::to_string(&ids)?);
        body_params.insert("source", "heartbeat".to_string());

        self.request_im_post(
            "https://www-hj.douyin.com/aweme/v1/web/im/user/active/status/",
            body_params,
        )
        .await
    }

    pub async fn create_im_conversation(&self, to_user_id: &str) -> Result<serde_json::Value> {
        let signer = self.im_proto_signer()?;
        let current_user = self.get_current_user().await?;
        let to_uid = to_user_id
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow!("缺少可用的数字 uid，无法创建私信会话"))?;
        let my_uid = current_user
            .uid
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow!("缺少可用的数字 uid，无法创建私信会话"))?;
        if to_uid == 0 || my_uid == 0 {
            return Err(anyhow!("缺少可用的数字 uid，无法创建私信会话"));
        }

        let sign_data = format!("avatar_url=&idempotent_id=&name=&participants={to_uid},{my_uid}");
        let request_sign = Self::ecdsa_request_sign(&sign_data, &signer.private_key)?;
        let body = im_proto::build_create_conversation_body(to_uid, my_uid);
        let payload = self.build_im_proto_request(
            609,
            &body,
            &request_sign,
            "1.1.3",
            "5fa6ff1:Detached: 5fa6ff1111fd53aafc4c753505d3c93daad74d27",
            None,
        )?;
        let response = self
            .post_im_proto(
                "https://imapi.douyin.com/v2/conversation/create",
                payload,
                false,
            )
            .await?;
        let conversation = im_proto::first_conversation(&response)
            .ok_or_else(|| anyhow!("创建会话成功但未返回会话信息"))?;
        Ok(serde_json::json!({
            "conversation_id": conversation.conversation_id,
            "conversation_short_id": conversation.conversation_short_id,
            "conversation_type": conversation.conversation_type,
            "ticket": conversation.ticket,
            "raw": response,
        }))
    }

    pub async fn send_im_text_message(
        &self,
        to_user_id: &str,
        content: &str,
    ) -> Result<serde_json::Value> {
        let message = content.trim();
        if message.is_empty() {
            return Err(anyhow!("消息内容不能为空"));
        }
        let msg_content = serde_json::json!({
            "mention_users": [],
            "aweType": 700,
            "richTextInfos": [],
            "text": message,
        })
        .to_string();
        self.send_im_content_message(to_user_id, msg_content, 7)
            .await
    }

    pub async fn send_im_video_share_message(
        &self,
        to_user_id: &str,
        video: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let video_object = video
            .as_object()
            .ok_or_else(|| anyhow!("缺少视频信息，无法分享"))?;
        let aweme_id = video_object
            .get("aweme_id")
            .or_else(|| video_object.get("itemId"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if aweme_id.is_empty() {
            return Err(anyhow!("缺少作品 ID，无法分享"));
        }
        let author = video_object
            .get("author")
            .and_then(|value| value.as_object());
        let video_data = video_object
            .get("video")
            .and_then(|value| value.as_object());
        let cover = Self::first_url_value(
            video_object
                .get("cover_url")
                .or_else(|| video_object.get("cover"))
                .or_else(|| video_data.and_then(|item| item.get("cover")))
                .or_else(|| video_data.and_then(|item| item.get("origin_cover")))
                .or_else(|| video_data.and_then(|item| item.get("dynamic_cover"))),
        );
        let author_avatar = Self::first_url_value(
            author
                .and_then(|item| item.get("avatar_thumb"))
                .or_else(|| author.and_then(|item| item.get("avatar_medium")))
                .or_else(|| author.and_then(|item| item.get("avatar_larger"))),
        );
        let cover_uri = Self::media_uri_from_url(&cover);
        let author_avatar_uri = Self::media_uri_from_url(&author_avatar);
        let cover_width = video_data
            .and_then(|item| item.get("width"))
            .or_else(|| video_object.get("width"))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.parse::<i64>().ok())
            })
            .unwrap_or_default();
        let cover_height = video_data
            .and_then(|item| item.get("height"))
            .or_else(|| video_object.get("height"))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.parse::<i64>().ok())
            })
            .unwrap_or_default();
        let author_uid = author
            .and_then(|item| item.get("uid"))
            .or_else(|| video_object.get("uid"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let content = serde_json::json!({
            "aweType": 800,
            "content_title": video_object
                .get("desc")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&aweme_id),
            "cover_height": cover_height,
            "cover_width": cover_width,
            "itemId": aweme_id,
            "cover_url": {
                "url_list": if cover.is_empty() { Vec::<String>::new() } else { vec![cover] },
                "uri": cover_uri,
            },
            "content_thumb": {
                "url_list": if author_avatar.is_empty() { Vec::<String>::new() } else { vec![author_avatar] },
                "uri": author_avatar_uri,
            },
            "uid": author_uid,
        })
        .to_string();
        let (token, device_id) = self.get_im_identity_security_token().await?;
        let extra_headers = HashMap::from([
            (
                "identity_security_token".to_string(),
                serde_json::json!({ "token": token }).to_string(),
            ),
            ("identity_security_device_id".to_string(), device_id),
            ("identity_security_aid".to_string(), "6383".to_string()),
        ]);
        self.send_im_content_message_with_headers(to_user_id, content, 8, Some(&extra_headers))
            .await
    }

    async fn get_im_image_upload_config(&self) -> Result<serde_json::Value> {
        let mut query_params = crate::config::get_common_params();
        query_params.extend(HashMap::from([
            ("update_version_code".to_string(), "170400".to_string()),
            ("version_code".to_string(), "170400".to_string()),
            ("version_name".to_string(), "17.4.0".to_string()),
            ("browser_name".to_string(), "Chrome".to_string()),
            ("browser_version".to_string(), "148.0.0.0".to_string()),
            ("engine_version".to_string(), "148.0.0.0".to_string()),
            ("round_trip_time".to_string(), "150".to_string()),
        ]));
        let mut headers = HashMap::from([
            ("User-Agent".to_string(), "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".to_string()),
            ("Cookie".to_string(), self.config.cookie.clone()),
            ("Referer".to_string(), "https://www.douyin.com/jingxuan".to_string()),
            ("sec-fetch-site".to_string(), "same-origin".to_string()),
            ("sec-ch-ua".to_string(), "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\"".to_string()),
            ("accept".to_string(), "application/json, text/plain, */*".to_string()),
        ]);
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
            .get("https://www.douyin.com/aweme/v1/web/im/upload/config/v2")
            .query(&query_params);
        for (key, value) in &headers {
            req = req.header(key, value);
        }
        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "获取图片上传配置失败（HTTP {}）",
                response.status()
            ));
        }
        let value = response.json::<serde_json::Value>().await?;
        let config = value
            .get("public_image_config_v2")
            .or_else(|| value.get("public_image_config"))
            .cloned()
            .ok_or_else(|| anyhow!("抖音未返回图片上传配置"))?;
        for key in [
            "access_key_id",
            "secret_access_key",
            "session_token",
            "space_name",
        ] {
            if config
                .get(key)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .is_empty()
            {
                return Err(anyhow!("抖音未返回完整图片上传配置，请刷新 Cookie 后重试"));
            }
        }
        Ok(config)
    }

    async fn apply_im_image_upload(
        &self,
        config: &serde_json::Value,
        file_size: usize,
    ) -> Result<serde_json::Value> {
        let access_key_id = config["access_key_id"].as_str().unwrap_or_default();
        let secret_access_key = config["secret_access_key"].as_str().unwrap_or_default();
        let session_token = config["session_token"].as_str().unwrap_or_default();
        let space_name = config["space_name"].as_str().unwrap_or_default();
        let random: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(10)
            .map(char::from)
            .collect();
        let query_params = BTreeMap::from([
            ("Action".to_string(), "ApplyUploadInner".to_string()),
            ("Version".to_string(), "2020-11-19".to_string()),
            ("SpaceName".to_string(), space_name.to_string()),
            ("FileType".to_string(), "image".to_string()),
            ("IsInner".to_string(), "1".to_string()),
            ("NeedFallback".to_string(), "true".to_string()),
            ("FileSize".to_string(), file_size.to_string()),
            ("s".to_string(), format!("r{}", random.to_ascii_lowercase())),
        ]);
        let empty_hash = format!("{:x}", Sha256::digest([]));
        let (query, auth_headers) = Self::aws_vod_auth_headers(
            "GET",
            &query_params,
            access_key_id,
            secret_access_key,
            session_token,
            &empty_hash,
            BTreeMap::new(),
        )?;
        let mut req = self
            .client
            .get(format!("https://vod.bytedanceapi.com/?{query}"))
            .header("accept", "*/*")
            .header("origin", "https://www.douyin.com")
            .header("referer", "https://www.douyin.com/")
            .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36");
        for (key, value) in auth_headers {
            req = req.header(key, value);
        }
        let response = req.send().await?;
        let status = response.status();
        let value = response.json::<serde_json::Value>().await?;
        if !status.is_success() || value.pointer("/ResponseMetadata/Error").is_some() {
            return Err(anyhow!("申请图片上传失败"));
        }
        let upload_address = value
            .pointer("/Result/UploadAddress")
            .cloned()
            .ok_or_else(|| anyhow!("申请图片上传成功但返回缺少上传地址"))?;
        if upload_address
            .get("SessionKey")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .is_empty()
        {
            return Err(anyhow!("申请图片上传成功但返回缺少 SessionKey"));
        }
        Ok(upload_address)
    }

    async fn upload_im_image_bytes(
        &self,
        upload_address: &serde_json::Value,
        image_bytes: Vec<u8>,
        crc32: &str,
    ) -> Result<()> {
        let store_info = upload_address
            .get("StoreInfos")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .ok_or_else(|| anyhow!("图片上传地址不完整"))?;
        let host = upload_address
            .get("UploadHosts")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let store_uri = store_info
            .get("StoreUri")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let auth = store_info
            .get("Auth")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if host.is_empty() || store_uri.is_empty() || auth.is_empty() {
            return Err(anyhow!("图片上传地址不完整"));
        }
        let mut req = self
            .client
            .post(format!("https://{host}/upload/v1/{store_uri}"))
            .header("accept", "*/*")
            .header("authorization", auth)
            .header("content-crc32", crc32)
            .header("content-disposition", "attachment; filename=\"undefined\"")
            .header("content-type", "application/octet-stream")
            .header("origin", "https://www.douyin.com")
            .header("referer", "https://www.douyin.com/")
            .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36");
        if let Some(user_id) = store_info
            .pointer("/StorageHeader/USER_ID")
            .and_then(|value| value.as_str())
        {
            req = req.header("x-storage-u", user_id);
        }
        let response = req.body(image_bytes).send().await?;
        let status = response.status();
        let value = response.json::<serde_json::Value>().await?;
        let code_ok = matches!(value.get("code"), Some(serde_json::Value::Number(n)) if n.as_i64() == Some(2000))
            || matches!(value.get("code"), Some(serde_json::Value::String(s)) if s == "2000");
        if !status.is_success() || !code_ok {
            return Err(anyhow!("上传图片文件失败"));
        }
        Ok(())
    }

    async fn commit_im_image_upload(
        &self,
        config: &serde_json::Value,
        session_key: &str,
    ) -> Result<serde_json::Value> {
        let access_key_id = config["access_key_id"].as_str().unwrap_or_default();
        let secret_access_key = config["secret_access_key"].as_str().unwrap_or_default();
        let session_token = config["session_token"].as_str().unwrap_or_default();
        let space_name = config["space_name"].as_str().unwrap_or_default();
        let query_params = BTreeMap::from([
            ("Action".to_string(), "CommitUploadInner".to_string()),
            ("Version".to_string(), "2020-11-19".to_string()),
            ("SpaceName".to_string(), space_name.to_string()),
        ]);
        let body = serde_json::json!({
            "SessionKey": session_key,
            "Functions": [{
                "name": "Encryption",
                "input": {
                    "Config": { "copies": "cipher_v2" },
                    "PolicyParams": { "policy-set": "check,thumb,medium,large" }
                }
            }]
        })
        .to_string()
        .into_bytes();
        let body_hash = format!("{:x}", Sha256::digest(&body));
        let (query, auth_headers) = Self::aws_vod_auth_headers(
            "POST",
            &query_params,
            access_key_id,
            secret_access_key,
            session_token,
            &body_hash,
            BTreeMap::from([("x-amz-content-sha256".to_string(), body_hash.clone())]),
        )?;
        let mut req = self
            .client
            .post(format!("https://vod.bytedanceapi.com/?{query}"))
            .header("accept", "*/*")
            .header("content-type", "text/plain;charset=UTF-8")
            .header("origin", "https://www.douyin.com")
            .header("referer", "https://www.douyin.com/")
            .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36");
        for (key, value) in auth_headers {
            req = req.header(key, value);
        }
        let response = req.body(body).send().await?;
        let status = response.status();
        let value = response.json::<serde_json::Value>().await?;
        if !status.is_success() || value.pointer("/ResponseMetadata/Error").is_some() {
            return Err(anyhow!("提交图片上传失败"));
        }
        value
            .pointer("/Result/Results/0")
            .cloned()
            .ok_or_else(|| anyhow!("提交图片上传成功但未返回资源信息"))
    }

    pub async fn send_im_image_message(
        &self,
        to_user_id: &str,
        image_data_url: &str,
        width: i64,
        height: i64,
        _file_name: &str,
        _mime_type: &str,
    ) -> Result<serde_json::Value> {
        let trimmed = image_data_url.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("图片内容不能为空"));
        }
        let inline_pic = trimmed
            .split_once(',')
            .map(|(_, payload)| payload)
            .unwrap_or(trimmed)
            .replace(['\r', '\n', ' '], "");
        if inline_pic.is_empty() {
            return Err(anyhow!("图片内容不能为空"));
        }
        let image_bytes = base64::engine::general_purpose::STANDARD
            .decode(inline_pic.as_bytes())
            .map_err(|_| anyhow!("图片数据解析失败"))?;
        if image_bytes.is_empty() {
            return Err(anyhow!("图片内容不能为空"));
        }
        let source_md5 = format!("{:x}", md5::compute(&image_bytes));
        let crc32 = crc32_hex(&image_bytes);
        let data_size = image_bytes.len();
        let config = self.get_im_image_upload_config().await?;
        let upload_address = self.apply_im_image_upload(&config, data_size).await?;
        self.upload_im_image_bytes(&upload_address, image_bytes, &crc32)
            .await?;
        let session_key = upload_address
            .get("SessionKey")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let commit_result = self.commit_im_image_upload(&config, session_key).await?;
        let encryption = commit_result
            .get("Encryption")
            .ok_or_else(|| anyhow!("提交图片上传成功但未返回加密资源信息"))?;
        let oid = encryption
            .get("Uri")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let skey = encryption
            .get("SecretKey")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if oid.is_empty() || skey.is_empty() {
            return Err(anyhow!("图片上传完成但缺少资源 oid/skey"));
        }
        let extra = encryption.get("Extra").and_then(|value| value.as_object());
        let cover_width = extra
            .and_then(|extra| extra.get("img_width"))
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(width.max(0));
        let cover_height = extra
            .and_then(|extra| extra.get("img_height"))
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(height.max(0));
        let uploaded_size = extra
            .and_then(|extra| extra.get("img_size"))
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(data_size);
        let sent_md5 = encryption
            .get("SourceMd5")
            .and_then(|value| value.as_str())
            .unwrap_or(&source_md5);
        let msg_content = serde_json::json!({
            "resource_url": {
                "oid": oid,
                "skey": skey,
                "data_size": uploaded_size,
                "md5": sent_md5,
            },
            "cover_height": cover_height,
            "cover_width": cover_width,
            "check_pics": [],
            "md5": sent_md5,
            "from_gallery": 1,
            "aweType": 2702,
        })
        .to_string();
        self.send_im_content_message(to_user_id, msg_content, 27)
            .await
    }

    async fn send_im_content_message(
        &self,
        to_user_id: &str,
        msg_content: String,
        message_type: i64,
    ) -> Result<serde_json::Value> {
        self.send_im_content_message_with_headers(to_user_id, msg_content, message_type, None)
            .await
    }

    async fn send_im_content_message_with_headers(
        &self,
        to_user_id: &str,
        msg_content: String,
        message_type: i64,
        extra_headers: Option<&HashMap<String, String>>,
    ) -> Result<serde_json::Value> {
        let conversation = self.create_im_conversation(to_user_id).await?;
        let client_message_id = Uuid::new_v4().to_string();
        let conversation_id = conversation
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let conversation_short_id = conversation
            .get("conversation_short_id")
            .and_then(|value| value.as_i64())
            .unwrap_or_default();
        let ticket = conversation
            .get("ticket")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let body = im_proto::build_send_message_body(
            conversation_id,
            conversation_short_id,
            ticket,
            &msg_content,
            &client_message_id,
            now_ms,
            message_type,
        );
        let payload = self.build_im_pc_proto_request_with_headers(100, &body, extra_headers)?;
        let response = self
            .post_im_proto("https://imapi.douyin.com/v1/message/send", payload, true)
            .await?;
        let Some(sent_message) = im_proto::sent_message(&response) else {
            return Ok(serde_json::json!({
                "message": "发送请求已提交，等待私信通道确认",
                "client_message_id": client_message_id,
                "pending_ack": true,
                "conversation": conversation,
                "raw": response,
            }));
        };
        Ok(serde_json::json!({
            "message": "发送成功",
            "client_message_id": client_message_id,
            "message_id": sent_message.server_message_id,
            "conversation_id": sent_message.conversation_id,
            "conversation_short_id": sent_message.conversation_short_id,
            "conversation_type": sent_message.conversation_type,
            "conversation": conversation,
            "raw": response,
        }))
    }

    fn normalize_im_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .filter_map(|item| {
                let object = item.as_object()?;
                let raw_content = object
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let text = serde_json::from_str::<serde_json::Value>(&raw_content)
                    .ok()
                    .and_then(|parsed| {
                        parsed
                            .get("text")
                            .or_else(|| parsed.get("tips"))
                            .or_else(|| parsed.get("hint_text"))
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string)
                    })
                    .unwrap_or_else(|| raw_content.clone());
                let ext = object.get("ext").and_then(|value| value.as_object());
                let mut create_time = object
                    .get("create_time")
                    .and_then(|value| value.as_i64())
                    .unwrap_or_default();
                if create_time == 0 {
                    create_time = ext
                        .and_then(|ext| {
                            ext.get("s:server_message_create_time")
                                .or_else(|| ext.get("server_message_create_time"))
                        })
                        .and_then(|value| {
                            value
                                .as_i64()
                                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                        })
                        .unwrap_or_default();
                }
                Some(serde_json::json!({
                    "conversation_id": object.get("conversation_id").cloned().unwrap_or_default(),
                    "conversation_short_id": object.get("conversation_short_id").cloned().unwrap_or_default(),
                    "conversation_type": object.get("conversation_type").cloned().unwrap_or_default(),
                    "server_message_id": object.get("server_message_id").cloned().unwrap_or_default(),
                    "index_in_conversation": object.get("index_in_conversation").cloned().unwrap_or_default(),
                    "sender_uid": object.get("sender").cloned().unwrap_or_default().to_string().trim_matches('"').to_string(),
                    "content": text,
                    "raw_content": raw_content,
                    "message_type": object.get("message_type").cloned().unwrap_or_default(),
                    "create_time": create_time,
                }))
            })
            .collect()
    }

    async fn get_im_recent_user_messages(&self, cursor: i64) -> Result<serde_json::Value> {
        self.im_proto_signer()?;
        let body = im_proto::build_get_user_message_body(cursor.max(0));
        let payload = self.build_im_pc_proto_request(128, &body)?;
        let response = self
            .post_im_proto(
                "https://imapi.douyin.com/v1/message/get_user_message",
                payload,
                false,
            )
            .await?;
        let body = response
            .pointer("/body/get_user_message_body")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(serde_json::json!({
            "message": "获取历史消息成功",
            "messages": Self::normalize_im_messages(&messages),
            "next_cursor": body.get("next_cursor").cloned().unwrap_or_default(),
            "has_more": body.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false),
        }))
    }

    fn filter_im_history_for_user(result: serde_json::Value, uid: &str) -> serde_json::Value {
        let uid = uid.trim();
        if uid.is_empty() {
            return result;
        }
        let messages = result
            .get("messages")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("sender_uid")
                            .and_then(|value| value.as_str())
                            .map(|sender| sender == uid)
                            .unwrap_or(false)
                            || item
                                .get("conversation_id")
                                .and_then(|value| value.as_str())
                                .map(|conversation_id| conversation_id.contains(uid))
                                .unwrap_or(false)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        serde_json::json!({
            "message": result.get("message").cloned().unwrap_or_else(|| serde_json::json!("获取历史消息成功")),
            "messages": messages,
            "next_cursor": result.get("next_cursor").cloned().unwrap_or_default(),
            "has_more": result.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false),
        })
    }

    pub async fn get_im_history_messages(
        &self,
        cursor: i64,
        to_user_id: Option<&str>,
        conversation_id: Option<&str>,
        conversation_short_id: Option<i64>,
        conversation_type: i64,
    ) -> Result<serde_json::Value> {
        self.im_proto_signer()?;
        let mut created_conversation_for_user = false;
        let conversation = if let (Some(conversation_id), Some(short_id)) = (
            conversation_id.filter(|value| !value.trim().is_empty()),
            conversation_short_id.filter(|value| *value > 0),
        ) {
            Some(serde_json::json!({
                "conversation_id": conversation_id,
                "conversation_short_id": short_id,
                "conversation_type": if conversation_type > 0 { conversation_type } else { 1 },
            }))
        } else if let Some(uid) = to_user_id.filter(|value| !value.trim().is_empty()) {
            match self.create_im_conversation(uid).await {
                Ok(conversation) => {
                    created_conversation_for_user = true;
                    Some(conversation)
                }
                Err(error) => {
                    log::warn!(
                        "Douyin IM create conversation failed, falling back to recent history: uid={} error={}",
                        uid,
                        error
                    );
                    let recent = self.get_im_recent_user_messages(cursor).await?;
                    return Ok(Self::filter_im_history_for_user(recent, uid));
                }
            }
        } else {
            None
        };

        let Some(conversation) = conversation else {
            let recent = self.get_im_recent_user_messages(cursor).await?;
            return Ok(if let Some(uid) = to_user_id {
                Self::filter_im_history_for_user(recent, uid)
            } else {
                recent
            });
        };

        let conversation_id = conversation
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let conversation_short_id = conversation
            .get("conversation_short_id")
            .and_then(|value| value.as_i64())
            .unwrap_or_default();
        let conversation_type = conversation
            .get("conversation_type")
            .and_then(|value| value.as_i64())
            .unwrap_or(1);
        let body = im_proto::build_get_by_conversation_body(
            conversation_id,
            conversation_short_id,
            conversation_type,
            cursor.max(0),
            IM_HISTORY_PAGE_SIZE,
        );
        let payload = self.build_im_pc_proto_request(301, &body)?;
        let response = match self
            .post_im_proto(
                "https://imapi.douyin.com/v1/message/get_by_conversation",
                payload,
                false,
            )
            .await
        {
            Ok(response) => response,
            Err(error) if created_conversation_for_user => {
                if let Some(uid) = to_user_id {
                    log::warn!(
                        "Douyin IM conversation history failed, falling back to recent history: uid={} error={}",
                        uid,
                        error
                    );
                    let recent = self.get_im_recent_user_messages(cursor).await?;
                    return Ok(Self::filter_im_history_for_user(recent, uid));
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let body = response
            .pointer("/body/get_by_conversation_body")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let result = serde_json::json!({
            "message": "获取历史消息成功",
            "messages": Self::normalize_im_messages(&messages),
            "next_cursor": body.get("next_cursor").cloned().unwrap_or_default(),
            "has_more": body.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false),
            "conversation": {
                "conversation_id": conversation_id,
                "conversation_short_id": conversation_short_id,
                "conversation_type": conversation_type,
            },
        });

        let message_count = result
            .get("messages")
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or_default();
        if message_count == 0 && created_conversation_for_user {
            if let Some(uid) = to_user_id {
                let recent = self.get_im_recent_user_messages(cursor).await?;
                return Ok(Self::filter_im_history_for_user(recent, uid));
            }
        }

        Ok(result)
    }

    pub async fn get_im_spotlight_relation_sec_user_ids(
        &self,
        limit: usize,
        include_all_users: bool,
    ) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        let mut max_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();

        let page_limit = 1;
        for _ in 0..page_limit {
            let mut params = HashMap::new();
            params.insert("count", "100".to_string());
            params.insert("source", "coldup".to_string());
            params.insert("max_time", max_time.clone());
            params.insert("min_time", "0".to_string());
            params.insert("need_remove_share_panel", "true".to_string());
            params.insert("need_sorted_info", "true".to_string());
            params.insert("with_fstatus", "1".to_string());

            let response = self
                .request_im_get(
                    "https://www-hj.douyin.com/aweme/v1/web/im/spotlight/relation/",
                    params,
                )
                .await?;

            Self::collect_spotlight_sec_user_ids(&response, include_all_users, &mut ids, &mut seen);
            if ids.len() >= limit {
                ids.truncate(limit);
                return Ok(ids);
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

            if !has_more || next_max_time.is_empty() || next_max_time == max_time {
                break;
            }
            max_time = next_max_time;
        }

        Ok(ids)
    }

    pub async fn get_im_device_id(&self) -> Result<String> {
        let mut params = HashMap::new();
        params.insert("publish_video_strategy_type", "2".to_string());
        let headers = HashMap::from([(
            "Referer".to_string(),
            "https://www.douyin.com/discover".to_string(),
        )]);
        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/query/user",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;
        let device_id = response
            .get("id")
            .and_then(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .or_else(|| value.as_i64().map(|number| number.to_string()))
            })
            .unwrap_or_default()
            .trim()
            .to_string();
        if device_id.is_empty() {
            return Err(anyhow!("未获取到 IM device_id"));
        }
        Ok(device_id)
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
        {
            return Err(anyhow!("作品ID不能为空"));
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
        let mut query_keys = query_params.keys().cloned().collect::<Vec<_>>();
        query_keys.sort();
        let mut body_keys = body_params
            .keys()
            .map(|key| key.to_string())
            .collect::<Vec<_>>();
        body_keys.sort();
        log::info!(
            "Douyin {} relation update request: host={} path={} query_keys={} uid_present={} uid_prefix={} body_keys={} signer_present={} ticket_guard_cookie={} ticket_guard_header={} csrf_present={} dtrait_present={}",
            action_name,
            url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .unwrap_or_default(),
            relation_path,
            query_keys.join(","),
            query_params.contains_key("uid"),
            query_params
                .get("uid")
                .map(|value| value.chars().take(8).collect::<String>())
                .unwrap_or_default(),
            body_keys.join(","),
            self.config.relation_signer.is_some(),
            self.config.cookie.contains("bd_ticket_guard_client_data"),
            headers.contains_key("bd-ticket-guard-client-data"),
            headers.contains_key("x-secsdk-csrf-token"),
            headers.contains_key("x-tt-session-dtrait"),
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
                "Douyin {} relation update rejected: http_status={} headers={:?}",
                action_name,
                status,
                response.headers()
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
        log::info!(
            "Douyin {} relation update response: status_code={} status_msg={} full_response={}",
            action_name,
            response["status_code"].as_i64().unwrap_or(0),
            response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or(""),
            response
        );

        let status_code = response["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("请求失败");
            log::warn!(
                "Douyin {} relation update failed: code={} message={} response={}",
                action_name,
                status_code,
                status_msg,
                response
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
        log::info!(
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
    fn image_post_collects_live_photo_and_static_image_urls() {
        let client = DouyinClient::new(AppConfig::default()).expect("client");
        let post = json!({
            "aweme_id": "7341234567890123456",
            "desc": "mixed image post",
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
        assert_eq!(
            video.image_urls.as_ref().expect("images"),
            &vec!["https://example.com/image-large.jpeg".to_string()]
        );
    }
}
