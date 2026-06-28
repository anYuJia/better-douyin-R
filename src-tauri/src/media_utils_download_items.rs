use crate::api::DownloadMediaItem;
use crate::api::VideoInfo;
use crate::media_utils_types::{
    MEDIA_TYPE_AUDIO, MEDIA_TYPE_IMAGE, MEDIA_TYPE_LIVE_PHOTO, MEDIA_TYPE_VIDEO,
    is_dash_video_only_url,
};
use crate::media_utils_normalize::{
    clean_video_download_url, is_watermark_video_url, no_watermark_video_url,
};
use crate::media_utils_extract::extract_payload_url;

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
    });
}

pub fn download_media_items_from_video(video: &VideoInfo) -> Vec<DownloadMediaItem> {
    use crate::api::DouyinClient;
    let mut items = Vec::new();

    if let Some(urls) = &video.live_photo_urls {
        for url in urls {
            if !url.trim().is_empty() {
                items.push(DownloadMediaItem {
                    r#type: MEDIA_TYPE_LIVE_PHOTO.to_string(),
                    url: url.clone(),
                });
            }
        }
    }

    if let Some(urls) = &video.image_urls {
        for url in urls {
            if !url.trim().is_empty() {
                items.push(DownloadMediaItem {
                    r#type: MEDIA_TYPE_IMAGE.to_string(),
                    url: url.clone(),
                });
            }
        }
    }

    if items.is_empty() {
        if let Some(url) =
            no_watermark_video_url(video).or_else(|| DouyinClient::get_no_watermark_url(video))
        {
            items.push(DownloadMediaItem {
                r#type: MEDIA_TYPE_VIDEO.to_string(),
                url,
            });
        }
    }

    items
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
