use crate::api::DownloadMediaItem;
use crate::api::VideoInfo;
use crate::config::AppConfig;
use crate::media_utils_extract::extract_payload_url;
use crate::media_utils_normalize::{
    clean_video_download_url, is_watermark_video_url, no_watermark_video_url,
};
use crate::media_utils_types::{
    is_dash_video_only_url, MEDIA_TYPE_AUDIO, MEDIA_TYPE_IMAGE, MEDIA_TYPE_LIVE_PHOTO,
    MEDIA_TYPE_VIDEO,
};

pub fn download_media_type_from_payload(payload: &serde_json::Value) -> String {
    if let Some(value) = payload.get("raw_media_type") {
        if let Some(media_type) = value.as_str() {
            return media_type.trim().to_lowercase();
        }
        if let Some(code) = value.as_i64() {
            return match code {
                1 => MEDIA_TYPE_IMAGE.to_string(),
                _ => MEDIA_TYPE_VIDEO.to_string(),
            };
        }
    }

    if let Some(media_type) = payload.get("media_type").and_then(|value| value.as_str()) {
        return media_type.trim().to_lowercase();
    }

    MEDIA_TYPE_VIDEO.to_string()
}

pub fn infer_download_item_type(url: &str, fallback_type: &str) -> String {
    let lower_url = url.to_lowercase();

    if lower_url.ends_with(".mp3") || lower_url.ends_with(".m4a") {
        return MEDIA_TYPE_AUDIO.to_string();
    }
    if lower_url.ends_with(".jpg")
        || lower_url.ends_with(".jpeg")
        || lower_url.ends_with(".png")
        || lower_url.ends_with(".webp")
        || lower_url.ends_with(".gif")
        || lower_url.contains("/image")
        || lower_url.contains("imagex")
    {
        return MEDIA_TYPE_IMAGE.to_string();
    }

    match fallback_type {
        MEDIA_TYPE_IMAGE | MEDIA_TYPE_LIVE_PHOTO | MEDIA_TYPE_VIDEO | MEDIA_TYPE_AUDIO => {
            fallback_type.to_string()
        }
        _ => MEDIA_TYPE_VIDEO.to_string(),
    }
}

pub fn parse_download_media_items(
    payload: &serde_json::Value,
    fallback_type: &str,
) -> Vec<DownloadMediaItem> {
    let mut items = Vec::new();

    append_media_array(
        &mut items,
        payload.get("media_urls"),
        fallback_type,
        fallback_type,
    );
    append_media_array(
        &mut items,
        payload
            .get("video")
            .and_then(|video| video.get("media_urls")),
        fallback_type,
        fallback_type,
    );
    append_media_array(
        &mut items,
        payload.get("live_photos"),
        MEDIA_TYPE_LIVE_PHOTO,
        MEDIA_TYPE_LIVE_PHOTO,
    );
    append_media_array(
        &mut items,
        payload.get("live_photo_urls"),
        MEDIA_TYPE_LIVE_PHOTO,
        MEDIA_TYPE_LIVE_PHOTO,
    );
    append_media_array(
        &mut items,
        payload.get("images"),
        MEDIA_TYPE_IMAGE,
        MEDIA_TYPE_IMAGE,
    );
    append_media_array(
        &mut items,
        payload.get("image_urls"),
        MEDIA_TYPE_IMAGE,
        MEDIA_TYPE_IMAGE,
    );
    append_media_array(
        &mut items,
        payload.get("videos"),
        fallback_type,
        MEDIA_TYPE_VIDEO,
    );

    if items.is_empty() {
        for value in [
            payload
                .get("video")
                .and_then(|video| video.get("play_addr_h264")),
            payload
                .get("video")
                .and_then(|video| video.get("play_addr")),
            payload
                .get("video")
                .and_then(|video| video.get("download_addr")),
            payload
                .get("video")
                .and_then(|video| video.get("preview_addr")),
            payload.get("play_addr"),
            payload.get("download_addr"),
            payload.get("video_url"),
            payload.get("url"),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(url) = extract_payload_url(value) {
                push_download_item(&mut items, MEDIA_TYPE_VIDEO, &url, MEDIA_TYPE_VIDEO);
                if !items.is_empty() {
                    break;
                }
            }
        }
    }

    items
}

pub fn filter_live_photo_media_items(
    media_items: Vec<DownloadMediaItem>,
    config: &AppConfig,
) -> Vec<DownloadMediaItem> {
    let has_live_photo = media_items
        .iter()
        .any(|item| item.r#type == MEDIA_TYPE_LIVE_PHOTO);
    if !has_live_photo {
        return media_items;
    }

    let mut keep_video = config.download_live_photo_video;
    let keep_image = config.download_live_photo_image;
    if !keep_video && !keep_image {
        keep_video = true;
    }

    media_items
        .into_iter()
        .filter(|item| match item.r#type.as_str() {
            MEDIA_TYPE_LIVE_PHOTO => keep_video,
            MEDIA_TYPE_IMAGE => keep_image,
            _ => true,
        })
        .collect()
}

fn append_media_array(
    items: &mut Vec<DownloadMediaItem>,
    value: Option<&serde_json::Value>,
    fallback_type: &str,
    default_type: &str,
) {
    let Some(media_urls) = value.and_then(|value| value.as_array()) else {
        return;
    };

    for media in media_urls {
        if let Some(url) = media.as_str() {
            push_download_item(items, default_type, url, fallback_type);
            continue;
        }

        let media_type = media
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or(default_type);
        if let Some(url) = extract_payload_url(media) {
            push_download_item(items, media_type, &url, fallback_type);
        }
    }
}

fn push_download_item(
    items: &mut Vec<DownloadMediaItem>,
    media_type: &str,
    url: &str,
    fallback_type: &str,
) {
    let original_url = url.trim();
    let mut url = original_url.to_string();
    if url.is_empty() {
        return;
    }

    let media_type = match media_type.trim().to_lowercase().as_str() {
        MEDIA_TYPE_IMAGE => MEDIA_TYPE_IMAGE.to_string(),
        MEDIA_TYPE_LIVE_PHOTO | "livephoto" => MEDIA_TYPE_LIVE_PHOTO.to_string(),
        MEDIA_TYPE_VIDEO => MEDIA_TYPE_VIDEO.to_string(),
        MEDIA_TYPE_AUDIO => MEDIA_TYPE_AUDIO.to_string(),
        _ => infer_download_item_type(&url, fallback_type),
    };

    if media_type == MEDIA_TYPE_VIDEO {
        url = clean_video_download_url(original_url);
        if url.is_empty() || is_watermark_video_url(&url) || is_dash_video_only_url(&url) {
            return;
        }
    }
    if items
        .iter()
        .any(|item| item.url == url && item.r#type == media_type)
    {
        return;
    }

    items.push(DownloadMediaItem {
        r#type: media_type,
        url,
        fallback_urls: Vec::new(),
    });
}

/// 从主地址列表与对应的镜像候选构建图片/Live Photo 下载项。
/// `candidates[i]` 是以主地址开头的完整镜像列表，主地址失败(403/限流)时轮询其余。
pub fn push_image_like_items(
    items: &mut Vec<DownloadMediaItem>,
    primary_urls: &[String],
    candidates: Option<&Vec<Vec<String>>>,
    media_type: &str,
) {
    for (idx, url) in primary_urls.iter().enumerate() {
        if url.trim().is_empty() {
            continue;
        }
        let fallback_urls = candidates
            .and_then(|list| list.get(idx))
            .map(|mirrors| mirrors.iter().skip(1).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        items.push(DownloadMediaItem {
            r#type: media_type.to_string(),
            url: url.clone(),
            fallback_urls,
        });
    }
}

pub fn download_media_items_from_video(video: &VideoInfo, config: &AppConfig) -> Vec<DownloadMediaItem> {
    use crate::api::DouyinClient;
    let mut items = Vec::new();

    if let Some(urls) = &video.live_photo_urls {
        push_image_like_items(
            &mut items,
            urls,
            video.live_photo_url_candidates.as_ref(),
            MEDIA_TYPE_LIVE_PHOTO,
        );
    }

    if let Some(urls) = &video.image_urls {
        push_image_like_items(
            &mut items,
            urls,
            video.image_url_candidates.as_ref(),
            MEDIA_TYPE_IMAGE,
        );
    }

    if items.is_empty() {
        if let Some(url) =
            no_watermark_video_url(video).or_else(|| DouyinClient::get_no_watermark_url(video))
        {
            items.push(DownloadMediaItem {
                r#type: MEDIA_TYPE_VIDEO.to_string(),
                url,
                fallback_urls: Vec::new(),
            });
        }
    }

    filter_live_photo_media_items(items, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_media_types_from_url() {
        assert_eq!(
            infer_download_item_type("https://example.com/test.mp3", "video"),
            "audio"
        );
        assert_eq!(
            infer_download_item_type("https://example.com/test.jpg", "video"),
            "image"
        );
        assert_eq!(
            infer_download_item_type("https://example.com/test.png", "video"),
            "image"
        );
        assert_eq!(
            infer_download_item_type("https://example.com/play", "video"),
            "video"
        );
        assert_eq!(
            infer_download_item_type("https://example.com/image/v1", "video"),
            "image"
        );
    }

    #[test]
    fn parse_download_items_cleans_watermark_video_urls() {
        let payload = serde_json::json!({
            "media_urls": [
                { "type": "video", "url": "https://example.com/aweme/v1/playwm/?watermark=1" },
                { "type": "video", "url": "https://example.com/clean.mp4" }
            ]
        });

        let parsed = parse_download_media_items(&payload, MEDIA_TYPE_VIDEO);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].url,
            "https://example.com/aweme/v1/play/?watermark=0"
        );
        assert_eq!(parsed[1].url, "https://example.com/clean.mp4");
    }

    #[test]
    fn parse_download_items_skips_dash_video_only_urls() {
        let payload = serde_json::json!({
            "media_urls": [
                { "type": "video", "url": "https://example.com/media-video-avc1" },
                { "type": "video", "url": "https://example.com/progressive.mp4" }
            ]
        });

        let parsed = parse_download_media_items(&payload, MEDIA_TYPE_VIDEO);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].url, "https://example.com/progressive.mp4");
    }

    #[test]
    fn parses_flat_download_media_items() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "raw_media_type": "video",
            "media_type": "video",
            "media_urls": [{ "type": "video", "url": "https://example.com/test.mp4" }],
        });

        let parsed = parse_download_media_items(&payload, "video");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "video");
        assert_eq!(parsed[0].url, "https://example.com/test.mp4");
    }

    #[test]
    fn parses_audio_download_media_items_without_video_fallback() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test audio",
            "raw_media_type": "audio",
            "media_type": "audio",
            "media_urls": [{
                "type": "audio",
                "url": "https://lf9-music-east.douyinstatic.com/obj/ies-music/test.mp3"
            }],
            "video": {
                "audio_addr": "https://lf9-music-east.douyinstatic.com/obj/ies-music/test.mp3",
                "play_addr": "",
                "download_addr": null
            }
        });

        let parsed = parse_download_media_items(&payload, "audio");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "audio");
        assert_eq!(
            parsed[0].url,
            "https://lf9-music-east.douyinstatic.com/obj/ies-music/test.mp3"
        );
    }

    #[test]
    fn parses_nested_react_video_payload() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "media_type": "video",
            "author": { "nickname": "tester" },
            "video": {
                "cover": "https://example.com/cover.jpg",
                "play_addr": "https://example.com/play.mp4"
            }
        });

        let parsed = parse_download_media_items(&payload, "video");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "video");
        assert_eq!(parsed[0].url, "https://example.com/play.mp4");
    }

    #[test]
    fn parses_image_and_live_photo_payloads() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "media_type": "mixed",
            "images": ["https://example.com/1.jpg"],
            "live_photos": ["https://example.com/1.mp4"]
        });

        let parsed = parse_download_media_items(&payload, "mixed");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].r#type, "live_photo");
        assert_eq!(parsed[1].r#type, "image");
    }

    #[test]
    fn resolves_download_media_type_from_string_and_numeric_payloads() {
        assert_eq!(
            download_media_type_from_payload(
                &serde_json::json!({ "raw_media_type": "live_photo" })
            ),
            "live_photo"
        );
        assert_eq!(
            download_media_type_from_payload(&serde_json::json!({ "raw_media_type": 1 })),
            "image"
        );
        assert_eq!(
            download_media_type_from_payload(&serde_json::json!({ "media_type": "mixed" })),
            "mixed"
        );
    }
}
