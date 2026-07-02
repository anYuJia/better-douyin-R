//! 用户客户端逻辑 - 搜索用户、获取用户详情、当前账号

use anyhow::{anyhow, Result};
use std::collections::HashMap;

use super::client::DouyinClient;
use super::types::*;

impl DouyinClient {
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

    /// 从分享文本中提取第一个可请求链接。
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
                    let error_message = e.to_string();
                    if looks_like_profile_transport_error(&error_message) {
                        log::warn!(
                            "Douyin profile cookie check is temporarily unavailable; keeping saved cookie usable: {}",
                            e
                        );
                        return Ok(CookieStatus {
                            valid: true,
                            user_name: None,
                            user_id: None,
                            sec_uid: None,
                            avatar_thumb: None,
                            avatar_medium: None,
                            avatar_larger: None,
                            expires_at: None,
                            message: format!(
                                "Cookie 暂时无法通过个人资料接口确认，但检测到登录 Cookie，已保留登录态: {}",
                                e
                            ),
                        });
                    }
                    if looks_like_logged_out_error(&error_message) {
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

    pub(super) async fn get_current_user_from_profile_self(&self) -> Result<UserInfo> {
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

fn looks_like_profile_transport_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("error sending request")
        || lower.contains("request failed")
        || lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("dns")
        || lower.contains("tls")
        || lower.contains("unexpected eof")
        || message.contains("网络")
        || message.contains("连接")
        || message.contains("超时")
}

fn looks_like_logged_out_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    message.contains("用户未登录")
        || message.contains("未登录")
        || message.contains("重新登录")
        || lower.contains("not login")
        || lower.contains("not logged in")
        || lower.contains("login required")
        || lower.contains("session expired")
}
