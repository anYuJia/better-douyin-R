use crate::config::get_user_agent;
use crate::AppState;
use crate::media_proxy_crypto::{decrypt_im_image_bytes, guess_image_content_type_from_bytes};
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
use crate::media_proxy_local::{local_media, frontend_dist_dir};
use crate::media_proxy_remote::{media_proxy, media_proxy_options, seek_debug};

pub const MEDIA_PROXY_PORT: u16 = 39143;
const INITIAL_VIDEO_RANGE: &str = "bytes=0-1048575";
const PREWARM_HEADER: &str = "x-douyin-prewarm";
const MAX_RETRIES: usize = 3;














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


