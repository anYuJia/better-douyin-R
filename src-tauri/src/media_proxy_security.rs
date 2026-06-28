use axum::http::{header, HeaderMap, HeaderValue};
use url::Url;

pub(crate) fn host_matches(host: &str, allowed_domain: &str) -> bool {
    host == allowed_domain || host.ends_with(&format!(".{}", allowed_domain))
}

pub(crate) fn is_allowed_media_url(url: &Url) -> bool {
    if url.scheme() != "https" && url.scheme() != "http" {
        return false;
    }

    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };

    const ALLOWED_MEDIA_DOMAINS: &[&str] = &[
        "douyin.com",
        "douyinvod.com",
        "douyinpic.com",
        "douyinstatic.com",
        "byteimg.com",
        "ixigua.com",
        "amemv.com",
        "snssdk.com",
        "pstatp.com",
    ];

    ALLOWED_MEDIA_DOMAINS
        .iter()
        .any(|domain| host_matches(&host, domain))
}

pub(crate) fn should_send_cookie(url: &Url) -> bool {
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };

    const COOKIE_DOMAINS: &[&str] = &["douyin.com", "amemv.com", "snssdk.com"];

    COOKIE_DOMAINS
        .iter()
        .any(|domain| host_matches(&host, domain))
}

pub(crate) fn media_url_label(raw_url: &str) -> String {
    Url::parse(raw_url)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_string();
            Some(format!("{}{}", host, url.path()))
        })
        .unwrap_or_else(|| raw_url.chars().take(80).collect::<String>())
}

pub(crate) fn allowed_request_origin(request_headers: &HeaderMap) -> Option<Option<HeaderValue>> {
    let Some(origin) = request_headers.get(header::ORIGIN) else {
        return Some(None);
    };

    let origin_str = origin.to_str().ok()?;
    let parsed = Url::parse(origin_str).ok()?;
    let scheme = parsed.scheme();
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed.port_or_known_default();

    let allowed =
        (scheme == "http" && (host == "127.0.0.1" || host == "localhost") && port.is_some())
            || (scheme == "http" && host == "tauri.localhost")
            || (scheme == "tauri" && host == "localhost");

    if allowed {
        Some(Some(origin.clone()))
    } else {
        None
    }
}

pub(crate) fn resolve_redirect_target(current_url: &Url, location: &str) -> Option<String> {
    if let Ok(url) = Url::parse(location) {
        return Some(url.to_string());
    }
    current_url.join(location).ok().map(|url| url.to_string())
}

pub(crate) fn guess_content_type(
    url: &str,
    upstream_content_type: &str,
    requested_media_type: &str,
) -> Option<&'static str> {
    let normalized = upstream_content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();

    if requested_media_type == "audio" {
        if normalized.starts_with("audio/") {
            return Some("audio/mpeg");
        }
        if url.ends_with(".m4a") {
            return Some("audio/mp4");
        }
        return Some("audio/mpeg");
    }

    if !normalized.is_empty() && normalized != "application/octet-stream" {
        return None;
    }

    if url.contains(".mp4") || url.contains("/play/") || requested_media_type == "video" {
        return Some("video/mp4");
    }
    if url.contains(".jpg") || url.contains(".jpeg") {
        return Some("image/jpeg");
    }
    if url.contains(".png") {
        return Some("image/png");
    }
    if url.contains(".webp") {
        return Some("image/webp");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_media_url_by_host() {
        let allowed =
            Url::parse("https://v3-dy-o-abtest.zjcdn.com.douyinvod.com/video.mp4").unwrap();
        assert!(is_allowed_media_url(&allowed));

        let malicious = Url::parse("https://evil.example/?next=douyin.com/video.mp4").unwrap();
        assert!(!is_allowed_media_url(&malicious));

        let lookalike = Url::parse("https://douyin.com.evil.example/video.mp4").unwrap();
        assert!(!is_allowed_media_url(&lookalike));
    }

    #[test]
    fn only_sends_cookie_to_login_related_hosts() {
        let douyin = Url::parse("https://www.douyin.com/aweme/v1/play/").unwrap();
        assert!(should_send_cookie(&douyin));

        let cdn = Url::parse("https://example.douyinvod.com/video.mp4").unwrap();
        assert!(!should_send_cookie(&cdn));
    }

    #[test]
    fn validates_request_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:39143"),
        );
        assert!(allowed_request_origin(&headers).is_some());

        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(allowed_request_origin(&headers).is_none());
    }
}
