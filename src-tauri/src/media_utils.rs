//! 媒体工具模块

use crate::api::{DownloadMediaItem, MediaType, VideoInfo};

pub use crate::media_utils_types::{
    MEDIA_TYPE_AUDIO, MEDIA_TYPE_IMAGE, MEDIA_TYPE_LIVE_PHOTO, MEDIA_TYPE_MIXED, MEDIA_TYPE_VIDEO,
    is_dash_video_only_url, media_type_from_payload_or_items, python_media_type,
};
pub use crate::media_utils_normalize::{
    normalize_music_duration_seconds, normalize_video_duration_seconds,
};
pub(crate) use crate::media_utils_normalize::{
    clean_video_download_url, is_watermark_video_url, no_watermark_video_url,
};
pub(crate) use crate::media_utils_extract::extract_payload_url;







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

    if items.is_empty() {
        if let Some(url) = no_watermark_video_url(video) {
            items.push(serde_json::json!({ "type": MEDIA_TYPE_VIDEO, "url": url }));
        }
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
        "duration": normalize_music_duration_seconds(video.music.as_ref().map(|music| music.duration).unwrap_or(0)),
    })
}

pub fn python_status_value(video: &VideoInfo) -> serde_json::Value {
    serde_json::json!({
        "is_delete": video.status.is_delete,
        "private_status": video.status.private_status,
        "review_status": video.status.review_status,
        "with_goods": video.status.with_goods,
        "is_prohibited": video.status.is_prohibited,
    })
}

pub fn python_user_value(user: &crate::api::UserInfo) -> serde_json::Value {
    serde_json::json!({
        "nickname": user.nickname,
        "unique_id": user.unique_id,
        "uid": user.uid,
        "follower_count": user.follower_count,
        "following_count": user.following_count,
        "total_favorited": user.total_favorited,
        "aweme_count": user.aweme_count,
        "favoriting_count": user.favoriting_count,
        "is_follow": user.is_follow,
        "follow_status": user.follow_status,
        "signature": user.signature,
        "sec_uid": user.sec_uid,
        "avatar_thumb": user.avatar_thumb,
        "avatar_medium": user.avatar_medium,
        "avatar_larger": user.avatar_larger,
        "verify_status": user.verify_status,
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
        "collect_count": video.statistics.collect_count,
        "is_liked": video.is_liked,
        "is_collected": video.is_collected,
        "statistics": {
            "digg_count": video.statistics.digg_count,
            "comment_count": video.statistics.comment_count,
            "share_count": video.statistics.share_count,
            "collect_count": video.statistics.collect_count,
            "play_count": video.statistics.play_count,
        },
        "cover_url": python_cover_url(video),
        "media_type": media_type,
        "status": python_status_value(video),
        "media_urls": media_urls,
        "bgm_url": bgm_url,
        "author": {
            "nickname": video.author.nickname,
            "avatar_thumb": video.author.avatar_thumb,
            "sec_uid": video.author.sec_uid,
        }
    });

    if include_duration {
        value["duration"] =
            serde_json::json!(normalize_video_duration_seconds(video.video.duration));
        value["duration_unit"] = serde_json::json!("seconds");
    }

    if include_music {
        let music = python_music_info(video);
        value["music"] = music.clone();
        value["music_title"] = music["title"].clone();
        value["music_author"] = music["author"].clone();
        value["music_url"] = music["play_url"].clone();
        value["music_duration"] = music["duration"].clone();
    }

    value["video"] = serde_json::json!({
        "cover": video.video.cover,
        "dynamic_cover": video.video.dynamic_cover,
        "origin_cover": video.video.origin_cover,
        "preview_addr": video.video.preview_addr,
        "play_addr": video.video.play_addr,
        "dash_addr": video.video.dash_addr,
        "audio_addr": video.video.audio_addr,
        "play_addr_h264": video.video.play_addr_h264,
        "play_addr_lowbr": video.video.play_addr_lowbr,
        "download_addr": video.video.download_addr,
        "width": video.video.width,
        "height": video.video.height,
        "duration": normalize_video_duration_seconds(video.video.duration),
        "duration_unit": "seconds",
        "ratio": video.video.ratio,
        "bit_rate": video.video.bit_rate,
    });

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
            "collect_count": video.statistics.collect_count,
            "play_count": video.statistics.play_count,
        },
        "status": python_status_value(video),
        "media_type": media_type,
        "media_urls": media_urls.clone(),
        "raw_media_type": media_type,
        "cover_url": python_cover_url(video),
        "is_liked": video.is_liked,
        "is_collected": video.is_collected,
        "images": video.image_urls.clone().unwrap_or_default(),
        "videos": media_urls,
        "bgm_url": python_music_play_url(video),
        "video": {
            "cover": video.video.cover,
            "dynamic_cover": video.video.dynamic_cover,
            "origin_cover": video.video.origin_cover,
            "preview_addr": video.video.preview_addr,
            "play_addr": video.video.play_addr,
            "dash_addr": video.video.dash_addr,
            "audio_addr": video.video.audio_addr,
            "play_addr_h264": video.video.play_addr_h264,
            "play_addr_lowbr": video.video.play_addr_lowbr,
            "download_addr": video.video.download_addr,
            "width": video.video.width,
            "height": video.video.height,
            "duration": normalize_video_duration_seconds(video.video.duration),
            "duration_unit": "seconds",
            "ratio": video.video.ratio,
            "bit_rate": video.video.bit_rate,
        },
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
        "is_liked": video.is_liked,
        "is_collected": video.is_collected,
        "is_image": video.is_image,
        "raw_media_type": media_type,
        "status": python_status_value(video),
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
            "collect_count": video.statistics.collect_count,
            "play_count": video.statistics.play_count,
        },
        "video": {
            "cover": video.video.cover,
            "dynamic_cover": video.video.dynamic_cover,
            "origin_cover": video.video.origin_cover,
            "preview_addr": video.video.preview_addr,
            "play_addr": video.video.play_addr,
            "dash_addr": video.video.dash_addr,
            "audio_addr": video.video.audio_addr,
            "play_addr_h264": video.video.play_addr_h264,
            "play_addr_lowbr": video.video.play_addr_lowbr,
            "download_addr": video.video.download_addr,
            "width": video.video.width,
            "height": video.video.height,
            "duration": normalize_video_duration_seconds(video.video.duration),
            "duration_unit": "seconds",
            "ratio": video.video.ratio,
            "bit_rate": video.video.bit_rate,
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
            is_liked: false,
            is_collected: false,
            live_photo_urls: Some(live_photo_urls),
            music: None,
            raw_media_type: None,
            text_extra: None,
        }
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
    fn python_media_urls_do_not_fallback_to_dash_video_only() {
        let mut video = sample_video_with_images(vec![], vec![]);
        video.video.play_addr.clear();
        video.video.dash_addr = Some("https://example.com/media-video-avc1".into());

        assert!(python_media_urls(&video).is_empty());
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
