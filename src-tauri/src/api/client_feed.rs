//! 推荐客户端逻辑 - 首页推荐/featured feed

use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};

use super::client::DouyinClient;
use super::client_content;
use super::types::*;

fn normalize_recommended_feed_type(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "recommended" | "recommend" | "tab" | "home" | "feed" => "recommended",
        _ => "featured",
    }
}

impl DouyinClient {
    /// 获取推荐视频
    pub async fn get_recommended_feed(
        &self,
        cursor: i64,
        count: u32,
        feed_type: &str,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        if normalize_recommended_feed_type(feed_type) == "recommended" {
            return self.get_home_recommended_feed(cursor, count).await;
        }

        let mut params = HashMap::new();
        params.insert("module_id", "3003101".to_string());
        params.insert("count", count.to_string());
        params.insert("pull_type", "0".to_string());
        params.insert("refresh_index", "1".to_string());
        params.insert("refer_type", "10".to_string());
        params.insert("filterGids", "".to_string());
        params.insert("presented_ids", "".to_string());
        params.insert("refer_id", "".to_string());
        params.insert("tag_id", "".to_string());
        params.insert("use_lite_type", "2".to_string());
        params.insert("Seo-Flag", "0".to_string());
        params.insert("pre_log_id", "".to_string());
        params.insert("pre_item_ids", "".to_string());
        params.insert("pre_room_ids", "".to_string());
        params.insert("pre_item_from", "sati".to_string());
        params.insert("xigua_user", "0".to_string());
        params.insert(
            "awemePcRecRawData",
            "{\"is_xigua_user\":0,\"danmaku_switch_status\":0,\"is_client\":false}".to_string(),
        );
        if cursor > 0 {
            params.insert("cursor", cursor.to_string());
        }

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.douyin.com/?recommend=1".to_string(),
        );

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v2/web/module/feed/",
                Some(params),
                "POST",
                Some(headers),
                false, // 需要签名
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let aweme_list = response["aweme_list"].as_array();
        // has_more 可能是布尔值或整数
        let has_more = response["has_more"]
            .as_bool()
            .or_else(|| response["has_more"].as_i64().map(|v| v == 1))
            .unwrap_or(false);
        let next_cursor = response["cursor"]
            .as_i64()
            .or_else(|| response["max_cursor"].as_i64())
            .or_else(|| response["min_cursor"].as_i64())
            .unwrap_or_else(|| if has_more { cursor + 1 } else { cursor });

        let videos = if let Some(list) = aweme_list {
            list.iter()
                .filter_map(|v| self.parse_video_info(v).ok())
                .collect()
        } else {
            vec![]
        };

        Ok((videos, next_cursor, has_more))
    }

    async fn get_home_recommended_feed(
        &self,
        cursor: i64,
        count: u32,
    ) -> Result<(Vec<VideoInfo>, i64, bool)> {
        let refresh_index = std::cmp::max(1, cursor + 1);
        let raw_data = serde_json::json!({
            "is_client": false,
            "ff_danmaku_status": 1,
            "danmaku_switch_status": 0,
            "is_dash_user": 1,
            "related_recommend": 1,
            "is_xigua_user": 0,
        })
        .to_string();

        let mut params = HashMap::new();
        params.insert("filterGids", String::new());
        params.insert("tag_id", String::new());
        params.insert("live_insert_type", String::new());
        params.insert("count", count.to_string());
        params.insert("refresh_index", refresh_index.to_string());
        params.insert("video_type_select", "1".to_string());
        params.insert("aweme_pc_rec_raw_data", raw_data);
        params.insert("globalwid", String::new());
        params.insert("pull_type", if cursor <= 0 { "0" } else { "2" }.to_string());
        params.insert("min_window", "0".to_string());
        params.insert("free_right", "0".to_string());
        params.insert("view_count", cursor.max(0).to_string());
        params.insert("plug_block", "0".to_string());
        params.insert("ug_source", String::new());
        params.insert("creative_id", String::new());
        params.insert("webcast_sdk_version", "170400".to_string());
        params.insert("webcast_version_code", "170400".to_string());

        let headers = HashMap::from([(
            "Referer".to_string(),
            "https://www.douyin.com/?recommend=1".to_string(),
        )]);

        let response = self
            .request_raw_json_with_options(
                "https://www.douyin.com/aweme/v1/web/tab/feed/",
                Some(params),
                "GET",
                Some(headers),
                false,
            )
            .await?;

        let status_code = response["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let status_msg = response["status_msg"]
                .as_str()
                .or_else(|| response["message"].as_str())
                .unwrap_or("unknown error");
            return Err(anyhow!("API error: {}", status_msg));
        }

        let has_more = response["has_more"]
            .as_bool()
            .or_else(|| response["has_more"].as_i64().map(|v| v == 1))
            .unwrap_or(false);
        let next_cursor = response["cursor"]
            .as_i64()
            .or_else(|| response["max_cursor"].as_i64())
            .or_else(|| response["min_cursor"].as_i64())
            .unwrap_or_else(|| if has_more { refresh_index } else { cursor });

        let videos = if let Some(list) = response["aweme_list"].as_array() {
            // 预解析 fallback 与 aweme_id，避免在并发任务里持有 list 借用。
            // tab/feed 返回的视频字段不完整（缺少可播放地址），需要逐个请求 detail 刷新。
            // 用 buffered(4) 并发请求，保持原列表顺序，与 Python 版行为一致。
            let prepared: Vec<(String, Option<VideoInfo>)> = list
                .iter()
                .map(|value| {
                    let fallback = self.parse_video_info(value).ok();
                    let aweme_id = value["aweme_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .or_else(|| fallback.as_ref().map(|video| video.aweme_id.clone()))
                        .unwrap_or_default();
                    (aweme_id, fallback)
                })
                .collect();

            use futures::stream::{self, StreamExt};
            const HYDRATE_CONCURRENCY: usize = 4;
            let aweme_ids: Vec<String> = prepared
                .iter()
                .map(|(aweme_id, _)| aweme_id.clone())
                .collect();
            let hydrated: Vec<Option<VideoInfo>> = stream::iter(aweme_ids)
                .map(|aweme_id| async move {
                    if aweme_id.is_empty() {
                        return None;
                    }
                    match self.get_video_detail(&aweme_id).await {
                        Ok(detail) if client_content::is_valid_recommended_video(&detail) => {
                            Some(detail)
                        }
                        Ok(_) => {
                            log::warn!(
                                "home recommended detail had no playable media: aweme_id={}",
                                aweme_id
                            );
                            None
                        }
                        Err(error) => {
                            log::warn!(
                                "home recommended detail hydration failed: aweme_id={} error={}",
                                aweme_id,
                                error
                            );
                            None
                        }
                    }
                })
                .buffered(HYDRATE_CONCURRENCY)
                .collect()
                .await;

            let mut videos = Vec::new();
            let mut seen_ids = HashSet::new();
            let mut hydrated_count = 0usize;
            for ((_, fallback), maybe_detail) in prepared.into_iter().zip(hydrated) {
                if maybe_detail.is_some() {
                    hydrated_count += 1;
                }
                let video = maybe_detail
                    .or(fallback.filter(client_content::is_valid_recommended_video));
                let Some(video) = video else { continue };
                if !video.aweme_id.trim().is_empty() && !seen_ids.insert(video.aweme_id.clone()) {
                    continue;
                }
                videos.push(video);
            }

            if hydrated_count > 0 {
                log::debug!(
                    "home recommended detail hydration completed: {}/{}",
                    hydrated_count,
                    videos.len()
                );
            }
            videos
        } else {
            Vec::new()
        };

        Ok((videos, next_cursor, has_more))
    }
}
