use crate::AppState;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

pub(crate) use crate::media_proxy_cache::CachedMediaRange;
use crate::media_proxy_local::{frontend_dist_dir, local_media};
use crate::media_proxy_remote::{media_proxy, media_proxy_options, seek_debug};

pub const MEDIA_PROXY_PORT: u16 = 39143;

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
