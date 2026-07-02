//! 互动关系客户端逻辑

use anyhow::{anyhow, Result};
use std::collections::HashMap;

use super::client::DouyinClient;
use crate::config::get_user_agent;
use crate::sign;

impl DouyinClient {
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

    pub async fn set_user_followed(
        &self,
        user_id: &str,
        follow: bool,
    ) -> Result<serde_json::Value> {
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
}
