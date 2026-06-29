use crate::api::VideoInfo;
use crate::media_utils_types::is_dash_video_only_url;

pub fn normalize_video_duration_seconds(value: i64) -> i64 {
    if value <= 0 {
        return 0;
    }

    if value >= 1_000 {
        return std::cmp::max(1, (value as f64 / 1_000.0).round() as i64);
    }

    std::cmp::max(1, value)
}

pub fn normalize_music_duration_seconds(value: i64) -> i64 {
    if value <= 0 {
        return 0;
    }

    if value >= 1_000 {
        return std::cmp::max(1, (value as f64 / 1_000.0).round() as i64);
    }
    if value >= 100 {
        return std::cmp::max(1, (value as f64 / 100.0).round() as i64);
    }

    std::cmp::max(1, value)
}

pub(crate) fn clean_video_download_url(url: &str) -> String {
    url.trim()
        .replace("watermark=1", "watermark=0")
        .replace("playwm", "play")
}

pub(crate) fn is_watermark_video_url(url: &str) -> bool {
    let normalized = url.trim().to_ascii_lowercase();
    normalized.contains("playwm")
        || normalized.contains("watermark=1")
        || normalized.contains("/aweme/v1/playwm")
}

pub(crate) fn no_watermark_video_url(video: &VideoInfo) -> Option<String> {
    for url in [
        video.video.play_addr_h264.as_deref(),
        Some(video.video.play_addr.as_str()),
        video.video.download_addr.as_deref(),
        video.video.play_addr_lowbr.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let clean_url = clean_video_download_url(url);
        if !clean_url.is_empty()
            && !is_watermark_video_url(&clean_url)
            && !is_dash_video_only_url(&clean_url)
        {
            return Some(clean_url);
        }
    }

    // Fallback: If no non-DASH urls found, allow dash_addr as a fallback
    if let Some(dash_url) = &video.video.dash_addr {
        let clean_url = clean_video_download_url(dash_url);
        if !clean_url.is_empty() && !is_watermark_video_url(&clean_url) {
            return Some(clean_url);
        }
    }

    // Secondary fallback: Allow any valid video URL even if it's DASH-only
    for url in [
        video.video.play_addr_h264.as_deref(),
        Some(video.video.play_addr.as_str()),
        video.video.download_addr.as_deref(),
        video.video.play_addr_lowbr.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let clean_url = clean_video_download_url(url);
        if !clean_url.is_empty() && !is_watermark_video_url(&clean_url) {
            return Some(clean_url);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_video_duration_seconds() {
        assert_eq!(normalize_video_duration_seconds(0), 0);
        assert_eq!(normalize_video_duration_seconds(-5), 0);
        assert_eq!(normalize_video_duration_seconds(50), 50);
        assert_eq!(normalize_video_duration_seconds(500), 500);
        assert_eq!(normalize_video_duration_seconds(5_000), 5);
        assert_eq!(normalize_video_duration_seconds(500_000), 500);
    }

    #[test]
    fn normalizes_music_duration_seconds() {
        assert_eq!(normalize_music_duration_seconds(0), 0);
        assert_eq!(normalize_music_duration_seconds(-5), 0);
        assert_eq!(normalize_music_duration_seconds(50), 50);
        assert_eq!(normalize_music_duration_seconds(500), 5);
        assert_eq!(normalize_music_duration_seconds(5_000), 5);
    }
}
