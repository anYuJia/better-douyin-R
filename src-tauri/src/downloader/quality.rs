//! 下载质量选择

use crate::api::types::VideoInfo;
use crate::media_utils::is_dash_video_only_url;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadQuality {
    Auto,
    Highest,
    H264,
    Smallest,
    TargetHeight(i32),
}

impl DownloadQuality {
    pub(crate) fn from_config(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "highest" => Self::Highest,
            "h264" => Self::H264,
            "smallest" => Self::Smallest,
            "480p" | "p480" => Self::TargetHeight(480),
            "720p" | "p720" => Self::TargetHeight(720),
            "1080p" | "p1080" => Self::TargetHeight(1080),
            "2k" | "1440p" | "p1440" => Self::TargetHeight(1440),
            "4k" | "2160p" | "p2160" => Self::TargetHeight(2160),
            _ => Self::Auto,
        }
    }
}
pub(crate) fn parse_quality_height_from_text(value: &str) -> i32 {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.contains("4k") || normalized.contains("uhd") || normalized.contains("2160") {
        return 2160;
    }
    if normalized.contains("2k") || normalized.contains("qhd") || normalized.contains("1440") {
        return 1440;
    }

    for token in normalized.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if let Some(number) = token.strip_suffix('p') {
            if let Ok(height) = number.parse::<i32>() {
                if (240..=4320).contains(&height) {
                    return height;
                }
            }
        }
        if let Ok(height) = token.parse::<i32>() {
            if (240..=4320).contains(&height) {
                return height;
            }
        }
    }

    0
}

pub(crate) fn nearest_standard_quality_height(value: i32) -> i32 {
    if value <= 0 {
        return 0;
    }

    const STANDARD_HEIGHTS: [i32; 9] = [4320, 2160, 1440, 1080, 720, 540, 480, 360, 240];
    let nearest = STANDARD_HEIGHTS
        .into_iter()
        .min_by_key(|height| (height - value).abs())
        .unwrap_or(0);
    let tolerance = std::cmp::max(24, nearest * 12 / 100);
    if (nearest - value).abs() <= tolerance {
        return nearest;
    }

    if (240..=4320).contains(&value) {
        value
    } else {
        0
    }
}

pub(crate) fn standard_quality_height_from_dimension(value: i32) -> i32 {
    if value <= 0 {
        return 0;
    }

    const STANDARD_HEIGHTS: [i32; 9] = [4320, 2160, 1440, 1080, 720, 540, 480, 360, 240];
    let nearest = STANDARD_HEIGHTS
        .into_iter()
        .min_by_key(|height| (height - value).abs())
        .unwrap_or(0);
    let tolerance = std::cmp::max(16, nearest * 4 / 100);
    if (nearest - value).abs() <= tolerance {
        nearest
    } else {
        0
    }
}

pub(crate) fn long_side_quality_height(value: i32) -> i32 {
    if value <= 0 {
        return 0;
    }

    const LONG_SIDE_TO_QUALITY: [(i32, i32); 7] = [
        (3840, 2160),
        (2560, 1440),
        (1920, 1080),
        (1280, 720),
        (960, 540),
        (854, 480),
        (852, 480),
    ];
    for (long_side, quality_height) in LONG_SIDE_TO_QUALITY {
        let tolerance = std::cmp::max(24, long_side * 4 / 100);
        if (value - long_side).abs() <= tolerance {
            return quality_height;
        }
    }

    0
}

pub(crate) fn dimension_quality_height(width: i32, height: i32) -> i32 {
    let width = width.max(0);
    let height = height.max(0);

    if width > 0 && height > 0 {
        let measured = [
            standard_quality_height_from_dimension(width),
            standard_quality_height_from_dimension(height),
            long_side_quality_height(width),
            long_side_quality_height(height),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        if measured > 0 {
            return measured;
        }
        return nearest_standard_quality_height(width.max(height));
    }

    let value = width.max(height);
    if value <= 0 {
        return 0;
    }

    let standard_height = standard_quality_height_from_dimension(value);
    if standard_height > 0 {
        return standard_height;
    }

    let long_side_height = long_side_quality_height(value);
    if long_side_height > 0 {
        return long_side_height;
    }

    nearest_standard_quality_height(value)
}

pub(crate) fn bit_rate_metric(bit_rate: &crate::api::types::BitRateInfo) -> i64 {
    if bit_rate.data_size > 0 {
        return bit_rate.data_size;
    }
    if bit_rate.bit_rate > 0 {
        return bit_rate.bit_rate;
    }
    if bit_rate.quality_type > 0 {
        return bit_rate.quality_type as i64;
    }
    if bit_rate.width > 0 && bit_rate.height > 0 {
        return bit_rate.width as i64 * bit_rate.height as i64;
    }
    0
}

pub(crate) fn bit_rate_height(bit_rate: &crate::api::types::BitRateInfo) -> i32 {
    let mut height = 0;
    let gear_height = parse_quality_height_from_text(&bit_rate.gear_name);
    if gear_height > 0 {
        height = height.max(gear_height);
    }
    match bit_rate.quality_type {
        72 | 73 => height = height.max(2160),
        _ => {}
    }
    height.max(dimension_quality_height(bit_rate.width, bit_rate.height))
}

#[derive(Debug, Clone)]
pub(crate) struct VideoCandidate {
    url: String,
    metric: i64,
    height: i32,
    is_h264: bool,
    is_quality_candidate: bool,
    is_download_addr: bool,
    is_lowbr: bool,
    is_watermark: bool,
}
pub(crate) fn collect_video_candidates(video: &VideoInfo) -> Vec<VideoCandidate> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let top_level_height = dimension_quality_height(video.video.width, video.video.height)
        .max(parse_quality_height_from_text(&video.video.ratio));
    let lowbr_height = if top_level_height > 0 {
        top_level_height.min(480)
    } else {
        480
    };

    let mut dash_fallbacks = Vec::new();
    let mut push_candidate = |url: Option<String>,
                              metric: i64,
                              height: i32,
                              is_h264: bool,
                              is_quality_candidate: bool,
                              is_download_addr: bool,
                              is_lowbr: bool| {
        let Some(url) = url else {
            return;
        };
        let url = clean_video_download_url(&url);
        if url.trim().is_empty() || !seen.insert(url.clone()) {
            return;
        }
        let is_dash = is_dash_video_only_url(&url);
        let candidate = VideoCandidate {
            is_watermark: is_watermark_url(&url),
            url,
            metric,
            height,
            is_h264,
            is_quality_candidate,
            is_download_addr,
            is_lowbr,
        };
        if is_dash {
            dash_fallbacks.push(candidate);
        } else {
            candidates.push(candidate);
        }
    };

    push_candidate(
        video.video.download_addr.clone(),
        0,
        top_level_height,
        false,
        false,
        true,
        false,
    );
    push_candidate(
        video.video.play_addr_h264.clone(),
        0,
        top_level_height,
        true,
        false,
        false,
        false,
    );
    push_candidate(
        video.video.play_addr_lowbr.clone(),
        1,
        lowbr_height,
        true,
        false,
        false,
        true,
    );

    if let Some(bit_rates) = &video.video.bit_rate {
        for bit_rate in bit_rates {
            let metric = bit_rate_metric(bit_rate);
            let height = bit_rate_height(bit_rate);
            let h264_metric = if metric > 0 { metric + 1 } else { 0 };

            push_candidate(
                bit_rate.play_addr_h264.clone(),
                h264_metric,
                height,
                true,
                true,
                false,
                false,
            );
            push_candidate(
                bit_rate.play_addr.clone(),
                metric,
                height,
                !bit_rate.is_h265,
                true,
                false,
                false,
            );
        }
    }

    push_candidate(
        video.video.preview_addr.clone(),
        0,
        top_level_height,
        false,
        false,
        false,
        false,
    );
    push_candidate(
        Some(video.video.play_addr.clone()),
        0,
        top_level_height,
        false,
        false,
        false,
        false,
    );

    if candidates.is_empty() {
        candidates = dash_fallbacks;
    }

    candidates
}

pub(crate) fn available_video_quality_height(video: &VideoInfo) -> i32 {
    collect_video_candidates(video)
        .into_iter()
        .filter(|candidate| {
            !candidate.is_watermark
                && !candidate.is_download_addr
                && !candidate.is_lowbr
                && candidate.height > 0
        })
        .map(|candidate| candidate.height)
        .max()
        .unwrap_or(0)
}

pub(crate) fn video_quality_candidate_count(video: &VideoInfo) -> usize {
    collect_video_candidates(video)
        .into_iter()
        .filter(|candidate| {
            !candidate.is_watermark
                && candidate.is_quality_candidate
                && !candidate.is_download_addr
                && !candidate.is_lowbr
        })
        .count()
}

pub fn video_quality_diagnostic(video: &VideoInfo, quality: &str) -> serde_json::Value {
    let quality_mode = DownloadQuality::from_config(quality);
    let candidates = collect_video_candidates(video);
    let clean_candidates = candidates
        .iter()
        .filter(|candidate| !candidate.is_watermark)
        .collect::<Vec<_>>();
    let ordered_urls = ordered_video_urls(video, quality_mode);
    let selected_url = ordered_urls.first().cloned().unwrap_or_default();
    let selected = clean_candidates
        .iter()
        .find(|candidate| candidate.url == selected_url);
    let supported_heights = {
        let mut heights = clean_candidates
            .iter()
            .filter(|candidate| {
                candidate.height > 0 && !candidate.is_download_addr && !candidate.is_lowbr
            })
            .map(|candidate| candidate.height)
            .collect::<Vec<_>>();
        heights.sort_unstable();
        heights.dedup();
        heights
    };

    serde_json::json!({
        "aweme_id": video.aweme_id,
        "requested_quality": quality,
        "selected_url": selected_url,
        "selected": selected.map(|candidate| serde_json::json!({
            "height": candidate.height,
            "metric": candidate.metric,
            "is_h264": candidate.is_h264,
            "is_quality_candidate": candidate.is_quality_candidate,
            "is_download_addr": candidate.is_download_addr,
            "is_lowbr": candidate.is_lowbr,
            "is_watermark": candidate.is_watermark,
        })),
        "supported_heights": supported_heights,
        "ordered_urls": ordered_urls,
        "candidates": candidates.into_iter().map(|candidate| serde_json::json!({
            "url": candidate.url,
            "height": candidate.height,
            "metric": candidate.metric,
            "is_h264": candidate.is_h264,
            "is_quality_candidate": candidate.is_quality_candidate,
            "is_download_addr": candidate.is_download_addr,
            "is_lowbr": candidate.is_lowbr,
            "is_watermark": candidate.is_watermark,
        })).collect::<Vec<_>>(),
    })
}

pub(crate) fn clean_video_download_url(url: &str) -> String {
    url.trim()
        .replace("watermark=1", "watermark=0")
        .replace("playwm", "play")
}

pub(crate) fn is_watermark_url(url: &str) -> bool {
    let normalized = url.trim().to_ascii_lowercase();
    normalized.contains("playwm")
        || normalized.contains("watermark=1")
        || normalized.contains("/aweme/v1/playwm")
}

pub(crate) fn select_video_url(video: &VideoInfo, quality: DownloadQuality) -> Option<String> {
    ordered_video_urls(video, quality).into_iter().next()
}

pub(crate) fn best_target_candidate<'a>(
    candidates: &[&'a VideoCandidate],
    target_height: i32,
) -> Option<&'a VideoCandidate> {
    let explicit_measured = candidates
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.is_quality_candidate && candidate.height > 0 && !candidate.is_download_addr
        })
        .collect::<Vec<_>>();
    let fallback_measured;
    let measured = if explicit_measured.is_empty() {
        fallback_measured = candidates
            .iter()
            .copied()
            .filter(|candidate| candidate.height > 0 && !candidate.is_download_addr)
            .collect::<Vec<_>>();
        &fallback_measured
    } else {
        &explicit_measured
    };

    if let Some(candidate) = measured
        .iter()
        .copied()
        .filter(|candidate| candidate.height <= target_height)
        .max_by_key(|candidate| {
            (
                candidate.height,
                if candidate.is_h264 { 1 } else { 0 },
                candidate.metric,
            )
        })
    {
        return Some(candidate);
    }

    measured
        .iter()
        .copied()
        .filter(|candidate| candidate.height > target_height)
        .min_by(|a, b| {
            a.height
                .cmp(&b.height)
                .then_with(|| (if a.is_h264 { 0 } else { 1 }).cmp(&(if b.is_h264 { 0 } else { 1 })))
                .then_with(|| b.metric.cmp(&a.metric))
        })
}

pub(crate) fn ordered_video_urls(video: &VideoInfo, quality: DownloadQuality) -> Vec<String> {
    let candidates = collect_video_candidates(video);

    if candidates.is_empty() {
        return Vec::new();
    }
    let clean_candidates = candidates
        .iter()
        .filter(|candidate| !candidate.is_watermark)
        .collect::<Vec<_>>();
    if clean_candidates.is_empty() {
        return Vec::new();
    }

    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |candidate: Option<&VideoCandidate>| {
        if let Some(candidate) = candidate {
            if seen.insert(candidate.url.clone()) {
                ordered.push(candidate.url.clone());
            }
        }
    };

    let download_addr = clean_candidates
        .iter()
        .copied()
        .find(|c| c.is_download_addr);
    let h264_best = clean_candidates
        .iter()
        .copied()
        .filter(|c| c.is_h264 && !c.is_lowbr)
        .max_by_key(|c| c.metric);
    let highest_metric = clean_candidates
        .iter()
        .copied()
        .filter(|c| c.metric > 0 && !c.is_download_addr && !c.is_lowbr)
        .max_by_key(|c| c.metric);
    let lowbr = clean_candidates.iter().copied().find(|c| c.is_lowbr);
    let smallest_metric = clean_candidates
        .iter()
        .copied()
        .filter(|c| c.metric > 0)
        .min_by_key(|c| c.metric);
    let first = clean_candidates.first().copied();

    match quality {
        DownloadQuality::Auto => {
            push(h264_best);
            push(highest_metric);
            push(download_addr);
            push(first);
        }
        DownloadQuality::Highest => {
            push(highest_metric);
            push(h264_best);
            push(download_addr);
            push(first);
        }
        DownloadQuality::H264 => {
            push(h264_best);
            push(highest_metric);
            push(download_addr);
            push(first);
        }
        DownloadQuality::Smallest => {
            push(lowbr);
            push(smallest_metric);
            push(h264_best);
            push(first);
        }
        DownloadQuality::TargetHeight(target_height) => {
            let target_best = best_target_candidate(&clean_candidates, target_height);
            let target_h264 = target_best.and_then(|selected| {
                clean_candidates.iter().copied().find(|candidate| {
                    candidate.is_h264
                        && candidate.height == selected.height
                        && !candidate.is_lowbr
                        && !candidate.is_download_addr
                })
            });
            push(target_best);
            push(target_h264);
            push(highest_metric);
            push(h264_best);
            push(download_addr);
            push(first);
        }
    }

    let mut rest = clean_candidates.to_vec();
    match quality {
        DownloadQuality::TargetHeight(target_height) => {
            rest.sort_by(|a, b| {
                let a_delta = if a.height > 0 {
                    (a.height - target_height).abs()
                } else {
                    i32::MAX
                };
                let b_delta = if b.height > 0 {
                    (b.height - target_height).abs()
                } else {
                    i32::MAX
                };
                a_delta
                    .cmp(&b_delta)
                    .then_with(|| b.height.cmp(&a.height))
                    .then_with(|| b.metric.cmp(&a.metric))
            });
        }
        _ => {
            rest.sort_by_key(|candidate| std::cmp::Reverse(candidate.metric));
        }
    }
    for candidate in rest {
        push(Some(candidate));
    }

    let quality_label = match quality {
        DownloadQuality::Auto => "auto".to_string(),
        DownloadQuality::Highest => "highest".to_string(),
        DownloadQuality::H264 => "h264".to_string(),
        DownloadQuality::Smallest => "smallest".to_string(),
        DownloadQuality::TargetHeight(height) => format!("{height}p"),
    };
    let candidate_heights = clean_candidates
        .iter()
        .filter(|candidate| !candidate.is_download_addr && !candidate.is_lowbr)
        .map(|candidate| {
            format!(
                "{}:{}:{}:{}",
                candidate.height,
                candidate.metric,
                if candidate.is_h264 { "h264" } else { "main" },
                if candidate.is_quality_candidate {
                    "bitrate"
                } else {
                    "top"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let selected = ordered.first().and_then(|url| {
        clean_candidates
            .iter()
            .find(|candidate| candidate.url == *url)
    });
    log::debug!(
        "video download quality selected: aweme_id={} quality={} candidates=[{}] selected_height={} selected_metric={} selected_h264={} selected_quality_candidate={}",
        video.aweme_id,
        quality_label,
        candidate_heights,
        selected.map(|candidate| candidate.height).unwrap_or(0),
        selected.map(|candidate| candidate.metric).unwrap_or(0),
        selected.map(|candidate| candidate.is_h264).unwrap_or(false),
        selected
            .map(|candidate| candidate.is_quality_candidate)
            .unwrap_or(false)
    );

    ordered
}
