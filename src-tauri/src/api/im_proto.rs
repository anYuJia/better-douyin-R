use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ConversationInfo {
    pub conversation_id: String,
    pub conversation_short_id: i64,
    pub conversation_type: i64,
    pub ticket: String,
}

#[derive(Debug, Clone)]
pub struct SentMessageInfo {
    pub conversation_id: String,
    pub conversation_short_id: i64,
    pub conversation_type: i64,
    pub server_message_id: i64,
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn key(field: u64, wire_type: u64) -> Vec<u8> {
    varint((field << 3) | wire_type)
}

fn int_field(field: u64, value: impl Into<i64>) -> Vec<u8> {
    let mut out = key(field, 0);
    out.extend(varint(value.into().max(0) as u64));
    out
}

fn bytes_field(field: u64, value: &[u8]) -> Vec<u8> {
    let mut out = key(field, 2);
    out.extend(varint(value.len() as u64));
    out.extend(value);
    out
}

fn string_field(field: u64, value: &str) -> Vec<u8> {
    bytes_field(field, value.as_bytes())
}

fn map_entry(field: u64, key: &str, value: &str) -> Vec<u8> {
    let mut entry = string_field(1, key);
    entry.extend(string_field(2, value));
    bytes_field(field, &entry)
}

fn packed_ints_field(field: u64, values: &[i64]) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in values {
        payload.extend(varint((*value).max(0) as u64));
    }
    bytes_field(field, &payload)
}

pub fn build_request(
    cmd: i64,
    token: &str,
    ts_sign: &str,
    sdk_cert: &str,
    request_sign: &str,
    body: &[u8],
    headers: &HashMap<String, String>,
    sequence_id: i64,
    sdk_version: &str,
    build_number: &str,
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend(int_field(1, cmd));
    payload.extend(int_field(2, sequence_id));
    payload.extend(string_field(3, sdk_version));
    payload.extend(string_field(4, token));
    payload.extend(int_field(5, 3));
    payload.extend(int_field(6, 0));
    payload.extend(string_field(7, build_number));
    payload.extend(bytes_field(8, body));
    payload.extend(string_field(9, "0"));
    payload.extend(string_field(11, "douyin_pc"));
    for (key, value) in headers {
        payload.extend(map_entry(15, key, value));
    }
    payload.extend(int_field(18, 4));
    payload.extend(string_field(21, "douyin_web"));
    payload.extend(string_field(22, "web_sdk"));
    payload.extend(string_field(23, ts_sign));
    payload.extend(string_field(24, sdk_cert));
    if !request_sign.trim().is_empty() {
        payload.extend(string_field(25, request_sign));
    }
    payload
}

pub fn build_create_conversation_body(to_uid: i64, my_uid: i64) -> Vec<u8> {
    let mut inner = int_field(1, 1);
    inner.extend(packed_ints_field(2, &[to_uid, my_uid]));
    bytes_field(609, &inner)
}

pub fn build_send_message_body(
    conversation_id: &str,
    conversation_short_id: i64,
    ticket: &str,
    content: &str,
    client_message_id: &str,
    now_ms: i64,
) -> Vec<u8> {
    let mut ext_client_id = string_field(1, "s:client_message_id");
    ext_client_id.extend(string_field(2, client_message_id));
    let mut ext_time = string_field(1, "s:stime");
    ext_time.extend(string_field(2, &now_ms.to_string()));
    let mut ext_mentions = string_field(1, "s:mentioned_users");
    ext_mentions.extend(string_field(2, ""));

    let mut inner = Vec::new();
    inner.extend(string_field(1, conversation_id));
    inner.extend(int_field(2, 1));
    inner.extend(int_field(3, conversation_short_id));
    inner.extend(string_field(4, content));
    inner.extend(bytes_field(5, &ext_client_id));
    inner.extend(bytes_field(5, &ext_time));
    inner.extend(bytes_field(5, &ext_mentions));
    inner.extend(int_field(6, 7));
    inner.extend(string_field(7, ticket));
    inner.extend(string_field(8, client_message_id));
    bytes_field(100, &inner)
}

pub fn build_get_user_message_body(cursor: i64) -> Vec<u8> {
    let mut paging = int_field(1, cursor.max(0));
    paging.extend(int_field(3, 0));
    let mut inner_zero = int_field(1, 0);
    inner_zero.extend(int_field(2, 0));
    paging.extend(bytes_field(4, &inner_zero));
    let inner = bytes_field(1, &paging);
    bytes_field(128, &inner)
}

pub fn build_get_by_conversation_body(
    conversation_id: &str,
    conversation_short_id: i64,
    conversation_type: i64,
    cursor: i64,
    count: i64,
) -> Vec<u8> {
    let mut inner = Vec::new();
    inner.extend(string_field(1, conversation_id));
    inner.extend(int_field(
        2,
        if conversation_type > 0 {
            conversation_type
        } else {
            1
        },
    ));
    inner.extend(int_field(3, conversation_short_id.max(0)));
    inner.extend(int_field(4, 1));
    if cursor > 0 {
        inner.extend(int_field(5, cursor));
    }
    inner.extend(int_field(6, count.clamp(1, 100)));
    bytes_field(301, &inner)
}

pub fn parse_response(data: &[u8]) -> Value {
    let fields = parse_fields(data);
    let cmd = first_int(&fields, 1);
    let body = first_bytes(&fields, 6);
    json!({
        "cmd": cmd,
        "sequence_id": first_int(&fields, 2),
        "error_desc": first_string(&fields, 3),
        "message": first_string(&fields, 4),
        "body": if body.is_empty() { json!({}) } else { parse_response_body(&body, cmd) },
    })
}

fn parse_response_body(data: &[u8], cmd: i64) -> Value {
    let fields = parse_fields(data);
    let by_conversation = first_bytes(&fields, 301);
    if !by_conversation.is_empty() {
        return json!({"get_by_conversation_body": parse_messages_by_conversation(&by_conversation)});
    }
    let user_message = first_bytes(&fields, 128);
    if !user_message.is_empty() {
        return json!({"get_user_message_body": parse_messages_by_conversation(&user_message)});
    }
    let notify = first_bytes(&fields, 500);
    if !notify.is_empty() {
        return json!({"new_message_notify": parse_new_message_notify(&notify)});
    }
    let create = first_bytes(&fields, 609);
    if !create.is_empty() {
        return json!({"create_conversation_v2_body": parse_conversation_info_list(&create)});
    }
    let info_list = first_bytes(&fields, 610);
    if !info_list.is_empty() {
        return json!({"get_conversation_info_list_body": parse_conversation_info_list(&info_list)});
    }
    if (cmd == 128 || cmd == 301) && !data.is_empty() {
        return json!({"get_by_conversation_body": parse_messages_by_conversation(data)});
    }
    json!({})
}

fn parse_conversation_info_list(data: &[u8]) -> Value {
    let fields = parse_fields(data);
    let mut conversations = Vec::new();
    for (wire_type, raw) in fields.get(&1).into_iter().flatten() {
        if *wire_type == 2 {
            conversations.push(parse_conversation_info(raw));
        }
    }
    json!({ "conversation_info_list": conversations })
}

fn parse_conversation_info(data: &[u8]) -> Value {
    let fields = parse_fields(data);
    json!({
        "conversation_id": first_string(&fields, 1),
        "conversation_short_id": first_int(&fields, 2),
        "conversation_type": first_int(&fields, 3),
        "ticket": first_string(&fields, 4),
    })
}

fn parse_new_message_notify(data: &[u8]) -> Value {
    let fields = parse_fields(data);
    let message = first_bytes(&fields, 5);
    json!({
        "conversation_id": first_string(&fields, 2),
        "conversation_type": first_int(&fields, 3),
        "notify_type": first_int(&fields, 4),
        "message": if message.is_empty() { json!({}) } else { parse_message_body(&message) },
    })
}

fn parse_message_body(data: &[u8]) -> Value {
    let fields = parse_fields(data);
    let mut ext = serde_json::Map::new();
    for field in [9_u64, 15_u64] {
        for (wire_type, raw) in fields.get(&field).into_iter().flatten() {
            if *wire_type != 2 {
                continue;
            }
            let entry_fields = parse_fields(raw);
            let key = first_string(&entry_fields, 1);
            if !key.is_empty() {
                ext.insert(key, Value::String(first_string(&entry_fields, 2)));
            }
        }
    }
    json!({
        "conversation_id": first_string(&fields, 1),
        "conversation_type": first_int(&fields, 2),
        "server_message_id": first_int(&fields, 3),
        "index_in_conversation": first_int(&fields, 4),
        "conversation_short_id": first_int(&fields, 5),
        "message_type": first_int(&fields, 6),
        "sender": first_int(&fields, 7),
        "content": first_string(&fields, 8),
        "create_time": first_int(&fields, 9),
        "status": first_int(&fields, 11),
        "order_in_conversation": first_int(&fields, 12),
        "ext": Value::Object(ext),
        "sec_sender": first_string(&fields, 18)
            .or_else(|| first_string(&fields, 16))
            .or_else(|| first_string(&fields, 14)),
    })
}

fn parse_messages_by_conversation(data: &[u8]) -> Value {
    let fields = parse_fields(data);
    let mut messages = Vec::new();
    for (wire_type, raw) in fields.get(&1).into_iter().flatten() {
        if *wire_type == 2 {
            messages.push(parse_message_body(raw));
        }
    }
    json!({
        "messages": messages,
        "next_cursor": first_int(&fields, 2).max(first_int(&fields, 3)).max(first_int(&fields, 5)),
        "has_more": first_int(&fields, 4) != 0 || first_int(&fields, 6) != 0,
    })
}

pub fn first_conversation(response: &Value) -> Option<ConversationInfo> {
    let item = response
        .pointer("/body/create_conversation_v2_body/conversation_info_list")?
        .as_array()?
        .first()?;
    let conversation_id = item.get("conversation_id")?.as_str()?.trim().to_string();
    let ticket = item.get("ticket")?.as_str()?.trim().to_string();
    let conversation_short_id = item.get("conversation_short_id")?.as_i64()?;
    let conversation_type = item
        .get("conversation_type")
        .and_then(|value| value.as_i64())
        .unwrap_or(1);
    if conversation_id.is_empty() || ticket.is_empty() || conversation_short_id == 0 {
        return None;
    }
    Some(ConversationInfo {
        conversation_id,
        conversation_short_id,
        conversation_type,
        ticket,
    })
}

pub fn sent_message(response: &Value) -> Option<SentMessageInfo> {
    let notify = response.pointer("/body/new_message_notify")?;
    let item = notify.get("message")?;
    let conversation_id = item
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .or_else(|| {
            notify
                .get("conversation_id")
                .and_then(|value| value.as_str())
        })?
        .trim()
        .to_string();
    let conversation_short_id = item.get("conversation_short_id")?.as_i64()?;
    let server_message_id = item.get("server_message_id")?.as_i64()?;
    if conversation_id.is_empty() || conversation_short_id == 0 || server_message_id == 0 {
        return None;
    }
    Some(SentMessageInfo {
        conversation_id,
        conversation_short_id,
        conversation_type: item
            .get("conversation_type")
            .and_then(|value| value.as_i64())
            .or_else(|| {
                notify
                    .get("conversation_type")
                    .and_then(|value| value.as_i64())
            })
            .unwrap_or(1),
        server_message_id,
    })
}

fn parse_fields(data: &[u8]) -> HashMap<u64, Vec<(u8, Vec<u8>)>> {
    let mut result: HashMap<u64, Vec<(u8, Vec<u8>)>> = HashMap::new();
    let mut index = 0usize;
    while index < data.len() {
        let Some((tag, next)) = read_varint(data, index) else {
            break;
        };
        index = next;
        let field = tag >> 3;
        let wire_type = (tag & 0x07) as u8;
        let value = match wire_type {
            0 => {
                let Some((value, next)) = read_varint(data, index) else {
                    break;
                };
                index = next;
                value.to_le_bytes().to_vec()
            }
            2 => {
                let Some((length, next)) = read_varint(data, index) else {
                    break;
                };
                index = next;
                let end = index.saturating_add(length as usize);
                if end > data.len() {
                    break;
                }
                let value = data[index..end].to_vec();
                index = end;
                value
            }
            _ => break,
        };
        result.entry(field).or_default().push((wire_type, value));
    }
    result
}

fn read_varint(data: &[u8], mut index: usize) -> Option<(u64, usize)> {
    let mut shift = 0u32;
    let mut value = 0u64;
    while index < data.len() {
        let byte = data[index];
        index += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return Some((value, index));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

fn first_int(fields: &HashMap<u64, Vec<(u8, Vec<u8>)>>, field: u64) -> i64 {
    fields
        .get(&field)
        .into_iter()
        .flatten()
        .find_map(|(wire_type, value)| {
            if *wire_type != 0 || value.len() != 8 {
                return None;
            }
            Some(u64::from_le_bytes(value.as_slice().try_into().ok()?) as i64)
        })
        .unwrap_or(0)
}

fn first_bytes(fields: &HashMap<u64, Vec<(u8, Vec<u8>)>>, field: u64) -> Vec<u8> {
    fields
        .get(&field)
        .into_iter()
        .flatten()
        .find_map(|(wire_type, value)| {
            if *wire_type == 2 {
                Some(value.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn first_string(fields: &HashMap<u64, Vec<(u8, Vec<u8>)>>, field: u64) -> String {
    String::from_utf8(first_bytes(fields, field)).unwrap_or_default()
}

trait OrElseString {
    fn or_else<F: FnOnce() -> String>(self, f: F) -> String;
}

impl OrElseString for String {
    fn or_else<F: FnOnce() -> String>(self, f: F) -> String {
        if self.is_empty() {
            f()
        } else {
            self
        }
    }
}
