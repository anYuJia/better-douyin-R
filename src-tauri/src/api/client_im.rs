//! IM 客户端逻辑

use anyhow::{anyhow, Result};
use base64::Engine;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use rand::Rng;
use std::collections::HashMap;

use super::client::DouyinClient;
use super::im_proto;
use crate::config::get_user_agent;
use crate::sign;


pub(super) fn crc32_hex(bytes: &[u8]) -> String {
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
    pub(super) fn im_proto_signer(&self) -> Result<&crate::config::RelationSignerConfig> {
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

    pub(super) fn ecdsa_request_sign(value: &str, private_key: &str) -> Result<String> {
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

    pub(super) fn build_im_proto_request(
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

    pub(super) fn build_im_pc_proto_request(&self, cmd: i64, body: &[u8]) -> Result<Vec<u8>> {
        self.build_im_pc_proto_request_with_headers(cmd, body, None)
    }

    pub(super) fn build_im_pc_proto_request_with_headers(
        &self,
        cmd: i64,
        body: &[u8],
        extra_headers: Option<&HashMap<String, String>>,
    ) -> Result<Vec<u8>> {
        self.build_im_proto_request(cmd, body, "", "0.1.6", "fef1a80:p/lzg/store", extra_headers)
    }

    pub(super) async fn post_im_proto(
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

    pub(super) async fn request_im_post(
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

    pub(super) async fn request_im_get(
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


    pub(super) fn first_url_value(value: Option<&serde_json::Value>) -> String {
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

    pub(super) fn media_uri_from_url(url: &str) -> String {
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


    pub(super) fn normalize_im_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
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
