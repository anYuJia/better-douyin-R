//! 通知消息客户端逻辑 - 点赞/评论/@/关注等互动通知
//!
//! 对应 Python 版 src/api/notice_client.py 与 src/web/notices.py 的整形逻辑。
//! 请求签名（a_bogus）与 msToken/verifyFp/webid 等参数由 request_with_options
//! 与 enrich_request 统一注入，这里只负责通知特有的业务参数与响应整形。

use anyhow::{anyhow, Result};
use std::collections::HashMap;

use super::client::DouyinClient;

/// notice_group 位掩码，960 对应「全部互动」分组（与网页端捕获一致）。
const DEFAULT_NOTICE_GROUP: i64 = 960;

/// 通知类型 → 中文标签（实测 notice_list_v2 的 type 值）。
/// 31=评论/回复，41=赞（赞作品/赞评论），45=@我，33=新粉丝，
/// 514/9002=互动汇总。其余 type 统一显示「通知」。
fn notice_type_label(notice_type: i64) -> &'static str {
    match notice_type {
        1 | 41 => "赞",
        31 => "评论",
        33 => "新粉丝",
        45 => "@我",
        514 | 9002 => "互动",
        _ => "通知",
    }
}

/// 从抖音常见的 url_list 结构里取第一个地址。
fn first_url(media: &serde_json::Value) -> String {
    if let Some(url_list) = media.get("url_list").and_then(|v| v.as_array()) {
        for url in url_list {
            if let Some(s) = url.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    String::new()
}

/// 取用户头像：优先 avatar_thumb，回退 avatar_larger。
fn user_avatar(user: &serde_json::Value) -> String {
    for key in ["avatar_thumb", "avatar_larger"] {
        if let Some(media) = user.get(key) {
            let url = first_url(media);
            if !url.is_empty() {
                return url;
            }
        }
    }
    String::new()
}

/// 整形单个用户信息。
fn format_user(user: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = user.as_object()?;
    let avatar = user_avatar(user);
    let uid = obj
        .get("uid")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let nickname = obj
        .get("nickname")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    // 过滤掉既无 uid 又无昵称的空用户。
    if uid.is_empty() && nickname.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "uid": uid,
        "nickname": nickname,
        "sec_uid": obj.get("sec_uid").and_then(|v| v.as_str()).unwrap_or(""),
        "avatar": avatar,
        "unique_id": obj.get("unique_id").and_then(|v| v.as_str()).unwrap_or(""),
        "follow_status": obj.get("follow_status").cloned(),
        "follower_status": obj.get("follower_status").cloned(),
        "is_verified": obj.get("is_verified").and_then(|v| v.as_bool()).unwrap_or(false),
    }))
}

/// 整形通知里附带的作品摘要（封面/描述）。
fn format_aweme_brief(aweme: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = aweme.as_object()?;
    let mut cover = String::new();
    if let Some(video) = aweme.get("video") {
        cover = first_url(video.get("cover").unwrap_or(&serde_json::Value::Null));
        if cover.is_empty() {
            cover = first_url(video.get("origin_cover").unwrap_or(&serde_json::Value::Null));
        }
    }
    if cover.is_empty() {
        if let Some(images) = aweme.get("images").and_then(|v| v.as_array()) {
            if let Some(first) = images.first() {
                cover = first_url(first);
            }
        }
    }
    Some(serde_json::json!({
        "aweme_id": obj.get("aweme_id").and_then(|v| v.as_str()).unwrap_or(""),
        "desc": obj.get("desc").and_then(|v| v.as_str()).unwrap_or("").trim(),
        "cover": cover,
        "aweme_type": obj.get("aweme_type").cloned(),
    }))
}

/// 从 label_list 取首个文案（label_text 为空时兜底）。
fn label_text_from(label_text: &str, digg: &serde_json::Value) -> String {
    if !label_text.is_empty() {
        return label_text.to_string();
    }
    if let Some(list) = digg.get("label_list").and_then(|v| v.as_array()) {
        if let Some(first) = list.first() {
            if let Some(text) = first.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    String::new()
}

/// 把单条 notice_list_v2 元素整形为前端可用的结构。
fn format_notice(item: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = item.as_object()?;
    let notice_type = obj
        .get("type")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let type_label = notice_type_label(notice_type);

    let mut users: Vec<serde_json::Value> = Vec::new();
    let mut content = String::new();
    let mut merge_count: i64 = 0;
    let mut label_text = String::new();
    let mut aweme_brief: Option<serde_json::Value> = None;
    let mut digg_type: Option<i64> = None;
    let mut comment_text = String::new();
    // 赞评论/赞回复的通知在 digg 里带 comment 字段（digg_type=10/3），
    // 赞作品（digg_type=1）则没有。用 comment 是否存在来区分，比硬编码 digg_type 稳。
    let mut is_comment_like = false;
    let mut is_reply = false;

    let digg = obj.get("digg");
    let follow = obj.get("follow");
    let comment_wrap = obj.get("comment");
    let at = obj.get("at");

    if let Some(digg) = digg.and_then(|v| v.as_object()) {
        let digg_val = serde_json::Value::Object(digg.clone());
        // 点赞类通知：from_user 是数组，可能合并多人。
        if let Some(from_users) = digg_val.get("from_user").and_then(|v| v.as_array()) {
            for user in from_users {
                if let Some(formatted) = format_user(user) {
                    users.push(formatted);
                }
            }
        }
        content = digg_val
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        merge_count = digg_val
            .get("merge_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        digg_type = digg_val.get("digg_type").and_then(|v| v.as_i64());
        let raw_label = digg_val
            .get("label_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        label_text = label_text_from(&raw_label, &digg_val);
        if let Some(comment) = digg_val.get("comment").and_then(|v| v.as_object()) {
            is_comment_like = true;
            comment_text = serde_json::Value::Object(comment.clone())
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
        }
        aweme_brief = digg_val
            .get("aweme")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .and_then(format_aweme_brief);
    } else if let Some(comment_wrap) = comment_wrap.and_then(|v| v.as_object()) {
        let wrap_val = serde_json::Value::Object(comment_wrap.clone());
        // 评论/回复类通知（type 31）：顶层 comment 是包装层，真实评论在
        // comment.comment（含 text + user），被回复评论在 comment.reply_comment。
        if let Some(inner) = wrap_val.get("comment").and_then(|v| v.as_object()) {
            let inner_val = serde_json::Value::Object(inner.clone());
            if let Some(user) = inner_val.get("user") {
                if let Some(formatted) = format_user(user) {
                    users.push(formatted);
                }
            }
            comment_text = inner_val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if let Some(reply) = wrap_val.get("reply_comment").and_then(|v| v.as_object()) {
            let reply_val = serde_json::Value::Object(reply.clone());
            let has_text = reply_val
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            let has_cid = reply_val
                .get("cid")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if has_text || has_cid {
                is_reply = true;
            }
        }
        merge_count = wrap_val
            .get("merge_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let raw_label = wrap_val
            .get("label_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        label_text = label_text_from(&raw_label, &wrap_val);
        aweme_brief = wrap_val
            .get("aweme")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .and_then(format_aweme_brief);
    } else if let Some(at) = at.and_then(|v| v.as_object()) {
        let at_val = serde_json::Value::Object(at.clone());
        // @我 通知（type 45）：用户在 user_info（单个对象），文案在 content。
        if let Some(user_info) = at_val.get("user_info") {
            if let Some(formatted) = format_user(user_info) {
                users.push(formatted);
            }
        }
        // at.content 形如 "@昵称"，不是通知主文案，仅作参考，故不写入 content。
        label_text = at_val
            .get("label_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        aweme_brief = at_val
            .get("aweme")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .and_then(format_aweme_brief);
    } else if let Some(follow) = follow.and_then(|v| v.as_object()) {
        let follow_val = serde_json::Value::Object(follow.clone());
        // 关注类通知：from_user 是单个对象。
        if let Some(from_user) = follow_val.get("from_user") {
            if let Some(formatted) = format_user(from_user) {
                users.push(formatted);
            }
        }
        content = follow_val
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        merge_count = follow_val
            .get("merge_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
    }

    // 兜底文案：接口 content 为空时按类型合成一句。
    if content.is_empty() {
        let names: String = users
            .iter()
            .filter_map(|u| u.get("nickname").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("、");
        content = match notice_type {
            33 => {
                if names.is_empty() {
                    "关注了你".to_string()
                } else {
                    format!("{} 关注了你", names)
                }
            }
            1 | 41 => {
                let target = if is_comment_like { "你的评论" } else { "你的作品" };
                if merge_count > 1 {
                    if names.is_empty() {
                        format!("{} 人赞了{}", merge_count, target)
                    } else {
                        format!("{} 等 {} 人赞了{}", names, merge_count, target)
                    }
                } else if names.is_empty() {
                    format!("赞了{}", target)
                } else {
                    format!("{} 赞了{}", names, target)
                }
            }
            31 => {
                let action = if is_reply { "回复了你的评论" } else { "评论了你" };
                if names.is_empty() {
                    action.to_string()
                } else {
                    format!("{} {}", names, action)
                }
            }
            45 => {
                if names.is_empty() {
                    "@了你".to_string()
                } else {
                    format!("{} @了你", names)
                }
            }
            _ => type_label.to_string(),
        };
    }

    let id = obj
        .get("nid_str")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("nid").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let create_time = obj.get("create_time").and_then(|v| v.as_i64()).unwrap_or(0);
    let has_read = obj.get("has_read").and_then(|v| v.as_bool()).unwrap_or(false);

    Some(serde_json::json!({
        "id": id,
        "type": notice_type,
        "type_label": type_label,
        "create_time": create_time,
        "has_read": has_read,
        "content": content,
        "merge_count": merge_count,
        "label_text": label_text,
        "users": users,
        "aweme": aweme_brief,
        "digg_type": digg_type,
        "is_comment_like": is_comment_like,
        "is_reply": is_reply,
        "comment_text": comment_text,
    }))
}

/// 通知分页结果。
pub struct NoticesResult {
    pub notices: Vec<serde_json::Value>,
    pub has_more: bool,
    pub cursor: i64,
    pub unread_count: i64,
}

impl DouyinClient {
    /// 获取通知消息列表（点赞/关注/评论等互动通知）。
    ///
    /// `max_time` 为翻历史游标：传入上一批返回的 cursor（即接口 max_time）。
    /// 首次拉取传 0。实测用 min_time 方向翻页会返回 status=4「服务器打瞌睡」，
    /// 故游标统一用 max_time。
    pub async fn get_notices(
        &self,
        count: u32,
        min_time: i64,
        max_time: i64,
        notice_group: i64,
    ) -> Result<NoticesResult> {
        let count = count.clamp(1, 50);
        let notice_group = if notice_group <= 0 {
            DEFAULT_NOTICE_GROUP
        } else {
            notice_group
        };

        let mut params: HashMap<&str, String> = HashMap::new();
        // is_new_notice=1 返回新版结构 notice_list_v2；is_mark_read=1 让接口带上已读状态。
        params.insert("is_new_notice", "1".to_string());
        params.insert("is_mark_read", "1".to_string());
        params.insert("notice_group", notice_group.to_string());
        params.insert("count", count.to_string());
        params.insert("min_time", min_time.to_string());
        params.insert("max_time", max_time.to_string());

        let headers = HashMap::from([(
            "Referer".to_string(),
            "https://www.douyin.com/".to_string(),
        )]);

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/notice/",
                Some(params),
                "GET",
                Some(headers),
                false, // 需要签名
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let raw_list = response["notice_list_v2"]
            .as_array()
            .or_else(|| response["notice_list"].as_array());
        let mut notices: Vec<serde_json::Value> = Vec::new();
        if let Some(list) = raw_list {
            for item in list {
                if let Some(formatted) = format_notice(item) {
                    notices.push(formatted);
                }
            }
        }

        let unread_count = notices
            .iter()
            .filter(|n| !n.get("has_read").and_then(|v| v.as_bool()).unwrap_or(false))
            .count() as i64;

        let has_more = response["has_more"]
            .as_bool()
            .or_else(|| response["has_more"].as_i64().map(|v| v == 1))
            .unwrap_or(false);
        // 翻历史游标：抖音 notice 接口用返回的 max_time 作为下一批的 max_time。
        let cursor = response["max_time"].as_i64().unwrap_or(0);

        Ok(NoticesResult {
            notices,
            has_more,
            cursor,
            unread_count,
        })
    }
}
