use crate::config::AppConfig;
use anyhow::Result;
use std::time::Duration;

const MEDIA_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const MEDIA_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MEDIA_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const MEDIA_TCP_KEEPALIVE: Duration = Duration::from_secs(60);
const MEDIA_POOL_MAX_IDLE_PER_HOST: usize = 16;

const API_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const API_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const API_TCP_KEEPALIVE: Duration = Duration::from_secs(60);
const API_POOL_MAX_IDLE_PER_HOST: usize = 8;

pub(crate) fn apply_tls_config(
    builder: reqwest::ClientBuilder,
    config: &AppConfig,
) -> reqwest::ClientBuilder {
    builder.danger_accept_invalid_certs(!config.ssl_verify)
}

pub(crate) fn build_media_http_client(config: &AppConfig) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .connect_timeout(MEDIA_CONNECT_TIMEOUT)
        .timeout(MEDIA_REQUEST_TIMEOUT)
        .pool_idle_timeout(MEDIA_POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(MEDIA_POOL_MAX_IDLE_PER_HOST)
        .tcp_keepalive(MEDIA_TCP_KEEPALIVE)
        .redirect(reqwest::redirect::Policy::none());

    Ok(apply_tls_config(builder, config).build()?)
}

pub(crate) fn apply_douyin_api_client_config(
    builder: reqwest::ClientBuilder,
) -> reqwest::ClientBuilder {
    builder
        .connect_timeout(API_CONNECT_TIMEOUT)
        .timeout(API_REQUEST_TIMEOUT)
        .pool_idle_timeout(API_POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(API_POOL_MAX_IDLE_PER_HOST)
        .tcp_keepalive(API_TCP_KEEPALIVE)
}
