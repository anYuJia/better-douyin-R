//! 媒体下载请求辅助

use crate::api::types::{DownloadMediaItem, VideoInfo};
use crate::api::DouyinClient;
use crate::config::AppConfig;
use crate::media_utils::is_dash_video_only_url;
use anyhow::{anyhow, Result};
use reqwest::header::HeaderMap;

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
            log::info!(
                "allowing dash video-only download as final fallback: aweme_id={}",
                aweme_id
            );
            return Ok((response, media.url.clone()));
        }

        // Try other DASH fallback URLs
        for url in fallback_urls {
            let fallback_response = client.get(&url).headers(headers.clone()).send().await?;
            if fallback_response.status().is_success() {
                return Ok((fallback_response, url));
            }
        }

        return Err(anyhow!("没有可用的视频下载地址"));
    }

    let response = client
        .get(&media.url)
        .headers(headers.clone())
        .send()
        .await?;

    if response.status().is_success() {
        return Ok((response, media.url.clone()));
    }

    let initial_status = response.status();
    if aweme_id.trim().is_empty() {
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
            log::info!(
                "download url refreshed after HTTP {}: aweme_id={}",
                initial_status,
                aweme_id
            );
            return Ok((fallback_response, url.clone()));
        }
    }

    // Secondary fallback: Allow any refreshed URLs even if they contain DASH media-video
    for url in fallback_urls {
        if url == media.url {
            continue;
        }

        let fallback_response = client.get(&url).headers(headers.clone()).send().await?;
        if fallback_response.status().is_success() {
            return Ok((fallback_response, url));
        }
    }

    Err(anyhow!(
        "HTTP error: {}; refreshed URLs were also unavailable",
        initial_status
    ))
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
