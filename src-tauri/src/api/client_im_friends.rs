//! IM 好友客户端逻辑

use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use super::client::DouyinClient;

impl DouyinClient {
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
}
