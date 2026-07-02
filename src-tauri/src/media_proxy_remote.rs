use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use futures::StreamExt;
use serde::Deserialize;
use url::Url;

use crate::config::get_user_agent;
use crate::media_proxy_cache::{
    cached_media_response, cached_remote_image_response, cap_remote_media_range,
    remote_image_cache_key, remote_media_range_cache_keys, CachedMediaRange, CachedRemoteImage,
    REMOTE_IMAGE_CACHE_MAX_ENTRY_BYTES, REMOTE_MEDIA_MAX_RANGE_BYTES,
    REMOTE_MEDIA_RANGE_CACHE_ENTRIES,
};
use crate::media_proxy_crypto::{decrypt_im_image_bytes, guess_image_content_type_from_bytes};
use crate::media_proxy_headers::{apply_cors_headers, build_error_response};
use crate::media_proxy_security::{
    allowed_request_origin, guess_content_type, is_allowed_media_url, media_url_label,
    resolve_redirect_target, should_send_cookie,
};
use crate::AppState;

const INITIAL_VIDEO_RANGE: &str = "bytes=0-1048575";
const PREWARM_HEADER: &str = "x-douyin-prewarm";
const MAX_RETRIES: usize = 3;

#[derive(Debug, Deserialize)]
pub(crate) struct MediaProxyQuery {
    pub(crate) url: String,
    pub(crate) media_type: Option<String>,
    pub(crate) skey: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeekDebugQuery {
    pub(crate) phase: Option<String>,
    pub(crate) target: Option<f64>,
    pub(crate) before: Option<f64>,
    pub(crate) after: Option<f64>,
    pub(crate) duration: Option<f64>,
    pub(crate) ready_state: Option<u32>,
    pub(crate) network_state: Option<u32>,
    pub(crate) paused: Option<bool>,
    pub(crate) src: Option<String>,
}

pub(crate) async fn media_proxy(
    State(state): State<AppState>,
    Query(query): Query<MediaProxyQuery>,
    request_headers: HeaderMap,
) -> Response<Body> {
    let query_url_label = media_url_label(&query.url);
    log::debug!(
        "media_proxy request received: url={} media_type={:?} Range={:?}",
        query_url_label,
        query.media_type,
        request_headers.get(header::RANGE)
    );
    let requested_media_type = query
        .media_type
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let request_range = request_headers.get(header::RANGE).cloned();
    let is_prewarm_request = request_headers
        .get(PREWARM_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == "1")
        .unwrap_or(false);
    let request_range_str = request_range
        .as_ref()
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let allow_origin = match allowed_request_origin(&request_headers) {
        Some(origin) => origin,
        None => return build_error_response(StatusCode::FORBIDDEN, "Forbidden"),
    };

    let parsed_url = match Url::parse(&query.url) {
        Ok(url) => url,
        Err(_) => return build_error_response(StatusCode::BAD_REQUEST, "Invalid URL"),
    };

    if query.url.is_empty() || !is_allowed_media_url(&parsed_url) {
        return build_error_response(StatusCode::BAD_REQUEST, "Invalid URL");
    }

    let image_cache_key = if requested_media_type == "image" {
        Some(remote_image_cache_key(&query.url, query.skey.as_deref()))
    } else {
        None
    };
    if let Some(key) = &image_cache_key {
        if let Some(cached) = state.media_image_cache.lock().await.get(key) {
            log::debug!("media proxy image cache hit: url={}", query_url_label);
            return cached_remote_image_response(cached, allow_origin);
        }
    }

    let config = state.config.lock().await.clone();
    let should_seed_video_range = false; // 禁用对标准 GET 请求强制注入 Range 的行为，遵循 RFC 7233 规范，返回标准的 200 OK。
    let upstream_range_value = if let Some(range) = &request_range {
        range.to_str().ok().map(|value| {
            cap_remote_media_range(value, &requested_media_type)
                .unwrap_or_else(|| value.to_string())
        })
    } else if should_seed_video_range {
        Some(INITIAL_VIDEO_RANGE.to_string())
    } else {
        None
    };
    let cache_key = if query.url.contains("/aweme/v1/play/") {
        Some(query.url.clone())
    } else {
        None
    };
    let cached_url = if let Some(key) = &cache_key {
        state.media_redirect_cache.lock().await.get(key).cloned()
    } else {
        None
    };
    let mut upstream_url = cached_url.clone().unwrap_or_else(|| query.url.clone());
    let range_cache_keys = remote_media_range_cache_keys(
        &query.url,
        &upstream_url,
        upstream_range_value.as_deref(),
        &requested_media_type,
    );
    for cache_key in &range_cache_keys {
        if let Some(cached) = state.media_range_cache.lock().await.get(cache_key).cloned() {
            log::debug!(
                "media proxy range cache hit: range=\"{}\" url={}",
                request_range_str,
                media_url_label(&upstream_url)
            );
            return cached_media_response(cached, allow_origin);
        }
    }

    let start = std::time::Instant::now();
    let mut redirect_hops = 0usize;
    let mut retry_count = 0usize;
    let upstream_response = loop {
        let parsed_upstream_url = match Url::parse(&upstream_url) {
            Ok(url) if is_allowed_media_url(&url) => url,
            _ => {
                if let Some(key) = &cache_key {
                    state.media_redirect_cache.lock().await.remove(key);
                }
                return build_error_response(StatusCode::BAD_GATEWAY, "Invalid URL");
            }
        };

        let media_http_client = state.media_http_client.lock().await.clone();
        let mut upstream = media_http_client
            .get(&upstream_url)
            .timeout(if requested_media_type == "image" {
                tokio::time::Duration::from_secs(8)
            } else {
                tokio::time::Duration::from_secs(45)
            })
            .header("User-Agent", get_user_agent())
            .header("Referer", "https://www.douyin.com/")
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity;q=1, *;q=0");

        if !config.cookie.is_empty() && should_send_cookie(&parsed_upstream_url) {
            upstream = upstream.header("Cookie", &config.cookie);
        }

        if let Some(range_value) = &upstream_range_value {
            upstream = upstream.header("Range", range_value);
        }

        match upstream.send().await {
            Ok(response) => {
                let status = response.status();

                // 处理重定向
                if status.is_redirection() {
                    let location = response
                        .headers()
                        .get(header::LOCATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("");

                    if location.is_empty() || redirect_hops >= 4 {
                        break response;
                    }

                    if let Some(next_url) = resolve_redirect_target(response.url(), location) {
                        let next_parsed = match Url::parse(&next_url) {
                            Ok(url) if is_allowed_media_url(&url) => url,
                            _ => {
                                if let Some(key) = &cache_key {
                                    state.media_redirect_cache.lock().await.remove(key);
                                }
                                return build_error_response(
                                    StatusCode::BAD_GATEWAY,
                                    "Invalid redirect URL",
                                );
                            }
                        };
                        redirect_hops += 1;
                        upstream_url = next_parsed.to_string();
                        continue;
                    }
                }

                // 处理服务器错误 (5xx)，尝试重试
                if status.is_server_error() && retry_count < MAX_RETRIES {
                    retry_count += 1;
                    log::warn!(
                        "media proxy upstream server error: status={} retry={}/{} url={}",
                        status,
                        retry_count,
                        MAX_RETRIES,
                        media_url_label(&upstream_url)
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        500 * retry_count as u64,
                    ))
                    .await;
                    continue;
                }

                if let Some(key) = &cache_key {
                    if upstream_url != *key {
                        state
                            .media_redirect_cache
                            .lock()
                            .await
                            .insert(key.clone(), upstream_url.clone());
                    }
                }

                break response;
            }
            Err(error) => {
                // 网络错误，尝试重试
                if retry_count < MAX_RETRIES {
                    retry_count += 1;
                    log::warn!(
                        "media proxy network error, retrying: {:?} retry={}/{} url={}",
                        error,
                        retry_count,
                        MAX_RETRIES,
                        media_url_label(&upstream_url)
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        500 * retry_count as u64,
                    ))
                    .await;
                    continue;
                }

                if let Some(key) = &cache_key {
                    state.media_redirect_cache.lock().await.remove(key);
                }
                log::error!(
                    "media proxy upstream request failed: {:?} elapsed={}ms seeded_range={} range=\"{}\" url={}",
                    error,
                    start.elapsed().as_millis(),
                    should_seed_video_range,
                    request_range_str,
                    media_url_label(&upstream_url)
                );
                return build_error_response(StatusCode::BAD_GATEWAY, "Proxy error");
            }
        }
    };

    let status = upstream_response.status();
    let upstream_content_range = upstream_response
        .headers()
        .get("content-range")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let upstream_content_length = upstream_response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let elapsed_ms = start.elapsed().as_millis();
    let upstream_url_label = media_url_label(&upstream_url);
    if status.is_success() && elapsed_ms < 8_000 {
        log::debug!(
            "media proxy upstream response: status={} seeded_range={} request_range=\"{}\" upstream_range=\"{}\" length=\"{}\" elapsed_ms={} url={}",
            status,
            should_seed_video_range,
            request_range_str,
            upstream_content_range,
            upstream_content_length,
            elapsed_ms,
            upstream_url_label
        );
    } else {
        log::warn!(
            "media proxy upstream response: status={} seeded_range={} request_range=\"{}\" upstream_range=\"{}\" length=\"{}\" elapsed_ms={} url={}",
            status,
            should_seed_video_range,
            request_range_str,
            upstream_content_range,
            upstream_content_length,
            elapsed_ms,
            upstream_url_label
        );
    }

    let mut response_builder = Response::builder().status(status);
    let response_headers = match response_builder.headers_mut() {
        Some(h) => h,
        None => return build_error_response(StatusCode::BAD_GATEWAY, "Failed to build response"),
    };

    let copy_headers: [(axum::http::header::HeaderName, &str); 3] = [
        (header::CONTENT_TYPE, "content-type"),
        (header::CONTENT_RANGE, "content-range"),
        (header::ACCEPT_RANGES, "accept-ranges"),
    ];
    for (header_ref, header_name) in copy_headers {
        if let Some(value) = upstream_response.headers().get(header_name) {
            response_headers.insert(header_ref, value.clone());
        }
    }

    let upstream_content_type = upstream_response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    if let Some(content_length) = upstream_response.headers().get("content-length") {
        response_headers.insert(header::CONTENT_LENGTH, content_length.clone());
    }

    if let Some(content_type) =
        guess_content_type(&query.url, &upstream_content_type, &requested_media_type)
    {
        response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    }

    if (requested_media_type == "audio" || requested_media_type == "video")
        && !response_headers.contains_key(header::ACCEPT_RANGES)
    {
        response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }

    apply_cors_headers(response_headers, allow_origin.clone());
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );

    log::debug!(
        "media_proxy response: status={} content_type={:?} content_length={:?} content_range={:?}",
        status,
        response_headers.get(header::CONTENT_TYPE),
        response_headers.get(header::CONTENT_LENGTH),
        response_headers.get(header::CONTENT_RANGE)
    );

    let should_cache_range = is_prewarm_request && !range_cache_keys.is_empty();
    if requested_media_type == "image" {
        if let Some(skey) = query
            .skey
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            match upstream_response.bytes().await {
                Ok(encrypted) => {
                    if let Some(decrypted) = decrypt_im_image_bytes(&encrypted, skey) {
                        let content_type = guess_image_content_type_from_bytes(&decrypted);
                        let body = Bytes::from(decrypted);
                        if status == StatusCode::OK
                            && body.len() <= REMOTE_IMAGE_CACHE_MAX_ENTRY_BYTES
                        {
                            if let Some(key) = &image_cache_key {
                                state.media_image_cache.lock().await.insert(
                                    key.clone(),
                                    CachedRemoteImage {
                                        status,
                                        content_type: Some(content_type.to_string()),
                                        cache_control: Some("public, max-age=3600".to_string()),
                                        body: body.clone(),
                                    },
                                );
                            }
                        }
                        let mut builder = Response::builder().status(status);
                        let headers = match builder.headers_mut() {
                            Some(headers) => headers,
                            None => {
                                return build_error_response(
                                    StatusCode::BAD_GATEWAY,
                                    "Failed to build response",
                                )
                            }
                        };
                        headers
                            .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
                        if let Ok(value) = HeaderValue::from_str(&body.len().to_string()) {
                            headers.insert(header::CONTENT_LENGTH, value);
                        }
                        headers.insert(
                            header::CACHE_CONTROL,
                            HeaderValue::from_static("public, max-age=3600"),
                        );
                        apply_cors_headers(headers, allow_origin);
                        return builder.body(Body::from(body)).unwrap_or_else(|_| {
                            build_error_response(StatusCode::BAD_GATEWAY, "Proxy error")
                        });
                    }
                    log::warn!(
                        "media proxy failed to decrypt IM image, returning raw response: url={}",
                        media_url_label(&upstream_url)
                    );
                    return response_builder
                        .body(Body::from(encrypted))
                        .unwrap_or_else(|_| {
                            build_error_response(StatusCode::BAD_GATEWAY, "Proxy error")
                        });
                }
                Err(error) => {
                    log::warn!("media proxy failed to read encrypted image body: {}", error);
                    return build_error_response(StatusCode::BAD_GATEWAY, "Proxy error");
                }
            }
        }
    }

    let declared_length = upstream_content_length.parse::<usize>().ok();
    let should_cache_image = requested_media_type == "image"
        && status == StatusCode::OK
        && image_cache_key.is_some()
        && declared_length
            .map(|length| length <= REMOTE_IMAGE_CACHE_MAX_ENTRY_BYTES)
            .unwrap_or(false);
    if should_cache_image {
        match upstream_response.bytes().await {
            Ok(bytes) => {
                if bytes.len() <= REMOTE_IMAGE_CACHE_MAX_ENTRY_BYTES {
                    let cached = CachedRemoteImage {
                        status,
                        content_type: response_headers
                            .get(header::CONTENT_TYPE)
                            .and_then(|value| value.to_str().ok())
                            .map(ToString::to_string),
                        cache_control: response_headers
                            .get(header::CACHE_CONTROL)
                            .and_then(|value| value.to_str().ok())
                            .map(ToString::to_string),
                        body: bytes.clone(),
                    };
                    if let Some(key) = &image_cache_key {
                        state
                            .media_image_cache
                            .lock()
                            .await
                            .insert(key.clone(), cached);
                    }
                }

                return response_builder
                    .body(Body::from(bytes))
                    .unwrap_or_else(|_| {
                        build_error_response(StatusCode::BAD_GATEWAY, "Proxy error")
                    });
            }
            Err(error) => {
                log::warn!("media proxy failed to read cacheable image body: {}", error);
                return build_error_response(StatusCode::BAD_GATEWAY, "Proxy error");
            }
        }
    }

    if should_cache_range {
        let declared_length = upstream_content_length
            .parse::<usize>()
            .unwrap_or(usize::MAX);
        if status == StatusCode::PARTIAL_CONTENT
            && declared_length <= REMOTE_MEDIA_MAX_RANGE_BYTES as usize
        {
            match upstream_response.bytes().await {
                Ok(bytes) => {
                    if bytes.len() <= REMOTE_MEDIA_MAX_RANGE_BYTES as usize {
                        let cached = CachedMediaRange {
                            status,
                            content_type: response_headers
                                .get(header::CONTENT_TYPE)
                                .and_then(|value| value.to_str().ok())
                                .map(ToString::to_string),
                            content_range: response_headers
                                .get(header::CONTENT_RANGE)
                                .and_then(|value| value.to_str().ok())
                                .map(ToString::to_string),
                            accept_ranges: response_headers
                                .get(header::ACCEPT_RANGES)
                                .and_then(|value| value.to_str().ok())
                                .map(ToString::to_string),
                            body: bytes.clone(),
                        };
                        let final_cache_keys = remote_media_range_cache_keys(
                            &query.url,
                            &upstream_url,
                            upstream_range_value.as_deref(),
                            &requested_media_type,
                        );
                        let mut cache = state.media_range_cache.lock().await;
                        for cache_key in final_cache_keys {
                            if cache.len() >= REMOTE_MEDIA_RANGE_CACHE_ENTRIES {
                                if let Some(oldest_key) = cache.keys().next().cloned() {
                                    cache.remove(&oldest_key);
                                }
                            }
                            cache.insert(cache_key, cached.clone());
                        }
                    }

                    return response_builder
                        .body(Body::from(bytes))
                        .unwrap_or_else(|_| {
                            build_error_response(StatusCode::BAD_GATEWAY, "Proxy error")
                        });
                }
                Err(error) => {
                    log::warn!("media proxy failed to read cacheable range body: {}", error);
                    return build_error_response(StatusCode::BAD_GATEWAY, "Proxy error");
                }
            }
        }
    }

    let stream = upstream_response
        .bytes_stream()
        .map(|result| result.map_err(std::io::Error::other));

    response_builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| build_error_response(StatusCode::BAD_GATEWAY, "Proxy error"))
}

pub(crate) async fn media_proxy_options(request_headers: HeaderMap) -> Response<Body> {
    let allow_origin = match allowed_request_origin(&request_headers) {
        Some(origin) => origin,
        None => return build_error_response(StatusCode::FORBIDDEN, "Forbidden"),
    };

    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()));
    apply_cors_headers(response.headers_mut(), allow_origin);
    response
}

pub(crate) async fn seek_debug(Query(query): Query<SeekDebugQuery>) -> &'static str {
    log::debug!(
        "player seek debug: phase={} target={:?} before={:?} after={:?} duration={:?} ready_state={:?} network_state={:?} paused={:?} src={}",
        query.phase.unwrap_or_default(),
        query.target,
        query.before,
        query.after,
        query.duration,
        query.ready_state,
        query.network_state,
        query.paused,
        query.src.unwrap_or_default().chars().take(160).collect::<String>()
    );
    "ok"
}
