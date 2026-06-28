//! 抖音视频下载器 - Tauri 应用

pub mod api;
pub mod config;
pub mod cookie;
pub mod downloader;
pub mod history;
pub mod media_proxy;
mod media_proxy_security;
mod media_proxy_headers;
mod media_proxy_cache;
mod media_proxy_crypto;
mod media_proxy_local;
mod media_proxy_remote;
pub mod media_utils;
pub mod reporter;
pub mod sign;
pub mod download_files;
pub mod friend_chat;
pub mod login_window;
pub mod system_open;
pub mod update;
pub mod commands;
pub mod state;
pub mod api_helpers;
pub mod im_listener;
pub mod download_payload;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let log_level = if cfg!(debug_assertions) {
                log::LevelFilter::Info
            } else {
                log::LevelFilter::Warn
            };
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log_level)
                    .build(),
            )?;

            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    if let Err(error) = window.set_decorations(false) {
                        log::warn!("failed to disable Windows window decorations: {}", error);
                    }
                }
            }

            let state = AppState::new();
            *state.app_handle.blocking_lock() = Some(app.handle().clone());
            tauri::async_runtime::spawn({
                let state = state.clone();
                async move {
                    if let Err(error) = media_proxy::spawn_media_proxy(state).await {
                        log::error!("failed to start media proxy: {}", error);
                    }
                }
            });
            app.manage(state);

            #[cfg(debug_assertions)]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::update_cmd::get_app_version,
            commands::update_cmd::restart_app,
            commands::update_cmd::check_update,
            commands::update_cmd::download_update,
            commands::config::init_client,
            commands::config::get_config,
            commands::config::save_config,
            commands::config::logout_cookie,
            commands::config::select_directory,
            commands::content_video::parse_url,
            commands::content_video::parse_link,
            commands::content_relations::set_video_liked,
            commands::content_relations::set_video_collected,
            commands::content_relations::set_user_followed,
            commands::content_video::get_video_detail,
            commands::content_user::search_user,
            commands::content_user::get_user_detail,
            commands::content_user::get_user_videos,
            commands::content_collection::get_liked_videos,
            commands::content_collection::get_collected_videos,
            commands::content_collection::get_collected_mixes,
            commands::content_collection::get_mix_videos,
            commands::content_user::get_liked_authors,
            commands::friends::get_friend_online_status,
            commands::friends::get_share_friends,
            commands::friends::send_friend_message,
            commands::friends::send_friend_video_share,
            commands::friends::send_friend_image_message,
            commands::friends::get_friend_message_history,
            commands::friends::get_friend_chat_state,
            commands::friends::save_friend_chat_state,
            commands::content_feed::get_recommended,
            commands::content_comments::get_comments,
            commands::content_comments::get_comment_replies,
            commands::content_comments::set_comment_liked,
            commands::content_comments::publish_comment,
            commands::config::verify_cookie,
            commands::config::get_current_user,
            commands::login::open_verify_browser,
            commands::login::cookie_browser_login,
            commands::login::cancel_cookie_browser_login,
            commands::downloads::download_video,
            commands::downloads::download_videos,
            commands::downloads::download_user_videos,
            commands::downloads::download_liked_videos,
            commands::downloads::download_liked_authors,
            commands::downloads::add_download_task,
            commands::downloads::start_download,
            commands::downloads::get_download_tasks,
            commands::downloads::cancel_download_task,
            commands::downloads::remove_download_task,
            commands::downloads::pause_download,
            commands::downloads::resume_download,
            commands::download_files_cmd::list_download_files,
            commands::history::get_history,
            commands::history::clear_history,
            commands::history::delete_history,
            commands::history::add_history,
            commands::system::open_file,
            commands::system::open_download_directory,
            commands::system::open_file_location,
            commands::system::open_external_url,
            commands::system::delete_file,
            commands::system::copy_text_to_clipboard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
