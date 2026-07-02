//! 好友聊天状态清洗和路径 helper。

use std::collections::HashSet;
use std::path::PathBuf;

pub(crate) fn sanitize_sec_user_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| {
            (value.starts_with("MS4wLjAB") || value.starts_with("MS4w.LjAB"))
                && value.len() <= 180
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
                && seen.insert(value.clone())
        })
        .take(500)
        .collect()
}

pub(crate) fn friend_chat_state_path(sec_uid: Option<&str>) -> PathBuf {
    let filename = match sec_uid {
        Some(uid) if !uid.is_empty() => format!("friend_chat_state_{}.json", uid),
        _ => "friend_chat_state.json".to_string(),
    };
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("better-douyin-R")
        .join(filename)
}

pub(crate) fn coerce_i64(value: Option<&serde_json::Value>, default: i64) -> i64 {
    value
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().map(|value| value as i64))
                .or_else(|| value.as_f64().map(|value| value as i64))
                .or_else(|| {
                    value
                        .as_str()
                        .and_then(|text| text.trim().parse::<i64>().ok())
                })
        })
        .unwrap_or(default)
}

pub(crate) fn sanitize_friend_chat_message(
    value: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let object = value?.as_object()?;
    let mut text = object
        .get("text")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    // Check if text is raw JSON command_type
    if text.starts_with('{') && text.contains("command_type") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(parsed_obj) = parsed.as_object() {
                if parsed_obj.contains_key("command_type")
                    || parsed_obj.get("command_type").and_then(|v| v.as_i64()) == Some(6)
                {
                    let mut found_spark = false;
                    if let Some(ext_data) = parsed_obj.get("ext_data").and_then(|v| v.as_array()) {
                        for ext_item in ext_data {
                            if let Some(ext_obj) = ext_item.as_object() {
                                if ext_obj.get("key").and_then(|v| v.as_str())
                                    == Some("a:consecutive_chat_data")
                                {
                                    if let Some(val_str) =
                                        ext_obj.get("value").and_then(|v| v.as_str())
                                    {
                                        if let Ok(val_json) =
                                            serde_json::from_str::<serde_json::Value>(val_str)
                                        {
                                            if let Some(count_info) =
                                                val_json.get("consecutive_count_info")
                                            {
                                                let count = count_info
                                                    .get("consecutive_count")
                                                    .and_then(|v| v.as_i64())
                                                    .unwrap_or(1);
                                                text = format!(
                                                    "🔥 连续聊天火花已亮起（第 {} 天）",
                                                    count
                                                );
                                                found_spark = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !found_spark {
                        return None; // Filter out generic command_type JSON
                    }
                }
            }
        }
    }

    let raw_content = object
        .get("rawContent")
        .or_else(|| object.get("raw_content"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .chars()
        .take(50000)
        .collect::<String>();
    if text.is_empty() && raw_content.is_empty() {
        return None;
    }
    let created_at = coerce_i64(
        object.get("createdAt").or_else(|| object.get("created_at")),
        0,
    );
    if created_at <= 0 {
        return None;
    }
    let direction = object
        .get("direction")
        .and_then(|value| value.as_str())
        .filter(|value| *value == "in" || *value == "out")
        .unwrap_or("out");
    let status = object
        .get("status")
        .and_then(|value| value.as_str())
        .filter(|value| matches!(*value, "pending" | "sent" | "error"))
        .unwrap_or("sent");
    let id = object
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.chars().take(160).collect::<String>())
        .unwrap_or_else(|| format!("message-{created_at}"));
    let sender_uid = object
        .get("senderUid")
        .or_else(|| object.get("sender_uid"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect::<String>();
    let mut message = serde_json::json!({
        "id": id,
        "text": if text.is_empty() { "[分享内容]".to_string() } else { text.chars().take(1000).collect::<String>() },
        "createdAt": created_at,
        "status": status,
        "direction": direction,
        "senderUid": sender_uid,
    });
    if !raw_content.is_empty() {
        if let Some(object) = message.as_object_mut() {
            object.insert(
                "rawContent".to_string(),
                serde_json::Value::String(raw_content),
            );
        }
    }
    Some(message)
}

pub(crate) fn sanitize_friend_chat_state(value: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object() else {
        return serde_json::json!({"summaries": {}, "unreadCounts": {}});
    };
    let raw_summaries = object
        .get("summaries")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let raw_unread = object
        .get("unreadCounts")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let mut summaries = serde_json::Map::new();
    let mut unread_counts = serde_json::Map::new();

    for (raw_sec_uid, raw_summary) in raw_summaries {
        let sec_uid = raw_sec_uid.trim().to_string();
        let Some(summary) = raw_summary.as_object() else {
            continue;
        };
        if sec_uid.is_empty() || sec_uid.chars().count() > 220 {
            continue;
        }
        let latest_message = sanitize_friend_chat_message(summary.get("latestMessage"));
        let mut latest_at = coerce_i64(summary.get("latestMessageAt"), 0);
        let unread_count = coerce_i64(summary.get("unreadCount"), 0).clamp(0, 999);
        if let Some(latest) = latest_message.as_ref() {
            latest_at = latest_at.max(coerce_i64(latest.get("createdAt"), 0));
        }
        if latest_at <= 0 && unread_count <= 0 {
            continue;
        }
        summaries.insert(
            sec_uid.clone(),
            serde_json::json!({
                "latestMessage": latest_message,
                "latestMessageAt": latest_at,
                "unreadCount": unread_count,
            }),
        );
        if unread_count > 0 {
            unread_counts.insert(sec_uid, serde_json::Value::Number(unread_count.into()));
        }
    }

    for (raw_sec_uid, raw_count) in raw_unread {
        let sec_uid = raw_sec_uid.trim().to_string();
        if sec_uid.is_empty() || sec_uid.chars().count() > 220 {
            continue;
        }
        let count = coerce_i64(Some(&raw_count), 0).clamp(0, 999);
        if count > 0 {
            unread_counts.insert(sec_uid.clone(), serde_json::Value::Number(count.into()));
            if let Some(summary) = summaries
                .get_mut(&sec_uid)
                .and_then(|value| value.as_object_mut())
            {
                let current = coerce_i64(summary.get("unreadCount"), 0);
                summary.insert(
                    "unreadCount".to_string(),
                    serde_json::Value::Number(current.max(count).into()),
                );
            }
        }
    }

    serde_json::json!({
        "summaries": serde_json::Value::Object(summaries),
        "unreadCounts": serde_json::Value::Object(unread_counts),
    })
}

pub(crate) fn json_object_with_success(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("success".to_string(), serde_json::Value::Bool(true));
        value
    } else {
        serde_json::json!({"success": true, "data": value})
    }
}
