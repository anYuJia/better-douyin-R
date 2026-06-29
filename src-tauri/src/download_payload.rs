//! 下载 payload 合并与候选质量分析

use crate::api::{BitRateInfo, DownloadMediaItem, MediaType, VideoInfo};
use crate::downloader::{available_video_quality_height, video_quality_candidate_count};
use crate::media_utils_types::{
    MEDIA_TYPE_AUDIO, MEDIA_TYPE_IMAGE, MEDIA_TYPE_LIVE_PHOTO, MEDIA_TYPE_VIDEO,
};
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

pub(crate) fn merge_download_media_items_into_video_info(
    video: &mut VideoInfo,
    media_items: &[DownloadMediaItem],
) {
    let mut image_urls = video.image_urls.take().unwrap_or_default();
    let mut live_photo_urls = video.live_photo_urls.take().unwrap_or_default();
    let mut has_video = false;

    for media in media_items {
        match media.r#type.as_str() {
            MEDIA_TYPE_IMAGE => merge_url_list(&mut image_urls, std::slice::from_ref(&media.url)),
            MEDIA_TYPE_LIVE_PHOTO => {
                merge_url_list(&mut live_photo_urls, std::slice::from_ref(&media.url));
            }
            MEDIA_TYPE_VIDEO => {
                has_video = true;
                merge_url_list(
                    &mut video.video.play_addr_candidates,
                    std::slice::from_ref(&media.url),
                );
                merge_url_list(&mut video.video.play_addr_candidates, &media.fallback_urls);
            }
            _ => {}
        }
    }

    video.image_urls = if image_urls.is_empty() {
        None
    } else {
        Some(image_urls)
    };
    video.live_photo_urls = if live_photo_urls.is_empty() {
        None
    } else {
        Some(live_photo_urls)
    };
    video.is_image = video
        .image_urls
        .as_ref()
        .map(|urls| !urls.is_empty())
        .unwrap_or(false);
    video.has_live_photo = video
        .live_photo_urls
        .as_ref()
        .map(|urls| !urls.is_empty())
        .unwrap_or(false);

    if video.has_live_photo && video.is_image {
        video.media_type = MediaType::Mixed;
    } else if video.has_live_photo {
        video.media_type = MediaType::LivePhoto;
    } else if video.is_image {
        video.media_type = MediaType::Image;
    } else if has_video {
        video.media_type = MediaType::Video;
    } else if media_items
        .iter()
        .any(|media| media.r#type == MEDIA_TYPE_AUDIO)
    {
        video.media_type = MediaType::Audio;
    }

    for media in media_items
        .iter()
        .filter(|media| media.r#type == MEDIA_TYPE_VIDEO)
    {
        merge_url_list(
            &mut video.video.play_addr_candidates,
            std::slice::from_ref(&media.url),
        );
        merge_url_list(&mut video.video.play_addr_candidates, &media.fallback_urls);
    }

    if video.video.play_addr.trim().is_empty() {
        if let Some(url) = video.video.play_addr_candidates.first() {
            video.video.play_addr = url.clone();
        }
    }
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

fn merge_url_list(target: &mut Vec<String>, source: &[String]) {
    for url in source
        .iter()
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
    {
        if !target.iter().any(|existing| existing == url) {
            target.push(url.to_string());
        }
    }
}

fn bit_rate_download_key(bit_rate: &BitRateInfo) -> String {
    let mut url_parts = [
        bit_rate.play_addr_h264.as_deref().unwrap_or("").trim(),
        bit_rate.play_addr.as_deref().unwrap_or("").trim(),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();
    for url in bit_rate
        .play_addr_h264_candidates
        .iter()
        .chain(bit_rate.play_addr_candidates.iter())
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
    {
        if !url_parts.iter().any(|existing| existing == url) {
            url_parts.push(url.to_string());
        }
    }
    let url_key = url_parts.join("|");
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
    merge_url_list(
        &mut target.video.play_addr_candidates,
        &source.video.play_addr_candidates,
    );
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
        payload_video
            .map(available_video_quality_height)
            .unwrap_or(0),
        payload_video
            .map(video_quality_candidate_count)
            .unwrap_or(0),
        available_video_quality_height(&combined),
        video_quality_candidate_count(&combined)
    );

    Some(combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{BitRateInfo, VideoInfo};
    use crate::downloader::{available_video_quality_height, video_quality_candidate_count};

    #[test]
    fn parses_video_info_from_download_payload_with_string_media_type() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "raw_media_type": "video",
            "media_type": "video",
            "author": { "nickname": "tester" },
            "video": {
                "cover": "https://example.com/cover.jpg",
                "play_addr": "https://example.com/play.mp4",
                "bit_rate": [
                    {
                        "gear_name": "normal_1080_0",
                        "height": 1080,
                        "play_addr_h264": "https://example.com/1080-h264.mp4"
                    }
                ]
            }
        });

        let video_info = video_info_from_download_payload(&payload).expect("video info");

        assert_eq!(video_info.aweme_id, "123");
        assert_eq!(
            video_info
                .video
                .bit_rate
                .as_ref()
                .and_then(|items| items.first())
                .and_then(|item| item.play_addr_h264.as_deref()),
            Some("https://example.com/1080-h264.mp4")
        );
    }

    #[test]
    fn combines_fresh_and_payload_quality_candidates() {
        let mut fresh = VideoInfo::default();
        fresh.aweme_id = "123".to_string();
        fresh.video.play_addr = "https://example.com/fresh-play.mp4".to_string();
        fresh.video.bit_rate = Some(vec![BitRateInfo {
            gear_name: "normal_720_0".to_string(),
            height: 720,
            data_size: 720,
            play_addr_h264: Some("https://example.com/720-h264.mp4".to_string()),
            ..Default::default()
        }]);

        let mut payload = VideoInfo::default();
        payload.aweme_id = "123".to_string();
        payload.video.play_addr = "https://example.com/payload-play.mp4".to_string();
        payload.video.bit_rate = Some(vec![BitRateInfo {
            gear_name: "normal_1080_0".to_string(),
            height: 1080,
            data_size: 1080,
            play_addr_h264: Some("https://example.com/1080-h264.mp4".to_string()),
            ..Default::default()
        }]);

        let combined =
            combined_video_info_for_download(Some(&fresh), Some(&payload), "123").expect("video");

        assert_eq!(available_video_quality_height(&combined), 1080);
        assert_eq!(video_quality_candidate_count(&combined), 2);
    }

    #[test]
    fn merge_download_media_items_marks_payload_as_image_post() {
        let mut video = VideoInfo::default();
        video.aweme_id = "123".to_string();
        video.video.play_addr = "https://example.com/audio-like-video-field".to_string();

        merge_download_media_items_into_video_info(
            &mut video,
            &[
                DownloadMediaItem {
                    r#type: MEDIA_TYPE_IMAGE.to_string(),
                    url: "https://example.com/1.jpeg".to_string(),
                    fallback_urls: Vec::new(),
                },
                DownloadMediaItem {
                    r#type: MEDIA_TYPE_IMAGE.to_string(),
                    url: "https://example.com/2.jpeg".to_string(),
                    fallback_urls: Vec::new(),
                },
            ],
        );

        assert_eq!(video.media_type, MediaType::Image);
        assert!(video.is_image);
        assert_eq!(
            video.image_urls,
            Some(vec![
                "https://example.com/1.jpeg".to_string(),
                "https://example.com/2.jpeg".to_string()
            ])
        );
    }
}
