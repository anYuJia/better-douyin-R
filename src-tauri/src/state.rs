//! 应用状态

use crate::api::DouyinClient;
use crate::config::AppConfig;
use crate::cookie::CookieLoginSession;
use crate::download_files::DownloadFileIndexCache;
use crate::downloader::Downloader;
use crate::history::HistoryManager;
use crate::http_client::build_media_http_client;
use crate::media_proxy::{RemoteImageCache, RemoteRangeCache};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub(crate) config: Arc<Mutex<AppConfig>>,
    pub(crate) client: Arc<Mutex<Option<DouyinClient>>>,
    pub(crate) downloader: Arc<Mutex<Option<Downloader>>>,
    pub(crate) history: Arc<Mutex<HistoryManager>>,
    pub(crate) app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    pub(crate) cookie_login: Arc<Mutex<Option<CookieLoginSession>>>,
    pub(crate) media_http_client: Arc<Mutex<reqwest::Client>>,
    pub(crate) media_redirect_cache: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) media_range_cache: Arc<Mutex<RemoteRangeCache>>,
    pub(crate) media_image_cache: Arc<Mutex<RemoteImageCache>>,
    pub(crate) download_file_index: Arc<Mutex<Option<DownloadFileIndexCache>>>,
    pub(crate) im_message_listener: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub(crate) im_message_listener_attempted_at: Arc<Mutex<Option<Instant>>>,
    pub(crate) im_connection_status: Arc<Mutex<Option<serde_json::Value>>>,
}

impl AppState {
    pub fn new() -> Self {
        let config = AppConfig::load();
        let history = HistoryManager::load();
        let media_http_client =
            build_media_http_client(&config).expect("failed to build media HTTP client");
        Self {
            config: Arc::new(Mutex::new(config)),
            client: Arc::new(Mutex::new(None)),
            downloader: Arc::new(Mutex::new(None)),
            history: Arc::new(Mutex::new(history)),
            app_handle: Arc::new(Mutex::new(None)),
            cookie_login: Arc::new(Mutex::new(None)),
            media_http_client: Arc::new(Mutex::new(media_http_client)),
            media_redirect_cache: Arc::new(Mutex::new(HashMap::new())),
            media_range_cache: Arc::new(Mutex::new(RemoteRangeCache::default())),
            media_image_cache: Arc::new(Mutex::new(RemoteImageCache::default())),
            download_file_index: Arc::new(Mutex::new(None)),
            im_message_listener: Arc::new(Mutex::new(None)),
            im_message_listener_attempted_at: Arc::new(Mutex::new(None)),
            im_connection_status: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
