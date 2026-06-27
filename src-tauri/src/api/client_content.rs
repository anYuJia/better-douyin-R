//! 内容客户端辅助函数 - 被多个子模块共享

use super::client::DouyinClient;
use super::types::*;

pub(super) fn clean_video_media_url(url: &str) -> String {
    url.trim()
        .replace("watermark=1", "watermark=0")
        .replace("playwm", "play")
}

pub(super) fn is_dash_video_only_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("media-video") || lower.contains("media_video")
}

pub(super) fn is_valid_recommended_video(video: &VideoInfo) -> bool {
    !video.aweme_id.trim().is_empty()
        && !video.video.play_addr.trim().is_empty()
        && !video.video.cover.trim().is_empty()
        && (!video.author.sec_uid.trim().is_empty()
            || !video.author.uid.trim().is_empty()
            || !video.author.nickname.trim().is_empty())
}

impl DouyinClient {
    pub(super) fn json_count_value(value: &serde_json::Value, keys: &[&str]) -> i64 {
        for key in keys {
            let item = &value[*key];
            if let Some(number) = item.as_i64() {
                return number;
            }
            if let Some(text) = item.as_str() {
                let normalized = text.trim().replace(',', "");
                if let Ok(number) = normalized.parse::<i64>() {
                    return number;
                }
            }
        }
        0
    }

    pub(super) fn json_has_more(value: &serde_json::Value) -> bool {
        value["has_more"].as_i64().unwrap_or(0) == 1
            || value["has_more"].as_bool().unwrap_or(false)
            || matches!(value["has_more"].as_str(), Some("1" | "true" | "True"))
    }

    pub(super) fn json_cursor(value: &serde_json::Value) -> i64 {
        value["cursor"]
            .as_i64()
            .or_else(|| value["max_cursor"].as_i64())
            .or_else(|| value["min_cursor"].as_i64())
            .unwrap_or(0)
    }

    pub(super) fn ensure_status_ok(value: &serde_json::Value) -> anyhow::Result<()> {
        use anyhow::anyhow;
        let status_code = value["status_code"].as_i64().unwrap_or(0);
        if status_code != 0 {
            let status_msg = value["status_msg"].as_str().unwrap_or("unknown error");
            log::warn!(
                "Douyin API status rejected: status_code={} status_msg={}",
                status_code,
                status_msg
            );
            return Err(anyhow!("API error: {} (code={})", status_msg, status_code));
        }
        Ok(())
    }
}
