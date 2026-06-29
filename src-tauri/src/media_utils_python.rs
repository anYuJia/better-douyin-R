use crate::api::{UserInfo, VideoInfo};
use crate::media_utils_normalize::{
    no_watermark_video_url, normalize_music_duration_seconds, normalize_video_duration_seconds,
};
use crate::media_utils_types::{
    python_media_type, MEDIA_TYPE_IMAGE, MEDIA_TYPE_LIVE_PHOTO, MEDIA_TYPE_VIDEO,
};

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

pub fn python_user_value(user: &UserInfo) -> serde_json::Value {
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
    let bgm_url = python_music_play_url(video);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AuthorInfo, MediaType, MusicInfo, Statistics, Status, VideoData};

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
    fn python_media_urls_do_not_fallback_to_dash_video_only() {
        let mut video = sample_video_with_images(vec![], vec![]);
        video.video.play_addr.clear();
        video.video.dash_addr = Some("https://example.com/media-video-avc1".into());

        let urls = python_media_urls(&video);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0]["url"], "https://example.com/media-video-avc1");
    }

    #[test]
    fn python_video_summary_does_not_use_video_url_as_bgm_url() {
        let mut video = sample_video_with_images(vec![], vec![]);
        video.video.play_addr = "https://example.com/video.mp4".to_string();

        let summary = python_video_summary(&video, true, true);

        assert_eq!(summary["bgm_url"], "");
        assert_eq!(summary["music"]["play_url"], "");
    }

    #[test]
    fn python_video_summary_uses_music_play_url_as_bgm_url() {
        let mut video = sample_video_with_images(vec![], vec![]);
        video.video.play_addr = "https://example.com/video.mp4".to_string();
        video.music = Some(MusicInfo {
            play_url: Some("https://example.com/music.mp3".to_string()),
            ..Default::default()
        });

        let summary = python_video_summary(&video, true, true);

        assert_eq!(summary["bgm_url"], "https://example.com/music.mp3");
        assert_eq!(
            summary["music"]["play_url"],
            "https://example.com/music.mp3"
        );
    }
}
