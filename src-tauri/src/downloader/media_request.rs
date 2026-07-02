//! 媒体下载请求辅助

use crate::api::types::{DownloadMediaItem, VideoInfo};
use crate::api::DouyinClient;
use crate::config::AppConfig;
use crate::media_utils::is_dash_video_only_url;
use anyhow::{anyhow, Result};
use reqwest::header::{HeaderMap, CONTENT_TYPE};

use super::quality::{ordered_video_urls, DownloadQuality};

pub(crate) async fn request_media_with_fallback(
    client: &reqwest::Client,
    config: &AppConfig,
    aweme_id: &str,
    media: &DownloadMediaItem,
    headers: &HeaderMap,
) -> Result<(reqwest::Response, String)> {
    if media.r#type == "video" && is_dash_video_only_url(&media.url) {
        if aweme_id.trim().is_empty() {
            return Err(anyhow!("下载地址是无声音轨的视频分片，缺少作品ID无法刷新"));
        }

        let fallback_urls = fresh_media_download_urls(config, aweme_id, media.r#type.as_str())
            .await
            .unwrap_or_default();
        for url in &fallback_urls {
            if is_dash_video_only_url(url) {
                continue;
            }

            let fallback_response = client.get(url).headers(headers.clone()).send().await?;
            if fallback_response.status().is_success() {
                if is_unexpected_audio_response(media.r#type.as_str(), &fallback_response) {
                    log::warn!(
                        "refreshed video URL returned audio content, trying next candidate: aweme_id={}",
                        aweme_id
                    );
                    continue;
                }
                log::info!(
                    "download url refreshed from dash video-only source: aweme_id={}",
                    aweme_id
                );
                return Ok((fallback_response, url.clone()));
            }
        }

        // Try the original DASH video URL directly
        let response = client
            .get(&media.url)
            .headers(headers.clone())
            .send()
            .await?;

        if response.status().is_success() {
            if is_unexpected_audio_response(media.r#type.as_str(), &response) {
                log::warn!(
                    "dash video fallback returned audio content, trying refreshed DASH candidates: aweme_id={}",
                    aweme_id
                );
            } else {
                log::info!(
                    "allowing dash video-only download as final fallback: aweme_id={}",
                    aweme_id
                );
                return Ok((response, media.url.clone()));
            }
        }

        // Try other DASH fallback URLs
        for url in fallback_urls {
            let fallback_response = client.get(&url).headers(headers.clone()).send().await?;
            if fallback_response.status().is_success() {
                if is_unexpected_audio_response(media.r#type.as_str(), &fallback_response) {
                    log::warn!(
                        "refreshed DASH URL returned audio content, trying next candidate: aweme_id={}",
                        aweme_id
                    );
                    continue;
                }
                return Ok((fallback_response, url));
            }
        }

        return Err(anyhow!("没有可用的视频下载地址"));
    }

    let media_type = media.r#type.as_str();
    let mut candidate_urls = Vec::new();
    push_unique_url(&mut candidate_urls, &media.url);
    for url in &media.fallback_urls {
        push_unique_url(&mut candidate_urls, url);
    }

    let mut initial_status = None;
    let mut saw_audio_response = false;
    for candidate_url in &candidate_urls {
        let response = client
            .get(candidate_url)
            .headers(headers.clone())
            .send()
            .await?;
        let status = response.status();
        if initial_status.is_none() {
            initial_status = Some(status);
        }

        if status.is_success() {
            if !is_unexpected_audio_response(media_type, &response) {
                return Ok((response, candidate_url.clone()));
            }
            saw_audio_response = true;
            log::warn!(
                "视频链接返回音频内容，尝试下一个候选：aweme_id={} url={}",
                aweme_id,
                candidate_url
            );
        }
    }

    let initial_status = initial_status.unwrap_or(reqwest::StatusCode::BAD_REQUEST);
    if saw_audio_response {
        log::warn!(
            "video candidates returned audio content, refreshing download URL: aweme_id={}",
            aweme_id
        );
    }

    if aweme_id.trim().is_empty() {
        if initial_status.is_success() {
            return Err(anyhow!("下载地址返回了音频内容，缺少作品ID无法刷新"));
        }
        return Err(anyhow!("HTTP error: {}", initial_status));
    }

    let fallback_urls =
        match fresh_media_download_urls(config, aweme_id, media.r#type.as_str()).await {
            Ok(urls) => urls,
            Err(error) => {
                return Err(anyhow!(
                    "HTTP error: {}; refresh failed: {}",
                    initial_status,
                    error
                ));
            }
        };
    for url in &fallback_urls {
        if url == &media.url || is_dash_video_only_url(url) {
            continue;
        }

        let fallback_response = client.get(url).headers(headers.clone()).send().await?;
        if fallback_response.status().is_success() {
            if is_unexpected_audio_response(media.r#type.as_str(), &fallback_response) {
                log::warn!(
                    "refreshed URL returned audio content, trying next candidate: aweme_id={}",
                    aweme_id
                );
                continue;
            }
            log::info!(
                "download url refreshed after HTTP {}: aweme_id={}",
                initial_status,
                aweme_id
            );
            return Ok((fallback_response, url.clone()));
        }
    }

    // Secondary fallback: Allow any refreshed URLs even if they contain DASH media-video.
    for url in fallback_urls {
        if url == media.url {
            continue;
        }

        let fallback_response = client.get(&url).headers(headers.clone()).send().await?;
        if fallback_response.status().is_success() {
            if is_unexpected_audio_response(media.r#type.as_str(), &fallback_response) {
                log::warn!(
                    "secondary refreshed URL returned audio content, trying next candidate: aweme_id={}",
                    aweme_id
                );
                continue;
            }
            return Ok((fallback_response, url));
        }
    }

    Err(anyhow!(
        "HTTP error: {}; refreshed URLs were also unavailable",
        initial_status
    ))
}

fn is_unexpected_audio_response(media_type: &str, response: &reqwest::Response) -> bool {
    is_unexpected_audio_content_type(
        media_type,
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    )
}

fn push_unique_url(urls: &mut Vec<String>, url: &str) {
    let url = url.trim();
    if !url.is_empty() && !urls.iter().any(|existing| existing == url) {
        urls.push(url.to_string());
    }
}

fn is_unexpected_audio_content_type(media_type: &str, content_type: Option<&str>) -> bool {
    if !matches!(media_type, "video" | "live_photo") {
        return false;
    }

    content_type
        .map(|content_type| {
            content_type
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .starts_with("audio/")
        })
        .unwrap_or(false)
}

async fn fresh_media_download_urls(
    config: &AppConfig,
    aweme_id: &str,
    media_type: &str,
) -> Result<Vec<String>> {
    let client = DouyinClient::new(config.clone())?;
    let video = client.get_video_detail(aweme_id).await?;
    Ok(match media_type {
        "live_photo" => video.live_photo_urls.unwrap_or_default(),
        "image" => video.image_urls.unwrap_or_default(),
        "video" => ordered_video_urls(
            &video,
            DownloadQuality::from_config(&config.download_quality),
        ),
        _ => fresh_download_urls_for_video(&video, config),
    })
}

fn fresh_download_urls_for_video(video: &VideoInfo, config: &AppConfig) -> Vec<String> {
    ordered_video_urls(
        video,
        DownloadQuality::from_config(&config.download_quality),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_audio_content_for_video_media() {
        assert!(is_unexpected_audio_content_type(
            "video",
            Some("audio/mp4; charset=utf-8")
        ));
    }

    #[test]
    fn allows_audio_content_for_audio_media() {
        assert!(!is_unexpected_audio_content_type(
            "audio",
            Some("audio/mpeg")
        ));
    }

    #[test]
    fn allows_video_content_for_video_media() {
        assert!(!is_unexpected_audio_content_type(
            "video",
            Some("video/mp4")
        ));
    }

    #[test]
    fn allows_unknown_content_type_for_video_media() {
        assert!(!is_unexpected_audio_content_type("video", None));
    }
}
