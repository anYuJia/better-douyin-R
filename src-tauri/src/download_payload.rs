//! 下载 payload 合并与候选质量分析

use crate::api::{BitRateInfo, VideoInfo};
use crate::downloader::{available_video_quality_height, video_quality_candidate_count};
use std::collections::HashSet;

pub(crate) fn video_info_has_download_candidates(video: &VideoInfo) -> bool {
    !video.video.play_addr.trim().is_empty()
        || video
            .video
            .play_addr_h264
            .as_deref()
            .map(|url| !url.trim().is_empty())
            .unwrap_or(false)
        || video
            .video
            .download_addr
            .as_deref()
            .map(|url| !url.trim().is_empty())
            .unwrap_or(false)
        || video
            .video
            .bit_rate
            .as_ref()
            .map(|items| {
                items.iter().any(|item| {
                    item.play_addr
                        .as_deref()
                        .map(|url| !url.trim().is_empty())
                        .unwrap_or(false)
                        || item
                            .play_addr_h264
                            .as_deref()
                            .map(|url| !url.trim().is_empty())
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false)
}

pub(crate) fn video_info_from_download_payload(payload: &serde_json::Value) -> Option<VideoInfo> {
    let mut value = payload.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("media_type");
        object.remove("raw_media_type");
    }
    serde_json::from_value::<VideoInfo>(value)
        .ok()
        .filter(video_info_has_download_candidates)
}

fn merge_non_empty(target: &mut String, source: &str) {
    if target.trim().is_empty() && !source.trim().is_empty() {
        *target = source.trim().to_string();
    }
}

fn merge_optional_url(target: &mut Option<String>, source: &Option<String>) {
    let target_empty = target
        .as_deref()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true);
    if target_empty {
        if let Some(source) = source
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            *target = Some(source.to_string());
        }
    }
}

fn bit_rate_download_key(bit_rate: &BitRateInfo) -> String {
    let url_key = [
        bit_rate.play_addr_h264.as_deref().unwrap_or("").trim(),
        bit_rate.play_addr.as_deref().unwrap_or("").trim(),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join("|");
    if !url_key.is_empty() {
        return url_key;
    }

    format!(
        "{}:{}:{}:{}:{}:{}",
        bit_rate.gear_name,
        bit_rate.format,
        bit_rate.quality_type,
        bit_rate.width,
        bit_rate.height,
        bit_rate.data_size
    )
}

fn merge_video_download_candidates(target: &mut VideoInfo, source: &VideoInfo) {
    merge_non_empty(&mut target.aweme_id, &source.aweme_id);
    merge_non_empty(&mut target.desc, &source.desc);
    merge_non_empty(&mut target.author.uid, &source.author.uid);
    merge_non_empty(&mut target.author.nickname, &source.author.nickname);
    if target.create_time <= 0 && source.create_time > 0 {
        target.create_time = source.create_time;
    }

    merge_non_empty(&mut target.video.play_addr, &source.video.play_addr);
    merge_optional_url(&mut target.video.preview_addr, &source.video.preview_addr);
    merge_optional_url(&mut target.video.dash_addr, &source.video.dash_addr);
    merge_optional_url(&mut target.video.audio_addr, &source.video.audio_addr);
    merge_optional_url(
        &mut target.video.play_addr_h264,
        &source.video.play_addr_h264,
    );
    merge_optional_url(
        &mut target.video.play_addr_lowbr,
        &source.video.play_addr_lowbr,
    );
    merge_optional_url(&mut target.video.download_addr, &source.video.download_addr);
    merge_non_empty(&mut target.video.cover, &source.video.cover);
    merge_non_empty(&mut target.video.dynamic_cover, &source.video.dynamic_cover);
    merge_non_empty(&mut target.video.origin_cover, &source.video.origin_cover);
    merge_non_empty(&mut target.video.ratio, &source.video.ratio);
    if target.video.width <= 0 && source.video.width > 0 {
        target.video.width = source.video.width;
    }
    if target.video.height <= 0 && source.video.height > 0 {
        target.video.height = source.video.height;
    }
    if target.video.duration <= 0 && source.video.duration > 0 {
        target.video.duration = source.video.duration;
    }

    let mut merged_bit_rates = target.video.bit_rate.take().unwrap_or_default();
    let mut seen = merged_bit_rates
        .iter()
        .map(bit_rate_download_key)
        .collect::<HashSet<_>>();
    if let Some(source_bit_rates) = &source.video.bit_rate {
        for bit_rate in source_bit_rates {
            let key = bit_rate_download_key(bit_rate);
            if !key.is_empty() && seen.insert(key) {
                merged_bit_rates.push(bit_rate.clone());
            }
        }
    }
    target.video.bit_rate = if merged_bit_rates.is_empty() {
        None
    } else {
        Some(merged_bit_rates)
    };
}

pub(crate) fn combined_video_info_for_download(
    fresh_video: Option<&VideoInfo>,
    payload_video: Option<&VideoInfo>,
    aweme_id: &str,
) -> Option<VideoInfo> {
    let mut combined = match (fresh_video, payload_video) {
        (Some(fresh), Some(payload)) => {
            let mut combined = fresh.clone();
            merge_video_download_candidates(&mut combined, payload);
            combined
        }
        (Some(fresh), None) => fresh.clone(),
        (None, Some(payload)) => payload.clone(),
        (None, None) => return None,
    };

    if let Some(payload) = payload_video {
        merge_video_download_candidates(&mut combined, payload);
    }
    if let Some(fresh) = fresh_video {
        merge_video_download_candidates(&mut combined, fresh);
    }

    log::debug!(
        "download_video quality source: aweme_id={} fresh_height={} fresh_count={} payload_height={} payload_count={} combined_height={} combined_count={}",
        aweme_id,
        fresh_video.map(available_video_quality_height).unwrap_or(0),
        fresh_video.map(video_quality_candidate_count).unwrap_or(0),
        payload_video.map(available_video_quality_height).unwrap_or(0),
        payload_video.map(video_quality_candidate_count).unwrap_or(0),
        available_video_quality_height(&combined),
        video_quality_candidate_count(&combined)
    );

    Some(combined)
}
