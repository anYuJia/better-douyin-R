//! IM 消息和媒体链接解析逻辑

use super::client::DouyinClient;

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
}
