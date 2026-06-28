use crate::media_proxy_headers::{apply_cors_headers, build_error_response};
use axum::body::{Body, Bytes};
use axum::http::{header, HeaderValue, Response, StatusCode};

pub(crate) const LOCAL_MEDIA_INITIAL_RANGE_BYTES: u64 = 1024 * 1024;
pub(crate) const LOCAL_MEDIA_MAX_RANGE_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const REMOTE_MEDIA_MAX_RANGE_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const REMOTE_MEDIA_RANGE_CACHE_ENTRIES: usize = 24;

#[derive(Clone)]
pub(crate) struct CachedMediaRange {
    pub(crate) status: StatusCode,
    pub(crate) content_type: Option<String>,
    pub(crate) content_range: Option<String>,
    pub(crate) accept_ranges: Option<String>,
    pub(crate) body: Bytes,
}

pub(crate) fn parse_byte_range(range_header: &str, file_size: u64) -> Option<(u64, u64)> {
    if file_size == 0 {
        return None;
    }

    let value = range_header.trim();
    let bytes = value.strip_prefix("bytes=")?.trim();
    let first = bytes.split(',').next()?.trim();
    let (start_raw, end_raw) = first.split_once('-')?;

    if start_raw.is_empty() {
        let suffix_length = end_raw.trim().parse::<u64>().ok()?;
        if suffix_length == 0 {
            return None;
        }
        let start = file_size.saturating_sub(suffix_length);
        return Some((start, file_size - 1));
    }

    let start = start_raw.trim().parse::<u64>().ok()?;
    if start >= file_size {
        return None;
    }

    let end = if end_raw.trim().is_empty() {
        file_size - 1
    } else {
        end_raw.trim().parse::<u64>().ok()?.min(file_size - 1)
    };

    if end < start {
        None
    } else {
        Some((start, end))
    }
}

pub(crate) fn cap_remote_media_range(
    range_header: &str,
    requested_media_type: &str,
) -> Option<String> {
    if requested_media_type != "video" && requested_media_type != "audio" {
        return None;
    }

    let value = range_header.trim();
    let bytes = value.strip_prefix("bytes=")?.trim();
    let first = bytes.split(',').next()?.trim();
    let (start_raw, end_raw) = first.split_once('-')?;
    if start_raw.trim().is_empty() {
        return None;
    }

    let start = start_raw.trim().parse::<u64>().ok()?;
    let requested_end = end_raw.trim().parse::<u64>().ok();
    let capped_end = start.saturating_add(REMOTE_MEDIA_MAX_RANGE_BYTES - 1);
    let end = requested_end.map_or(capped_end, |value| value.min(capped_end));
    if end < start {
        return None;
    }

    let capped = format!("bytes={}-{}", start, end);
    if capped == value {
        None
    } else {
        Some(capped)
    }
}

pub(crate) fn remote_media_range_cache_key(
    url: &str,
    range: Option<&str>,
    requested_media_type: &str,
) -> Option<String> {
    if requested_media_type != "video" && requested_media_type != "audio" {
        return None;
    }
    let range = range?.trim();
    if range.is_empty() || !range.starts_with("bytes=") {
        return None;
    }
    Some(format!("{requested_media_type}::{range}::{url}"))
}

pub(crate) fn remote_media_range_cache_keys(
    original_url: &str,
    upstream_url: &str,
    range: Option<&str>,
    requested_media_type: &str,
) -> Vec<String> {
    let mut keys = Vec::new();
    for url in [upstream_url, original_url] {
        let Some(key) = remote_media_range_cache_key(url, range, requested_media_type) else {
            continue;
        };
        if !keys.iter().any(|existing| existing == &key) {
            keys.push(key);
        }
    }
    keys
}

pub(crate) fn cached_media_response(
    cached: CachedMediaRange,
    allow_origin: Option<HeaderValue>,
) -> Response<Body> {
    let mut response_builder = Response::builder().status(cached.status);
    let headers = match response_builder.headers_mut() {
        Some(headers) => headers,
        None => return build_error_response(StatusCode::BAD_GATEWAY, "Failed to build response"),
    };

    if let Some(value) = cached
        .content_type
        .as_deref()
        .and_then(|value| HeaderValue::from_str(value).ok())
    {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Some(value) = cached
        .content_range
        .as_deref()
        .and_then(|value| HeaderValue::from_str(value).ok())
    {
        headers.insert(header::CONTENT_RANGE, value);
    }
    if let Some(value) = cached
        .accept_ranges
        .as_deref()
        .and_then(|value| HeaderValue::from_str(value).ok())
    {
        headers.insert(header::ACCEPT_RANGES, value);
    }
    if !headers.contains_key(header::ACCEPT_RANGES) {
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Ok(value) = HeaderValue::from_str(&cached.body.len().to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    apply_cors_headers(headers, allow_origin);

    response_builder
        .body(Body::from(cached.body))
        .unwrap_or_else(|_| build_error_response(StatusCode::BAD_GATEWAY, "Proxy error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_large_remote_video_ranges() {
        assert_eq!(
            cap_remote_media_range("bytes=196608-90483921", "video").as_deref(),
            Some("bytes=196608-4390911")
        );
        assert_eq!(cap_remote_media_range("bytes=0-1", "video"), None);
        assert_eq!(cap_remote_media_range("bytes=0-90483921", "image"), None);
    }
}
