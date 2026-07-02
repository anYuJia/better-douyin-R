//! HTTP 下载辅助

use crate::config::AppConfig;
use crate::http_client::apply_tls_config;
use anyhow::Result;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, COOKIE, RANGE, REFERER, USER_AGENT,
};
use std::time::Duration;

const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const DOWNLOAD_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const DOWNLOAD_TCP_KEEPALIVE: Duration = Duration::from_secs(60);
const DOWNLOAD_POOL_MAX_IDLE_PER_HOST: usize = 16;

pub(crate) fn build_download_client(config: &AppConfig) -> Result<reqwest::Client> {
    let mut builder = apply_tls_config(
        reqwest::Client::builder()
            .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
            .timeout(DOWNLOAD_REQUEST_TIMEOUT)
            .pool_idle_timeout(DOWNLOAD_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(DOWNLOAD_POOL_MAX_IDLE_PER_HOST)
            .tcp_keepalive(DOWNLOAD_TCP_KEEPALIVE),
        config,
    );

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
    headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1"));

    if !config.cookie.trim().is_empty() {
        if let Ok(cookie) = HeaderValue::from_str(&config.cookie) {
            headers.insert(COOKIE, cookie);
        }
    }

    headers
}
