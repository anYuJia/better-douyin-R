//! IM 历史消息逻辑

use anyhow::Result;

use super::client::DouyinClient;
use super::im_proto;

const IM_HISTORY_PAGE_SIZE: i64 = 20;

impl DouyinClient {
    async fn get_im_recent_user_messages(&self, cursor: i64) -> Result<serde_json::Value> {
        self.im_proto_signer()?;
        let body = im_proto::build_get_user_message_body(cursor.max(0));
        let payload = self.build_im_pc_proto_request(128, &body)?;
        let response = self
            .post_im_proto(
                "https://imapi.douyin.com/v1/message/get_user_message",
                payload,
                false,
            )
            .await?;
        let body = response
            .pointer("/body/get_user_message_body")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(serde_json::json!({
            "message": "获取历史消息成功",
            "messages": Self::normalize_im_messages(&messages),
            "next_cursor": body.get("next_cursor").cloned().unwrap_or_default(),
            "has_more": body.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false),
        }))
    }

    fn filter_im_history_for_user(result: serde_json::Value, uid: &str) -> serde_json::Value {
        let uid = uid.trim();
        if uid.is_empty() {
            return result;
        }
        let messages = result
            .get("messages")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("sender_uid")
                            .and_then(|value| value.as_str())
                            .map(|sender| sender == uid)
                            .unwrap_or(false)
                            || item
                                .get("conversation_id")
                                .and_then(|value| value.as_str())
                                .map(|conversation_id| conversation_id.contains(uid))
                                .unwrap_or(false)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        serde_json::json!({
            "message": result.get("message").cloned().unwrap_or_else(|| serde_json::json!("获取历史消息成功")),
            "messages": messages,
            "next_cursor": result.get("next_cursor").cloned().unwrap_or_default(),
            "has_more": result.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false),
        })
    }

    pub async fn get_im_history_messages(
        &self,
        cursor: i64,
        to_user_id: Option<&str>,
        conversation_id: Option<&str>,
        conversation_short_id: Option<i64>,
        conversation_type: i64,
    ) -> Result<serde_json::Value> {
        self.im_proto_signer()?;
        let mut created_conversation_for_user = false;
        let conversation = if let (Some(conversation_id), Some(short_id)) = (
            conversation_id.filter(|value| !value.trim().is_empty()),
            conversation_short_id.filter(|value| *value > 0),
        ) {
            Some(serde_json::json!({
                "conversation_id": conversation_id,
                "conversation_short_id": short_id,
                "conversation_type": if conversation_type > 0 { conversation_type } else { 1 },
            }))
        } else if let Some(uid) = to_user_id.filter(|value| !value.trim().is_empty()) {
            match self.create_im_conversation(uid).await {
                Ok(conversation) => {
                    created_conversation_for_user = true;
                    Some(conversation)
                }
                Err(error) => {
                    log::warn!(
                        "Douyin IM create conversation failed, falling back to recent history: uid={} error={}",
                        uid,
                        error
                    );
                    let recent = self.get_im_recent_user_messages(cursor).await?;
                    return Ok(Self::filter_im_history_for_user(recent, uid));
                }
            }
        } else {
            None
        };

        let Some(conversation) = conversation else {
            let recent = self.get_im_recent_user_messages(cursor).await?;
            return Ok(if let Some(uid) = to_user_id {
                Self::filter_im_history_for_user(recent, uid)
            } else {
                recent
            });
        };

        let conversation_id = conversation
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let conversation_short_id = conversation
            .get("conversation_short_id")
            .and_then(|value| value.as_i64())
            .unwrap_or_default();
        let conversation_type = conversation
            .get("conversation_type")
            .and_then(|value| value.as_i64())
            .unwrap_or(1);
        let body = im_proto::build_get_by_conversation_body(
            conversation_id,
            conversation_short_id,
            conversation_type,
            cursor.max(0),
            IM_HISTORY_PAGE_SIZE,
        );
        let payload = self.build_im_pc_proto_request(301, &body)?;
        let response = match self
            .post_im_proto(
                "https://imapi.douyin.com/v1/message/get_by_conversation",
                payload,
                false,
            )
            .await
        {
            Ok(response) => response,
            Err(error) if created_conversation_for_user => {
                if let Some(uid) = to_user_id {
                    log::warn!(
                        "Douyin IM conversation history failed, falling back to recent history: uid={} error={}",
                        uid,
                        error
                    );
                    let recent = self.get_im_recent_user_messages(cursor).await?;
                    return Ok(Self::filter_im_history_for_user(recent, uid));
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let body = response
            .pointer("/body/get_by_conversation_body")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let result = serde_json::json!({
            "message": "获取历史消息成功",
            "messages": Self::normalize_im_messages(&messages),
            "next_cursor": body.get("next_cursor").cloned().unwrap_or_default(),
            "has_more": body.get("has_more").and_then(|value| value.as_bool()).unwrap_or(false),
            "conversation": {
                "conversation_id": conversation_id,
                "conversation_short_id": conversation_short_id,
                "conversation_type": conversation_type,
            },
        });

        let message_count = result
            .get("messages")
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or_default();
        if message_count == 0 && created_conversation_for_user {
            if let Some(uid) = to_user_id {
                let recent = self.get_im_recent_user_messages(cursor).await?;
                return Ok(Self::filter_im_history_for_user(recent, uid));
            }
        }

        Ok(result)
    }
}
