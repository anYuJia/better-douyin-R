use crate::config::get_user_agent;
use crate::AppState;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use futures::StreamExt;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use url::Url;

use crate::media_proxy_headers::{apply_cors_headers, build_error_response};
use crate::media_proxy_security::{
    allowed_request_origin, guess_content_type, is_allowed_media_url, media_url_label,
    resolve_redirect_target, should_send_cookie,
};
pub(crate) use crate::media_proxy_cache::CachedMediaRange;
use crate::media_proxy_cache::{
    cap_remote_media_range, parse_byte_range,
    remote_media_range_cache_keys, cached_media_response,
    LOCAL_MEDIA_INITIAL_RANGE_BYTES, LOCAL_MEDIA_MAX_RANGE_BYTES,
    REMOTE_MEDIA_MAX_RANGE_BYTES, REMOTE_MEDIA_RANGE_CACHE_ENTRIES,
};

pub const MEDIA_PROXY_PORT: u16 = 39143;
const INITIAL_VIDEO_RANGE: &str = "bytes=0-1048575";


#[derive(Debug, Deserialize)]
struct MediaProxyQuery {
    url: String,
    media_type: Option<String>,
    skey: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalMediaQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct SeekDebugQuery {
    phase: Option<String>,
    target: Option<f64>,
    before: Option<f64>,
    after: Option<f64>,
    duration: Option<f64>,
    ready_state: Option<u32>,
    network_state: Option<u32>,
    paused: Option<bool>,
    src: Option<String>,
}



fn hex_to_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(trimmed.len() / 2);
    for index in (0..trimmed.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&trimmed[index..index + 2], 16).ok()?);
    }
    Some(bytes)
}

fn guess_image_content_type_from_bytes(data: &[u8]) -> &'static str {
    if data.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

fn decrypt_im_image_bytes(encrypted: &[u8], skey: &str) -> Option<Vec<u8>> {
    if encrypted.len() <= 28 {
        return None;
    }
    let key = hex_to_bytes(skey)?;
    if key.len() != 32 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    cipher
        .decrypt(Nonce::from_slice(&encrypted[..12]), &encrypted[12..])
        .ok()
}



fn frontend_dist_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist")
}



fn local_media_content_type(path: &Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "avif" => Some("image/avif"),
        "heic" => Some("image/heic"),
        "heif" => Some("image/heif"),
        "mp4" | "m4v" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        "webm" => Some("video/webm"),
        "mkv" => Some("video/x-matroska"),
        "avi" => Some("video/x-msvideo"),
        "flv" => Some("video/x-flv"),
        "mp3" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        "aac" => Some("audio/aac"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "ogg" => Some("audio/ogg"),
        _ => None,
    }
}

fn local_media_kind(path: &Path) -> Option<&'static str> {
    let content_type = local_media_content_type(path)?;
    if content_type.starts_with("image/") {
        Some("image")
    } else if content_type.starts_with("video/") {
        Some("video")
    } else if content_type.starts_with("audio/") {
        Some("audio")
    } else {
        None
    }
}

const PREWARM_HEADER: &str = "x-douyin-prewarm";
const MAX_RETRIES: usize = 3;

async fn allowed_local_media_path(state: &AppState, raw_path: &str) -> Result<PathBuf, StatusCode> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let target = Path::new(trimmed)
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;

    if !target.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }

    if local_media_kind(&target).is_none() {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    let download_path = {
        let config = state.config.lock().await;
        config.download_path.clone()
    };

    if download_path.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let download_root = Path::new(download_path.trim())
        .canonicalize()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    if target.starts_with(download_root) {
        return Ok(target);
    }

    let history_paths = {
        let history = state.history.lock().await;
        history
            .get_all()
            .into_iter()
            .map(|item| item.file_path)
            .collect::<Vec<_>>()
    };

    let is_history_file = history_paths.iter().any(|path| {
        Path::new(path)
            .canonicalize()
            .map(|history_path| history_path == target)
            .unwrap_or(false)
    });

    if is_history_file {
        Ok(target)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn local_media(
    State(state): State<AppState>,
    Query(query): Query<LocalMediaQuery>,
    request_headers: HeaderMap,
) -> Response<Body> {
    let allow_origin = match allowed_request_origin(&request_headers) {
        Some(origin) => origin,
        None => return build_error_response(StatusCode::FORBIDDEN, "Forbidden"),
    };

    let path = match allowed_local_media_path(&state, &query.path).await {
        Ok(path) => path,
        Err(StatusCode::BAD_REQUEST) => {
            return build_error_response(StatusCode::BAD_REQUEST, "Invalid path")
        }
        Err(StatusCode::NOT_FOUND) => {
            return build_error_response(StatusCode::NOT_FOUND, "File not found")
        }
        Err(StatusCode::UNSUPPORTED_MEDIA_TYPE) => {
            return build_error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Unsupported media type",
            )
        }
        Err(status) => return build_error_response(status, "Forbidden"),
    };

    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return build_error_response(StatusCode::NOT_FOUND, "File not found"),
    };
    let file_size = metadata.len();
    let content_type = local_media_content_type(&path).unwrap_or("application/octet-stream");
    let media_kind = local_media_kind(&path).unwrap_or_default();
    let request_range = request_headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_byte_range(value, file_size));

    let should_seed_range = request_range.is_none()
        && file_size > LOCAL_MEDIA_INITIAL_RANGE_BYTES
        && media_kind != "image";

    let (status, start, end) = if let Some((start, end)) = request_range {
        (StatusCode::PARTIAL_CONTENT, start, end)
    } else if should_seed_range {
        (
            StatusCode::PARTIAL_CONTENT,
            0,
            (LOCAL_MEDIA_INITIAL_RANGE_BYTES - 1).min(file_size.saturating_sub(1)),
        )
    } else if file_size == 0 {
        (StatusCode::OK, 0, 0)
    } else {
        (StatusCode::OK, 0, file_size - 1)
    };

    let read_length = if file_size == 0 { 0 } else { end - start + 1 };
    let capped_length = if status == StatusCode::PARTIAL_CONTENT {
        read_length.min(LOCAL_MEDIA_MAX_RANGE_BYTES)
    } else {
        read_length
    };
    let capped_end = if capped_length == 0 {
        start
    } else {
        start + capped_length - 1
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return build_error_response(StatusCode::NOT_FOUND, "File not found"),
    };

    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error");
    }

    let mut buffer = vec![0u8; capped_length as usize];
    if capped_length > 0 && file.read_exact(&mut buffer).await.is_err() {
        return build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error");
    }

    let mut response_builder = Response::builder().status(status);
    let headers = match response_builder.headers_mut() {
        Some(headers) => headers,
        None => {
            return build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error")
        }
    };

    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        allow_origin.unwrap_or_else(|| HeaderValue::from_static("*")),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&capped_length.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );

    if status == StatusCode::PARTIAL_CONTENT {
        let content_range = format!("bytes {}-{}/{}", start, capped_end, file_size);
        if let Ok(value) = HeaderValue::from_str(&content_range) {
            headers.insert(header::CONTENT_RANGE, value);
        }
    }

    response_builder
        .body(Body::from(buffer))
        .unwrap_or_else(|_| {
            build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error")
        })
}

async fn media_proxy(
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
                return build_error_response(StatusCode::BAD_REQUEST, "Invalid URL");
            }
        };

        let mut upstream = state
            .media_http_client
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
                                    StatusCode::BAD_REQUEST,
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
                        headers.insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static(guess_image_content_type_from_bytes(
                                &decrypted,
                            )),
                        );
                        if let Ok(value) = HeaderValue::from_str(&decrypted.len().to_string()) {
                            headers.insert(header::CONTENT_LENGTH, value);
                        }
                        headers.insert(
                            header::CACHE_CONTROL,
                            HeaderValue::from_static("public, max-age=3600"),
                        );
                        apply_cors_headers(headers, allow_origin);
                        return builder.body(Body::from(decrypted)).unwrap_or_else(|_| {
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

async fn media_proxy_options(request_headers: HeaderMap) -> Response<Body> {
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

async fn seek_debug(Query(query): Query<SeekDebugQuery>) -> &'static str {
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

pub async fn spawn_media_proxy(state: AppState) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], MEDIA_PROXY_PORT));
    let listener = TcpListener::bind(addr).await?;
    let dist_dir = frontend_dist_dir();

    log::info!(
        "local web server listening on http://{} (dist={})",
        addr,
        dist_dir.display()
    );

    tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/api/media/proxy",
                get(media_proxy).options(media_proxy_options),
            )
            .route("/api/local-media", get(local_media))
            .route("/api/debug/seek", get(seek_debug))
            .fallback_service(ServeDir::new(dist_dir).append_index_html_on_directories(true))
            .with_state(state);

        if let Err(error) = axum::serve(listener, app).await {
            log::error!("local web server failed: {}", error);
        }
    });

    Ok(())
}


