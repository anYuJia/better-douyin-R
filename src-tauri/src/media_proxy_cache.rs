use crate::media_proxy_headers::{apply_cors_headers, build_error_response};
use axum::body::{Body, Bytes};
use axum::http::{header, HeaderValue, Response, StatusCode};
use std::collections::{HashMap, VecDeque};

pub(crate) const LOCAL_MEDIA_INITIAL_RANGE_BYTES: u64 = 1024 * 1024;
pub(crate) const LOCAL_MEDIA_MAX_RANGE_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const REMOTE_MEDIA_MAX_RANGE_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const REMOTE_MEDIA_RANGE_CACHE_MAX_BYTES: usize = 96 * 1024 * 1024;
pub(crate) const REMOTE_IMAGE_CACHE_MAX_ENTRY_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const REMOTE_IMAGE_CACHE_MAX_BYTES: usize = 96 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct CachedMediaRange {
    pub(crate) status: StatusCode,
    pub(crate) content_type: Option<String>,
    pub(crate) content_range: Option<String>,
    pub(crate) accept_ranges: Option<String>,
    pub(crate) body: Bytes,
}

pub(crate) struct RemoteRangeCache {
    max_bytes: usize,
    max_entry_bytes: usize,
    current_bytes: usize,
    entries: HashMap<String, CachedMediaRange>,
    lru: VecDeque<String>,
}

impl RemoteRangeCache {
    pub(crate) fn new(max_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            max_bytes,
            max_entry_bytes,
            current_bytes: 0,
            entries: HashMap::new(),
            lru: VecDeque::new(),
        }
    }

    pub(crate) fn get(&mut self, key: &str) -> Option<CachedMediaRange> {
        let cached = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(cached)
    }

    pub(crate) fn insert(&mut self, key: String, cached: CachedMediaRange) -> bool {
        if cached.status != StatusCode::PARTIAL_CONTENT
            || cached.body.is_empty()
            || cached.body.len() > self.max_entry_bytes
            || cached.body.len() > self.max_bytes
        {
            return false;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(previous.body.len());
            self.remove_from_lru(&key);
        }

        self.current_bytes += cached.body.len();
        self.entries.insert(key.clone(), cached);
        self.lru.push_back(key);
        self.evict_to_budget();
        true
    }

    #[cfg(test)]
    fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    #[cfg(test)]
    fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    fn touch(&mut self, key: &str) {
        self.remove_from_lru(key);
        self.lru.push_back(key.to_string());
    }

    fn remove_from_lru(&mut self, key: &str) {
        self.lru.retain(|candidate| candidate != key);
    }

    fn evict_to_budget(&mut self) {
        while self.current_bytes > self.max_bytes {
            let Some(oldest_key) = self.lru.pop_front() else {
                break;
            };
            if let Some(oldest) = self.entries.remove(&oldest_key) {
                self.current_bytes = self.current_bytes.saturating_sub(oldest.body.len());
            }
        }
    }
}

impl Default for RemoteRangeCache {
    fn default() -> Self {
        Self::new(
            REMOTE_MEDIA_RANGE_CACHE_MAX_BYTES,
            REMOTE_MEDIA_MAX_RANGE_BYTES as usize,
        )
    }
}

#[derive(Clone)]
pub(crate) struct CachedRemoteImage {
    pub(crate) status: StatusCode,
    pub(crate) content_type: Option<String>,
    pub(crate) cache_control: Option<String>,
    pub(crate) body: Bytes,
}

pub(crate) struct RemoteImageCache {
    max_bytes: usize,
    current_bytes: usize,
    entries: HashMap<String, CachedRemoteImage>,
    lru: VecDeque<String>,
}

impl RemoteImageCache {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            current_bytes: 0,
            entries: HashMap::new(),
            lru: VecDeque::new(),
        }
    }

    pub(crate) fn get(&mut self, key: &str) -> Option<CachedRemoteImage> {
        let cached = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(cached)
    }

    pub(crate) fn insert(&mut self, key: String, cached: CachedRemoteImage) -> bool {
        if cached.status != StatusCode::OK
            || cached.body.is_empty()
            || cached.body.len() > REMOTE_IMAGE_CACHE_MAX_ENTRY_BYTES
            || cached.body.len() > self.max_bytes
        {
            return false;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(previous.body.len());
            self.remove_from_lru(&key);
        }

        self.current_bytes += cached.body.len();
        self.entries.insert(key.clone(), cached);
        self.lru.push_back(key);
        self.evict_to_budget();
        true
    }

    #[cfg(test)]
    fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    fn touch(&mut self, key: &str) {
        self.remove_from_lru(key);
        self.lru.push_back(key.to_string());
    }

    fn remove_from_lru(&mut self, key: &str) {
        self.lru.retain(|candidate| candidate != key);
    }

    fn evict_to_budget(&mut self) {
        while self.current_bytes > self.max_bytes {
            let Some(oldest_key) = self.lru.pop_front() else {
                break;
            };
            if let Some(oldest) = self.entries.remove(&oldest_key) {
                self.current_bytes = self.current_bytes.saturating_sub(oldest.body.len());
            }
        }
    }
}

impl Default for RemoteImageCache {
    fn default() -> Self {
        Self::new(REMOTE_IMAGE_CACHE_MAX_BYTES)
    }
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

pub(crate) fn remote_image_cache_key(url: &str, skey: Option<&str>) -> String {
    match skey.map(str::trim).filter(|value| !value.is_empty()) {
        Some(skey) => format!("image::{url}::skey={skey}"),
        None => format!("image::{url}"),
    }
}

pub(crate) fn cached_remote_image_response(
    cached: CachedRemoteImage,
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
    if let Ok(value) = HeaderValue::from_str(&cached.body.len().to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Some(value) = cached
        .cache_control
        .as_deref()
        .and_then(|value| HeaderValue::from_str(value).ok())
    {
        headers.insert(header::CACHE_CONTROL, value);
    } else {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600"),
        );
    }
    apply_cors_headers(headers, allow_origin);

    response_builder
        .body(Body::from(cached.body))
        .unwrap_or_else(|_| build_error_response(StatusCode::BAD_GATEWAY, "Proxy error"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn caps_large_remote_video_ranges() {
        assert_eq!(
            cap_remote_media_range("bytes=196608-90483921", "video").as_deref(),
            Some("bytes=196608-4390911")
        );
        assert_eq!(cap_remote_media_range("bytes=0-1", "video"), None);
        assert_eq!(cap_remote_media_range("bytes=0-90483921", "image"), None);
    }

    fn range_entry(body: &'static [u8]) -> CachedMediaRange {
        CachedMediaRange {
            status: StatusCode::PARTIAL_CONTENT,
            content_type: Some("video/mp4".to_string()),
            content_range: Some("bytes 0-1/10".to_string()),
            accept_ranges: Some("bytes".to_string()),
            body: Bytes::from_static(body),
        }
    }

    #[test]
    fn remote_range_cache_refreshes_lru_on_hit() {
        let mut cache = RemoteRangeCache::new(6, 4);
        assert!(cache.insert("a".to_string(), range_entry(b"aa")));
        assert!(cache.insert("b".to_string(), range_entry(b"bb")));
        assert!(cache.get("a").is_some());
        assert!(cache.insert("c".to_string(), range_entry(b"ccc")));

        assert!(cache.contains_key("a"));
        assert!(!cache.contains_key("b"));
        assert!(cache.contains_key("c"));
        assert!(cache.current_bytes() <= 6);
    }

    #[test]
    fn remote_range_cache_evicts_to_byte_budget() {
        let mut cache = RemoteRangeCache::new(5, 4);
        assert!(cache.insert("a".to_string(), range_entry(b"aaa")));
        assert!(cache.insert("b".to_string(), range_entry(b"bbb")));

        assert!(!cache.contains_key("a"));
        assert!(cache.contains_key("b"));
        assert_eq!(cache.current_bytes(), 3);
    }

    #[test]
    fn remote_range_cache_rejects_oversized_entries() {
        let mut cache = RemoteRangeCache::new(10, 4);
        assert!(!cache.insert("too-large".to_string(), range_entry(b"12345")));
        assert!(!cache.contains_key("too-large"));
        assert_eq!(cache.current_bytes(), 0);
    }

    #[test]
    fn remote_image_cache_evicts_lru_by_byte_budget() {
        let mut cache = RemoteImageCache::new(5);
        assert!(cache.insert(
            "a".to_string(),
            CachedRemoteImage {
                status: StatusCode::OK,
                content_type: Some("image/jpeg".to_string()),
                cache_control: None,
                body: Bytes::from_static(b"aaa"),
            },
        ));
        assert!(cache.insert(
            "b".to_string(),
            CachedRemoteImage {
                status: StatusCode::OK,
                content_type: Some("image/png".to_string()),
                cache_control: None,
                body: Bytes::from_static(b"bbb"),
            },
        ));
        assert!(!cache.contains_key("a"));
        assert!(cache.contains_key("b"));
    }

    #[test]
    fn remote_image_cache_skips_oversized_entries() {
        let mut cache = RemoteImageCache::new(4);
        assert!(!cache.insert(
            "too-large".to_string(),
            CachedRemoteImage {
                status: StatusCode::OK,
                content_type: Some("image/jpeg".to_string()),
                cache_control: None,
                body: Bytes::from_static(b"12345"),
            },
        ));
        assert!(!cache.contains_key("too-large"));
    }

    #[tokio::test]
    async fn cached_remote_image_response_preserves_headers() {
        let response = cached_remote_image_response(
            CachedRemoteImage {
                status: StatusCode::OK,
                content_type: Some("image/webp".to_string()),
                cache_control: Some("public, max-age=3600".to_string()),
                body: Bytes::from_static(b"image"),
            },
            Some(HeaderValue::from_static("http://localhost:1087")),
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/webp"
        );
        assert_eq!(response.headers().get(header::CONTENT_LENGTH).unwrap(), "5");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=3600"
        );
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "http://localhost:1087"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"image");
    }
}
