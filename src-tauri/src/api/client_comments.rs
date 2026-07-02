//! 评论客户端逻辑

use anyhow::{anyhow, Result};
use rand::Rng;
use std::collections::HashMap;

use super::client::DouyinClient;
use super::types::*;
use crate::config::get_user_agent;
use crate::sign;

impl DouyinClient {
    pub async fn get_comments(
        &self,
        aweme_id: &str,
        cursor: i64,
        count: u32,
        insert_ids: &str,
    ) -> Result<(Vec<CommentInfo>, i64, bool, i64)> {
        let mut params = HashMap::new();
        params.insert("aweme_id", aweme_id.to_string());
        params.insert("cursor", cursor.to_string());
        params.insert("count", count.to_string());
        params.insert("pc_img_format", "webp".to_string());
        params.insert("item_type", "0".to_string());
        params.insert("insert_ids", insert_ids.to_string());
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
        let reply_to_user = data
            .get("reply_to_user")
            .or_else(|| data.get("reply_user"))
            .or_else(|| data.get("reply_to_user_info"))
            .or_else(|| data.get("to_user"));
        let reply_to_user_id = data
            .get("reply_to_userid")
            .or_else(|| data.get("reply_to_uid"))
            .or_else(|| data.get("reply_to_user_id"))
            .and_then(|v| v.as_str())
            .or_else(|| reply_to_user.and_then(|u| u.get("uid")).and_then(|v| v.as_str()))
            .unwrap_or_default()
            .to_string();
        let reply_to_user_name = data
            .get("reply_to_user_name")
            .or_else(|| data.get("reply_to_nickname"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                reply_to_user
                    .and_then(|u| u.get("nickname"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or_default()
            .to_string();
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
            sub_comments: data["reply_comment"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| self.parse_comment(c))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    data["sub_comments"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|c| self.parse_comment(c))
                                .collect::<Vec<_>>()
                        })
                        .filter(|v| !v.is_empty())
                }),
            reply_id: data["reply_id"]
                .as_str()
                .or_else(|| data["comment_id"].as_str())
                .or_else(|| data["parent_id"].as_str())
                .unwrap_or_default()
                .to_string(),
            reply_to_reply_id: data["reply_to_reply_id"]
                .as_str()
                .or_else(|| data["reply_to_cid"].as_str())
                .or_else(|| data["reply_to_comment_id"].as_str())
                .unwrap_or_default()
                .to_string(),
            reply_to_user_id,
            reply_to_user_name,
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
}
