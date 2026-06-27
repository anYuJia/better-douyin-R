//! HTTP 下载辅助

use crate::config::{get_user_agent, AppConfig};
use anyhow::Result;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, COOKIE, RANGE, REFERER,
    USER_AGENT,
};
use std::time::Duration;

pub(crate) fn build_download_client(config: &AppConfig) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .danger_accept_invalid_certs(false);

    if let Some(proxy) = &config.proxy {
        let proxy = proxy.trim();
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
    }

    Ok(builder.build()?)
}
pub(crate) fn build_download_headers(config: &AppConfig) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        ACCEPT_ENCODING,
        HeaderValue::from_static("identity;q=1, *;q=0"),
    );
    headers.insert(RANGE, HeaderValue::from_static("bytes=0-"));
    headers.insert(REFERER, HeaderValue::from_static("https://www.douyin.com/"));
    headers.insert(USER_AGENT, HeaderValue::from_static(get_user_agent()));

    if !config.cookie.trim().is_empty() {
        if let Ok(cookie) = HeaderValue::from_str(&config.cookie) {
            headers.insert(COOKIE, cookie);
        }
    }

    headers
}

