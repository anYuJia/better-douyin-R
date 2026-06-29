use crate::api::{DownloadMediaItem, MediaType, VideoInfo};

pub const MEDIA_TYPE_VIDEO: &str = "video";
pub const MEDIA_TYPE_IMAGE: &str = "image";
pub const MEDIA_TYPE_LIVE_PHOTO: &str = "live_photo";
pub const MEDIA_TYPE_MIXED: &str = "mixed";
pub const MEDIA_TYPE_AUDIO: &str = "audio";

pub fn is_dash_video_only_url(url: &str) -> bool {
    let normalized = url.trim().to_ascii_lowercase();
    normalized.contains("media-video") || normalized.contains("media_video")
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
    } else if video
        .video
        .dash_addr
        .as_ref()
        .map(|url| !url.trim().is_empty())
        .unwrap_or(false)
        || !video.video.play_addr.is_empty()
    {
        MEDIA_TYPE_VIDEO
    } else {
        "unknown"
    }
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
            is_liked: false,
            is_collected: false,
            live_photo_urls: Some(live_photo_urls),
            music: None,
            raw_media_type: None,
            text_extra: None,
        }
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
                    url: "".into(),
                    fallback_urls: Vec::new()
                }]
            ),
            MediaType::Image
        );
        assert_eq!(
            media_type_from_payload_or_items(
                "",
                &[DownloadMediaItem {
                    r#type: "live_photo".into(),
                    url: "".into(),
                    fallback_urls: Vec::new()
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
                        url: "".into(),
                        fallback_urls: Vec::new()
                    },
                    DownloadMediaItem {
                        r#type: "image".into(),
                        url: "".into(),
                        fallback_urls: Vec::new()
                    }
                ]
            ),
            MediaType::Mixed
        );
    }
}
