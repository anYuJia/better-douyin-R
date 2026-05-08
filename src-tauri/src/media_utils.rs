//! 媒体工具模块

use crate::api::{DownloadMediaItem, MediaType, VideoInfo};

// ============================================================================
// 常量
// ============================================================================

pub const MEDIA_TYPE_VIDEO: &str = "video";
pub const MEDIA_TYPE_IMAGE: &str = "image";
pub const MEDIA_TYPE_LIVE_PHOTO: &str = "live_photo";
pub const MEDIA_TYPE_MIXED: &str = "mixed";
pub const MEDIA_TYPE_AUDIO: &str = "audio";

pub fn normalize_duration_seconds(value: i64) -> i64 {
    if value <= 0 {
        return 0;
    }

    if value >= 100_000 {
        return std::cmp::max(1, (value as f64 / 100_000.0).round() as i64);
    }
    if value >= 1_000 {
        return std::cmp::max(1, (value as f64 / 1_000.0).round() as i64);
    }
    if value >= 100 {
        return std::cmp::max(1, (value as f64 / 100.0).round() as i64);
    }

    std::cmp::max(1, value)
}

pub fn python_media_type(video: &VideoInfo) -> &'static str {
    let has_images = video
        .image_urls
        .as_ref()
        .map(|urls| !urls.is_empty())
        .unwrap_or(false);
    let has_live = video.has_live_photo
        || video
            .live_photo_urls
            .as_ref()
            .map(|urls| !urls.is_empty())
            .unwrap_or(false);

    if has_live && has_images {
        MEDIA_TYPE_MIXED
    } else if has_live {
        MEDIA_TYPE_LIVE_PHOTO
    } else if has_images || video.is_image {
        MEDIA_TYPE_IMAGE
    } else if !video.video.play_addr.is_empty() {
        MEDIA_TYPE_VIDEO
    } else {
        "unknown"
    }
}

pub fn python_media_urls(video: &VideoInfo) -> Vec<serde_json::Value> {
    let mut items = Vec::new();

    if let Some(urls) = &video.live_photo_urls {
        for url in urls {
            if !url.is_empty() {
                items.push(serde_json::json!({ "type": MEDIA_TYPE_LIVE_PHOTO, "url": url }));
            }
        }
    }

    if let Some(urls) = &video.image_urls {
        for url in urls {
            if !url.is_empty() {
                items.push(serde_json::json!({ "type": MEDIA_TYPE_IMAGE, "url": url }));
            }
        }
    }

    if items.is_empty() && !video.video.play_addr.is_empty() {
        items.push(serde_json::json!({ "type": MEDIA_TYPE_VIDEO, "url": video.video.play_addr }));
    }

    items
}

pub fn python_cover_url(video: &VideoInfo) -> String {
    if !video.video.cover.is_empty() {
        return video.video.cover.clone();
    }

    video
        .image_urls
        .as_ref()
        .and_then(|urls| urls.first())
        .cloned()
        .unwrap_or_default()
}

pub fn python_music_play_url(video: &VideoInfo) -> String {
    video
        .music
        .as_ref()
        .and_then(|music| music.play_url.clone())
        .unwrap_or_default()
}

pub fn python_music_info(video: &VideoInfo) -> serde_json::Value {
    let play_url = python_music_play_url(video);
    serde_json::json!({
        "title": video.music.as_ref().map(|music| music.title.clone()).unwrap_or_default(),
        "author": video.music.as_ref().map(|music| music.author.clone()).unwrap_or_default(),
        "play_url": play_url,
        "duration": normalize_duration_seconds(video.music.as_ref().map(|music| music.duration).unwrap_or(0)),
    })
}

pub fn python_user_value(user: &crate::api::UserInfo) -> serde_json::Value {
    serde_json::json!({
        "nickname": user.nickname,
        "unique_id": user.unique_id,
        "follower_count": user.follower_count,
        "following_count": user.following_count,
        "total_favorited": user.total_favorited,
        "aweme_count": user.aweme_count,
        "signature": user.signature,
        "sec_uid": user.sec_uid,
        "avatar_thumb": user.avatar_thumb,
        "avatar_larger": user.avatar_larger,
    })
}

pub fn python_video_summary(
    video: &VideoInfo,
    include_duration: bool,
    include_music: bool,
) -> serde_json::Value {
    let media_type = python_media_type(video);
    let media_urls = python_media_urls(video);
    let mut bgm_url = python_music_play_url(video);

    if bgm_url.is_empty() && !video.video.play_addr.is_empty() {
        bgm_url = video.video.play_addr.clone();
    }

    let mut value = serde_json::json!({
        "aweme_id": video.aweme_id,
        "desc": video.desc,
        "create_time": video.create_time,
        "digg_count": video.statistics.digg_count,
        "comment_count": video.statistics.comment_count,
        "share_count": video.statistics.share_count,
        "cover_url": python_cover_url(video),
        "media_type": media_type,
        "media_urls": media_urls,
        "bgm_url": bgm_url,
        "author": {
            "nickname": video.author.nickname,
            "avatar_thumb": video.author.avatar_thumb,
            "sec_uid": video.author.sec_uid,
        }
    });

    if include_duration {
        value["duration"] = serde_json::json!(normalize_duration_seconds(video.video.duration));
    }

    if include_music {
        let music = python_music_info(video);
        value["music"] = music.clone();
        value["music_title"] = music["title"].clone();
        value["music_author"] = music["author"].clone();
        value["music_url"] = music["play_url"].clone();
        value["music_duration"] = music["duration"].clone();
    }

    value
}

pub fn python_video_detail_value(video: &VideoInfo) -> serde_json::Value {
    let media_type = python_media_type(video);
    let media_urls = python_media_urls(video);

    serde_json::json!({
        "aweme_id": video.aweme_id,
        "desc": video.desc,
        "create_time": video.create_time,
        "digg_count": video.statistics.digg_count,
        "comment_count": video.statistics.comment_count,
        "share_count": video.statistics.share_count,
        "author": {
            "nickname": video.author.nickname,
            "unique_id": video.author.uid,
            "sec_uid": video.author.sec_uid,
            "avatar_thumb": video.author.avatar_thumb,
        },
        "statistics": {
            "digg_count": video.statistics.digg_count,
            "comment_count": video.statistics.comment_count,
            "share_count": video.statistics.share_count,
            "play_count": video.statistics.play_count,
        },
        "media_type": media_type,
        "media_urls": media_urls.clone(),
        "raw_media_type": media_type,
        "cover_url": python_cover_url(video),
        "images": video.image_urls.clone().unwrap_or_default(),
        "videos": media_urls,
        "bgm_url": python_music_play_url(video),
    })
}

pub fn python_recommended_video(video: &VideoInfo) -> serde_json::Value {
    let media_type = python_media_type(video);
    let media_urls = python_media_urls(video);
    let music = python_music_info(video);
    let bgm_url = python_music_play_url(video);

    serde_json::json!({
        "aweme_id": video.aweme_id,
        "desc": video.desc,
        "create_time": video.create_time,
        "media_type": media_type,
        "media_urls": media_urls,
        "bgm_url": bgm_url,
        "images": video.image_urls.clone().unwrap_or_default(),
        "live_photos": video.live_photo_urls.clone().unwrap_or_default(),
        "has_live_photo": video.has_live_photo,
        "is_image": video.is_image,
        "raw_media_type": media_type,
        "author": {
            "uid": video.author.uid,
            "nickname": video.author.nickname,
            "avatar_thumb": video.author.avatar_thumb,
            "sec_uid": video.author.sec_uid,
        },
        "statistics": {
            "digg_count": video.statistics.digg_count,
            "comment_count": video.statistics.comment_count,
            "share_count": video.statistics.share_count,
            "play_count": video.statistics.play_count,
        },
        "video": {
            "cover": video.video.cover,
            "dynamic_cover": video.video.dynamic_cover,
            "play_addr": video.video.play_addr,
            "width": video.video.width,
            "height": video.video.height,
            "duration": normalize_duration_seconds(video.video.duration),
        },
        "music": {
            "title": music["title"],
            "author": music["author"],
            "play_url": music["play_url"],
            "duration": music["duration"],
            "cover": video.music.as_ref().map(|item| item.cover_thumb.clone()).unwrap_or_default(),
        }
    })
}

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
                .and_then(|video| video.get("download_addr")),
            payload
                .get("video")
                .and_then(|video| video.get("play_addr")),
            payload
                .get("video")
                .and_then(|video| video.get("preview_addr")),
            payload.get("download_addr"),
            payload.get("play_addr"),
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

fn extract_payload_url(value: &serde_json::Value) -> Option<String> {
    if let Some(url) = value.as_str() {
        let url = url.trim();
        return (!url.is_empty()).then(|| url.to_string());
    }

    if let Some(values) = value.as_array() {
        for value in values {
            if let Some(url) = extract_payload_url(value) {
                return Some(url);
            }
        }
        return None;
    }

    for key in ["url", "play_url", "play_addr", "download_addr", "url_list"] {
        if let Some(url) = value.get(key).and_then(extract_payload_url) {
            return Some(url);
        }
    }

    None
}

fn push_download_item(
    items: &mut Vec<DownloadMediaItem>,
    media_type: &str,
    url: &str,
    fallback_type: &str,
) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }

    let media_type = match media_type.trim().to_lowercase().as_str() {
        MEDIA_TYPE_IMAGE => MEDIA_TYPE_IMAGE.to_string(),
        MEDIA_TYPE_LIVE_PHOTO | "livephoto" => MEDIA_TYPE_LIVE_PHOTO.to_string(),
        MEDIA_TYPE_VIDEO => MEDIA_TYPE_VIDEO.to_string(),
        MEDIA_TYPE_AUDIO => MEDIA_TYPE_AUDIO.to_string(),
        _ => infer_download_item_type(url, fallback_type),
    };

    if media_type == MEDIA_TYPE_AUDIO {
        return;
    }
    if items
        .iter()
        .any(|item| item.url == url && item.r#type == media_type)
    {
        return;
    }

    items.push(DownloadMediaItem {
        r#type: media_type,
        url: url.to_string(),
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
        if let Some(url) = DouyinClient::get_no_watermark_url(video) {
            items.push(DownloadMediaItem {
                r#type: MEDIA_TYPE_VIDEO.to_string(),
                url,
            });
        } else if !video.video.play_addr.trim().is_empty() {
            items.push(DownloadMediaItem {
                r#type: MEDIA_TYPE_VIDEO.to_string(),
                url: video.video.play_addr.clone(),
            });
        }
    }

    items
}

pub fn media_type_from_payload_or_items(
    raw_media_type: &str,
    items: &[DownloadMediaItem],
) -> MediaType {
    if !raw_media_type.is_empty() {
        return match raw_media_type {
            MEDIA_TYPE_IMAGE => MediaType::Image,
            MEDIA_TYPE_LIVE_PHOTO => MediaType::LivePhoto,
            MEDIA_TYPE_MIXED => MediaType::Mixed,
            MEDIA_TYPE_AUDIO => MediaType::Audio,
            _ => MediaType::Video,
        };
    }

    let has_live = items
        .iter()
        .any(|item| item.r#type == MEDIA_TYPE_LIVE_PHOTO);
    let has_image = items.iter().any(|item| item.r#type == MEDIA_TYPE_IMAGE);

    if has_live && has_image {
        MediaType::Mixed
    } else if has_live {
        MediaType::LivePhoto
    } else if has_image {
        MediaType::Image
    } else {
        MediaType::Video
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AuthorInfo, Statistics, Status, VideoData};

    fn sample_video_with_images(
        image_urls: Vec<String>,
        live_photo_urls: Vec<String>,
    ) -> VideoInfo {
        let is_image = !image_urls.is_empty();
        let has_live_photo = !live_photo_urls.is_empty();
        VideoInfo {
            aweme_id: "123".to_string(),
            desc: "test".to_string(),
            create_time: 0,
            author: AuthorInfo::default(),
            video: VideoData {
                play_addr: if !is_image && !has_live_photo {
                    "https://example.com/play".to_string()
                } else {
                    "".to_string()
                },
                ..Default::default()
            },
            statistics: Statistics::default(),
            status: Status::default(),
            image_urls: Some(image_urls),
            is_image,
            media_type: MediaType::Image,
            has_live_photo,
            live_photo_urls: Some(live_photo_urls),
            music: None,
            raw_media_type: None,
            text_extra: None,
        }
    }

    #[test]
    fn normalizes_duration_seconds() {
        assert_eq!(normalize_duration_seconds(0), 0);
        assert_eq!(normalize_duration_seconds(-5), 0);
        assert_eq!(normalize_duration_seconds(50), 50);
        assert_eq!(normalize_duration_seconds(500), 5); // /100
        assert_eq!(normalize_duration_seconds(5_000), 5); // /1000
        assert_eq!(normalize_duration_seconds(500_000), 5); // /100000
    }

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
    fn determines_python_media_type() {
        let video_only = sample_video_with_images(vec![], vec![]);
        assert_eq!(python_media_type(&video_only), "video");

        let images_only = sample_video_with_images(vec!["a.jpg".into()], vec![]);
        assert_eq!(python_media_type(&images_only), "image");

        let live_only = sample_video_with_images(vec![], vec!["a.mp4".into()]);
        assert_eq!(python_media_type(&live_only), "live_photo");

        let mixed = sample_video_with_images(vec!["a.jpg".into()], vec!["a.mp4".into()]);
        assert_eq!(python_media_type(&mixed), "mixed");
    }

    #[test]
    fn resolves_media_type_from_payload_or_items() {
        assert_eq!(
            media_type_from_payload_or_items("image", &[]),
            MediaType::Image
        );
        assert_eq!(
            media_type_from_payload_or_items("live_photo", &[]),
            MediaType::LivePhoto
        );
        assert_eq!(
            media_type_from_payload_or_items("mixed", &[]),
            MediaType::Mixed
        );
        assert_eq!(
            media_type_from_payload_or_items(
                "",
                &[DownloadMediaItem {
                    r#type: "image".into(),
                    url: "".into()
                }]
            ),
            MediaType::Image
        );
        assert_eq!(
            media_type_from_payload_or_items(
                "",
                &[DownloadMediaItem {
                    r#type: "live_photo".into(),
                    url: "".into()
                }]
            ),
            MediaType::LivePhoto
        );
        assert_eq!(
            media_type_from_payload_or_items(
                "",
                &[
                    DownloadMediaItem {
                        r#type: "live_photo".into(),
                        url: "".into()
                    },
                    DownloadMediaItem {
                        r#type: "image".into(),
                        url: "".into()
                    }
                ]
            ),
            MediaType::Mixed
        );
    }
}
