use crate::config::AppConfig;
use anyhow::Result;
use std::time::Duration;

pub(crate) fn apply_tls_config(
    builder: reqwest::ClientBuilder,
    config: &AppConfig,
) -> reqwest::ClientBuilder {
    builder.danger_accept_invalid_certs(!config.ssl_verify)
}

pub(crate) fn build_media_http_client(config: &AppConfig) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none());

    Ok(apply_tls_config(builder, config).build()?)
}
