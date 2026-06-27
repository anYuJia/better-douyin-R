//! IM 客户端逻辑

use anyhow::{anyhow, Result};
use base64::Engine;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::client::DouyinClient;
use super::im_proto;
use crate::config::get_user_agent;
use crate::sign;

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

impl DouyinClient {
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
        let cookie_dict = self.cookie_dict();
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
            let cookie_dict = self.cookie_dict();
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
        log::debug!(
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

        log::debug!("Douyin IM GET request: path={}", relation_path);

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
        let cookie_dict = self.cookie_dict();
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
                
                let mut text = String::new();
                
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw_content) {
                    if let Some(parsed_obj) = parsed.as_object() {
                        if parsed_obj.contains_key("command_type") || parsed_obj.get("command_type").and_then(|v| v.as_i64()) == Some(6) {
                            let mut is_system_command = true;
                            if let Some(ext_data) = parsed_obj.get("ext_data").and_then(|v| v.as_array()) {
                                for ext_item in ext_data {
                                    if let Some(ext_obj) = ext_item.as_object() {
                                        if ext_obj.get("key").and_then(|v| v.as_str()) == Some("a:consecutive_chat_data") {
                                            text = "🔥 连续聊天火花已亮起".to_string();
                                            is_system_command = false;
                                            if let Some(val_str) = ext_obj.get("value").and_then(|v| v.as_str()) {
                                                if let Ok(val_json) = serde_json::from_str::<serde_json::Value>(val_str) {
                                                    if let Some(count_info) = val_json.get("consecutive_count_info") {
                                                        let count = count_info.get("consecutive_count").and_then(|v| v.as_i64()).unwrap_or(1);
                                                        text = format!("🔥 连续聊天火花已亮起（第 {} 天）", count);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if is_system_command {
                                return None; // Skip control messages
                            }
                        } else {
                            if let Some(t) = parsed.get("text").or_else(|| parsed.get("tips")).or_else(|| parsed.get("hint_text")).and_then(|value| value.as_str()) {
                                text = t.to_string();
                            } else {
                                text = raw_content.clone();
                            }
                        }
                    } else {
                        text = raw_content.clone();
                    }
                } else {
                    text = raw_content.clone();
                }
                
                let ext_value = object.get("ext");
                let ext_obj = ext_value.and_then(|v| v.as_object()).cloned().or_else(|| {
                    ext_value
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|v| v.as_object().cloned())
                });
                let mut create_time = object
                    .get("create_time")
                    .and_then(|value| value.as_i64())
                    .unwrap_or_default();
                if create_time == 0 {
                    if let Some(ref ext) = ext_obj {
                        let raw_time = ext.get("s:server_message_create_time")
                            .or_else(|| ext.get("server_message_create_time"));
                        if let Some(value) = raw_time {
                            create_time = value.as_i64().or_else(|| {
                                value.as_str().and_then(|s| s.parse::<i64>().ok())
                            }).unwrap_or_default();
                        }
                    }
                }
                if create_time == 0 {
                    create_time = object.get("version")
                        .or_else(|| object.get("group_version"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or_default();
                    if create_time > 0 && create_time < 10_000_000_000 {
                        create_time *= 1000;
                    }
                }
                if create_time == 0 {
                    create_time = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
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
}
