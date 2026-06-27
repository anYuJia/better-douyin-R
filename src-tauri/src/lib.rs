//! 抖音视频下载器 - Tauri 应用

pub mod api;
pub mod config;
pub mod cookie;
pub mod downloader;
pub mod history;
pub mod media_proxy;
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

// ============================================================================
// Tauri Commands
// ============================================================================
















// ============================================================================
// 应用入口
// ============================================================================

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
            commands::content::parse_url,
            commands::content::parse_link,
            commands::content::set_video_liked,
            commands::content::set_video_collected,
            commands::content::set_user_followed,
            commands::content::get_video_detail,
            commands::content::search_user,
            commands::content::get_user_detail,
            commands::content::get_user_videos,
            commands::content::get_liked_videos,
            commands::content::get_collected_videos,
            commands::content::get_collected_mixes,
            commands::content::get_mix_videos,
            commands::content::get_liked_authors,
            commands::friends::get_friend_online_status,
            commands::friends::get_share_friends,
            commands::friends::send_friend_message,
            commands::friends::send_friend_video_share,
            commands::friends::send_friend_image_message,
            commands::friends::get_friend_message_history,
            commands::friends::get_friend_chat_state,
            commands::friends::save_friend_chat_state,
            commands::content::get_recommended,
            commands::content::get_comments,
            commands::content::get_comment_replies,
            commands::content::set_comment_liked,
            commands::content::publish_comment,
            commands::config::verify_cookie,
            commands::config::get_current_user,
            commands::login::open_verify_browser,
            commands::login::cookie_browser_login,
            commands::login::cancel_cookie_browser_login,
            commands::downloads::download_video,
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

#[cfg(test)]
mod tests {
    use super::api::{BitRateInfo, VideoInfo};
    use super::download_files::{download_file_matches_query, download_file_media_kind, is_hidden_download_path, DownloadFileEntry};
    use super::download_payload::{combined_video_info_for_download, video_info_from_download_payload};
    use super::downloader::{available_video_quality_height, video_quality_candidate_count};
    use super::media_utils::{download_media_type_from_payload, parse_download_media_items};
    use std::path::Path;

    #[test]
    fn parses_flat_download_media_items() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "raw_media_type": "video",
            "media_type": "video",
            "media_urls": [{ "type": "video", "url": "https://example.com/test.mp4" }],
        });

        let parsed = parse_download_media_items(&payload, "video");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "video");
        assert_eq!(parsed[0].url, "https://example.com/test.mp4");
    }

    #[test]
    fn parses_nested_react_video_payload() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "media_type": "video",
            "author": { "nickname": "tester" },
            "video": {
                "cover": "https://example.com/cover.jpg",
                "play_addr": "https://example.com/play.mp4"
            }
        });

        let parsed = parse_download_media_items(&payload, "video");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].r#type, "video");
        assert_eq!(parsed[0].url, "https://example.com/play.mp4");
    }

    #[test]
    fn parses_video_info_from_download_payload_with_string_media_type() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "desc": "test",
            "raw_media_type": "video",
            "media_type": "video",
            "author": { "nickname": "tester" },
            "video": {
                "cover": "https://example.com/cover.jpg",
                "play_addr": "https://example.com/play.mp4",
                "bit_rate": [
                    {
                        "gear_name": "normal_1080_0",
                        "height": 1080,
                        "play_addr_h264": "https://example.com/1080-h264.mp4"
                    }
                ]
            }
        });

        let video_info = video_info_from_download_payload(&payload).expect("video info");

        assert_eq!(video_info.aweme_id, "123");
        assert_eq!(
            video_info
                .video
                .bit_rate
                .as_ref()
                .and_then(|items| items.first())
                .and_then(|item| item.play_addr_h264.as_deref()),
            Some("https://example.com/1080-h264.mp4")
        );
    }

    #[test]
    fn combines_fresh_and_payload_quality_candidates() {
        let mut fresh = VideoInfo::default();
        fresh.aweme_id = "123".to_string();
        fresh.video.play_addr = "https://example.com/fresh-play.mp4".to_string();
        fresh.video.bit_rate = Some(vec![BitRateInfo {
            gear_name: "normal_720_0".to_string(),
            height: 720,
            data_size: 720,
            play_addr_h264: Some("https://example.com/720-h264.mp4".to_string()),
            ..Default::default()
        }]);

        let mut payload = VideoInfo::default();
        payload.aweme_id = "123".to_string();
        payload.video.play_addr = "https://example.com/payload-play.mp4".to_string();
        payload.video.bit_rate = Some(vec![BitRateInfo {
            gear_name: "normal_1080_0".to_string(),
            height: 1080,
            data_size: 1080,
            play_addr_h264: Some("https://example.com/1080-h264.mp4".to_string()),
            ..Default::default()
        }]);

        let combined =
            combined_video_info_for_download(Some(&fresh), Some(&payload), "123").expect("video");

        assert_eq!(available_video_quality_height(&combined), 1080);
        assert_eq!(video_quality_candidate_count(&combined), 2);
    }

    #[test]
    fn parses_image_and_live_photo_payloads() {
        let payload = serde_json::json!({
            "aweme_id": "123",
            "media_type": "mixed",
            "images": ["https://example.com/1.jpg"],
            "live_photos": ["https://example.com/1.mp4"]
        });

        let parsed = parse_download_media_items(&payload, "mixed");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].r#type, "live_photo");
        assert_eq!(parsed[1].r#type, "image");
    }

    #[test]
    fn resolves_download_media_type_from_string_and_numeric_payloads() {
        assert_eq!(
            download_media_type_from_payload(
                &serde_json::json!({ "raw_media_type": "live_photo" })
            ),
            "live_photo"
        );
        assert_eq!(
            download_media_type_from_payload(&serde_json::json!({ "raw_media_type": 1 })),
            "image"
        );
        assert_eq!(
            download_media_type_from_payload(&serde_json::json!({ "media_type": "mixed" })),
            "mixed"
        );
    }

    #[test]
    fn classifies_download_media_files_and_filters_auxiliary_files() {
        assert_eq!(
            download_file_media_kind(Path::new("clip.mp4")),
            Some("video")
        );
        assert_eq!(
            download_file_media_kind(Path::new("image.WEBP")),
            Some("image")
        );
        assert_eq!(
            download_file_media_kind(Path::new("sound.m4a")),
            Some("audio")
        );
        assert_eq!(download_file_media_kind(Path::new(".downloaded")), None);
        assert_eq!(download_file_media_kind(Path::new("metadata.json")), None);

        assert!(is_hidden_download_path(Path::new(".DS_Store")));
        assert!(is_hidden_download_path(Path::new(".downloaded")));
        assert!(!is_hidden_download_path(Path::new("作品.mp4")));
    }

    #[test]
    fn matches_download_files_by_full_index_fields() {
        let item = DownloadFileEntry {
            id: "/downloads/作者/风吹过我的头发.mp4".to_string(),
            filename: "风吹过我的头发".to_string(),
            path: "/downloads/作者/风吹过我的头发.mp4".to_string(),
            author: "草坪穿搭".to_string(),
            desc: String::new(),
            size: 1024,
            timestamp: 10,
            file_type: "mp4".to_string(),
            media_type: "video".to_string(),
        };

        assert!(download_file_matches_query(&item, "头发"));
        assert!(download_file_matches_query(&item, "草坪"));
        assert!(download_file_matches_query(&item, "mp4"));
        assert!(!download_file_matches_query(&item, "不存在"));
    }
}
