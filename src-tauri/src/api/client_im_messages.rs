//! IM 消息发送逻辑

use anyhow::{anyhow, Result};
use base64::Engine;
use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use super::client::DouyinClient;
use super::client_im_parse::crc32_hex;
use super::im_proto;
use crate::config::get_user_agent;
use crate::sign;

impl DouyinClient {
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

    pub async fn send_im_content_message_with_headers(
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
}
