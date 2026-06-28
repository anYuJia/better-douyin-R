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
pub use crate::media_utils_download_items::{
    download_media_items_from_video, download_media_type_from_payload,
    infer_download_item_type, parse_download_media_items,
};
pub use crate::media_utils_python::{
    python_cover_url, python_music_info, python_music_play_url, python_recommended_video,
    python_status_value, python_user_value, python_video_detail_value, python_video_summary,
    python_media_urls,
};























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
    fn python_media_urls_do_not_fallback_to_dash_video_only() {
        let mut video = sample_video_with_images(vec![], vec![]);
        video.video.play_addr.clear();
        video.video.dash_addr = Some("https://example.com/media-video-avc1".into());

        assert!(python_media_urls(&video).is_empty());
    }
}
