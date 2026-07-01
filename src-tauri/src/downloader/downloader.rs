//! 下载器实现

use crate::api::types::{DownloadStatus, DownloadTask, VideoInfo};
use crate::config::AppConfig;
use crate::history::HistoryManager;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, Mutex};

use super::batch::start_batch_download_impl;
use super::events::emit_event;
use super::http::build_download_client;
use super::media_group::download_media_group;

#[derive(Debug, Clone)]
pub struct DownloaderEvent {
    pub name: &'static str,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct DownloadRuntime {
    pub(crate) client: reqwest::Client,
    pub(crate) config: AppConfig,
    pub(crate) tasks: Arc<Mutex<Vec<DownloadTask>>>,
    pub(crate) progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    pub(crate) cancel_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) pause_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) history: Arc<Mutex<HistoryManager>>,
    pub(crate) downloaded_cache: Arc<RwLock<HashSet<String>>>,
    pub(crate) record_write_lock: Arc<Mutex<()>>,
}

/// 下载器
#[derive(Clone)]
pub struct Downloader {
    pub(crate) client: reqwest::Client,
    pub(crate) config: AppConfig,
    pub(crate) tasks: Arc<Mutex<Vec<DownloadTask>>>,
    pub(crate) progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    pub(crate) cancel_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) pause_tokens: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) history: Arc<Mutex<HistoryManager>>,
    pub(crate) downloaded_cache: Arc<RwLock<HashSet<String>>>,
    pub(crate) downloaded_cache_loaded: Arc<AtomicBool>,
    pub(crate) record_write_lock: Arc<Mutex<()>>,
}

impl Downloader {
    pub fn new(
        config: AppConfig,
        progress_tx: Option<mpsc::Sender<DownloaderEvent>>,
    ) -> Result<Self> {
        let client = build_download_client(&config)?;

        Ok(Self {
            client,
            config,
            tasks: Arc::new(Mutex::new(Vec::new())),
            progress_tx,
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
            pause_tokens: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(HistoryManager::load())),
            downloaded_cache: Arc::new(RwLock::new(HashSet::new())),
            downloaded_cache_loaded: Arc::new(AtomicBool::new(false)),
            record_write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn update_config(&mut self, config: AppConfig) -> Result<()> {
        let download_path_changed = self.config.download_path != config.download_path;
        if self.config.proxy != config.proxy || self.config.ssl_verify != config.ssl_verify {
            self.client = build_download_client(&config)?;
        }
        self.config = config;
        if download_path_changed {
            if let Ok(mut cache) = self.downloaded_cache.write() {
                cache.clear();
            }
            self.downloaded_cache_loaded.store(false, Ordering::Release);
        }
        Ok(())
    }

    /// 开始下载
    pub async fn start_download(&self, task_id: &str) -> Result<()> {
        let task_id_owned = task_id.to_string();
        let runtime = DownloadRuntime {
            client: self.client.clone(),
            config: self.config.clone(),
            tasks: self.tasks.clone(),
            progress_tx: self.progress_tx.clone(),
            cancel_tokens: self.cancel_tokens.clone(),
            pause_tokens: self.pause_tokens.clone(),
            history: self.history.clone(),
            downloaded_cache: self.downloaded_cache.clone(),
            record_write_lock: self.record_write_lock.clone(),
        };

        runtime
            .cancel_tokens
            .lock()
            .await
            .insert(task_id_owned.clone(), false);
        runtime
            .pause_tokens
            .lock()
            .await
            .insert(task_id_owned.clone(), false);

        {
            let mut tasks_lock = runtime.tasks.lock().await;
            if let Some(task) = tasks_lock.iter_mut().find(|t| t.id == task_id) {
                task.status = DownloadStatus::Downloading;
            }
        }

        tokio::spawn(async move {
            if let Err(error) = download_media_group(runtime.clone(), task_id_owned.clone()).await {
                let is_cancelled = error.to_string().to_lowercase().contains("cancelled")
                    || error.to_string().to_lowercase().contains("canceled")
                    || *runtime
                        .cancel_tokens
                        .lock()
                        .await
                        .get(&task_id_owned)
                        .unwrap_or(&false);
                if !is_cancelled {
                    log::error!("Download error: {}", error);
                }

                {
                    let mut tasks_lock = runtime.tasks.lock().await;
                    if let Some(task) = tasks_lock.iter_mut().find(|t| t.id == task_id_owned) {
                        task.status = if is_cancelled {
                            DownloadStatus::Cancelled
                        } else {
                            DownloadStatus::Failed
                        };
                        task.error_msg = Some(error.to_string());
                    }
                }

                if is_cancelled {
                    emit_event(
                        &runtime.progress_tx,
                        "download-cancelled",
                        serde_json::json!({
                            "task_id": task_id_owned,
                            "message": "下载已取消"
                        }),
                    )
                    .await;
                } else {
                    emit_event(
                        &runtime.progress_tx,
                        "download-failed",
                        serde_json::json!({
                            "task_id": task_id_owned,
                            "error": format!("下载失败: {}", error)
                        }),
                    )
                    .await;

                    emit_event(
                        &runtime.progress_tx,
                        "download-error",
                        serde_json::json!({
                            "task_id": task_id_owned,
                            "message": format!("下载失败: {}", error)
                        }),
                    )
                    .await;
                }
            }
        });

        Ok(())
    }

    /// 批量并发下载视频列表
    pub async fn start_batch_download(
        &self,
        videos: Vec<VideoInfo>,
        batch_task_id: String,
        nickname: String,
    ) -> Result<()> {
        start_batch_download_impl(self, videos, batch_task_id, nickname).await
    }
}

/// 从作者目录的隐藏文件 `.downloaded` 加载已下载的 aweme_id 集合

/// 选择视频下载URL

#[cfg(test)]
mod tests {
    use super::super::downloaded_cache::{
        extract_downloaded_aweme_id, is_complete_download_file, parse_downloaded_set,
    };
    use super::super::filename::{
        build_output_dir, create_unique_output_file, generate_filename_with_config,
        media_extension, MAX_FILENAME_BYTES,
    };
    use super::super::quality::{select_video_url, DownloadQuality};
    use super::*;
    use crate::api::types::BitRateInfo;
    use chrono::{Local, TimeZone};
    use std::path::PathBuf;

    fn video_with_quality_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "play-default".to_string();
        video.video.download_addr = Some("download-default".to_string());
        video.video.play_addr_h264 = Some("top-h264".to_string());
        video.video.play_addr_lowbr = Some("lowbr".to_string());
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                play_addr: Some("bitrate-small".to_string()),
                play_addr_h264: Some("bitrate-small-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                play_addr: Some("bitrate-high".to_string()),
                play_addr_h264: Some("bitrate-high-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_resolution_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "play-default".to_string();
        video.video.height = 1080;
        video.video.play_addr_h264 = Some("top-h264".to_string());
        video.video.play_addr_lowbr = Some("lowbr".to_string());
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                height: 480,
                play_addr: Some("p480".to_string()),
                play_addr_h264: Some("p480-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                height: 720,
                play_addr: Some("p720".to_string()),
                play_addr_h264: Some("p720-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                gear_name: "normal_1080_0".to_string(),
                height: 1920,
                play_addr: Some("p1080".to_string()),
                play_addr_h264: Some("p1080-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 800,
                bit_rate: 800,
                gear_name: "normal_1080_1".to_string(),
                height: 1920,
                is_h265: true,
                play_addr: Some("p1080-h265".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 700,
                bit_rate: 700,
                gear_name: "adapt_2k_1440p".to_string(),
                play_addr: Some("p1440".to_string()),
                play_addr_h264: Some("p1440-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 900,
                bit_rate: 900,
                gear_name: "adapt_4k".to_string(),
                play_addr: Some("p2160".to_string()),
                play_addr_h264: Some("p2160-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_sparse_resolution_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "play-default".to_string();
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                height: 480,
                play_addr: Some("sparse-480".to_string()),
                play_addr_h264: Some("sparse-480-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                gear_name: "normal_1080_0".to_string(),
                height: 1920,
                play_addr: Some("sparse-1080".to_string()),
                play_addr_h264: Some("sparse-1080-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_top_level_low_resolution_and_quality_candidates(include_1080: bool) -> VideoInfo {
        let mut bit_rates = vec![
            BitRateInfo {
                data_size: 100,
                bit_rate: 100,
                gear_name: "normal_540_0".to_string(),
                height: 580,
                play_addr: Some("toplow-540".to_string()),
                play_addr_h264: Some("toplow-540-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                gear_name: "normal_720_0".to_string(),
                height: 580,
                play_addr: Some("toplow-720".to_string()),
                play_addr_h264: Some("toplow-720-h264".to_string()),
                ..Default::default()
            },
        ];
        if include_1080 {
            bit_rates.push(BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                gear_name: "normal_1080_0".to_string(),
                height: 580,
                play_addr: Some("toplow-1080".to_string()),
                play_addr_h264: Some("toplow-1080-h264".to_string()),
                ..Default::default()
            });
        }

        let mut video = VideoInfo::default();
        video.video.play_addr = "toplow-default".to_string();
        video.video.height = 580;
        video.video.play_addr_h264 = Some("toplow-top-h264".to_string());
        video.video.bit_rate = Some(bit_rates);
        video
    }

    fn video_with_portrait_dimension_quality_candidates() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "portrait-default".to_string();
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                width: 720,
                height: 1280,
                play_addr: Some("portrait-720".to_string()),
                play_addr_h264: Some("portrait-720-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 700,
                bit_rate: 700,
                width: 1440,
                height: 2560,
                play_addr: Some("portrait-2k".to_string()),
                play_addr_h264: Some("portrait-2k-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    fn video_with_narrow_portrait_1080_candidate() -> VideoInfo {
        let mut video = VideoInfo::default();
        video.video.play_addr = "narrow-default".to_string();
        video.video.bit_rate = Some(vec![
            BitRateInfo {
                data_size: 300,
                bit_rate: 300,
                width: 404,
                height: 720,
                play_addr: Some("narrow-720".to_string()),
                play_addr_h264: Some("narrow-720-h264".to_string()),
                ..Default::default()
            },
            BitRateInfo {
                data_size: 500,
                bit_rate: 500,
                width: 608,
                height: 1080,
                play_addr: Some("narrow-1080".to_string()),
                play_addr_h264: Some("narrow-1080-h264".to_string()),
                ..Default::default()
            },
        ]);
        video
    }

    #[test]
    fn auto_prefers_best_h264_candidate() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::Auto).as_deref(),
            Some("bitrate-high-h264")
        );
    }

    #[test]
    fn selection_skips_watermark_candidates() {
        let mut video = VideoInfo::default();
        video.video.play_addr = "https://example.com/aweme/v1/playwm/?watermark=1".to_string();
        video.video.download_addr = Some("https://example.com/clean.mp4".to_string());

        assert_eq!(
            select_video_url(&video, DownloadQuality::Auto).as_deref(),
            Some("https://example.com/clean.mp4")
        );
    }

    #[test]
    fn selection_keeps_dash_video_only_candidates_as_fallbacks() {
        let mut video = VideoInfo::default();
        video.video.play_addr = "https://example.com/progressive.mp4".to_string();
        video.video.bit_rate = Some(vec![BitRateInfo {
            data_size: 900,
            bit_rate: 900,
            gear_name: "adapt_4k".to_string(),
            play_addr: Some("https://example.com/media-video-avc1".to_string()),
            play_addr_h264: Some("https://example.com/media_video_h264".to_string()),
            ..Default::default()
        }]);

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("https://example.com/progressive.mp4")
        );
        assert!(super::super::quality::ordered_video_urls(
            &video,
            DownloadQuality::TargetHeight(2160)
        )
        .iter()
        .any(|url| url.contains("media-video-avc1")));
    }

    #[test]
    fn selection_skips_audio_music_candidates() {
        let mut video = VideoInfo::default();
        video.video.play_addr =
            "https://sf6-cdn-tos.douyinstatic.com/obj/ies-music/123.mp3".to_string();
        video.video.play_addr_h264 = Some("https://example.com/video.mp4".to_string());

        assert_eq!(
            select_video_url(&video, DownloadQuality::Auto).as_deref(),
            Some("https://example.com/video.mp4")
        );
    }

    #[test]
    fn highest_prefers_measured_quality_candidate() {
        let video = video_with_quality_candidates();
        let selected = select_video_url(&video, DownloadQuality::Highest);
        assert!(matches!(
            selected.as_deref(),
            Some("bitrate-high" | "bitrate-high-h264")
        ));
    }

    #[test]
    fn h264_prefers_best_h264_candidate() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::H264).as_deref(),
            Some("bitrate-high-h264")
        );
    }

    #[test]
    fn smallest_prefers_lowbr_candidate() {
        let video = video_with_quality_candidates();
        assert_eq!(
            select_video_url(&video, DownloadQuality::Smallest).as_deref(),
            Some("lowbr")
        );
    }

    #[test]
    fn target_quality_prefers_closest_resolution_not_above_target() {
        let video = video_with_resolution_candidates();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(480)).as_deref(),
            Some("p480-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1080)).as_deref(),
            Some("p1080-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1440)).as_deref(),
            Some("p1440-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("p2160-h264")
        );
    }

    #[test]
    fn target_quality_is_a_maximum_height_with_downward_fallback() {
        let video = video_with_sparse_resolution_candidates();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("sparse-1080-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1080)).as_deref(),
            Some("sparse-1080-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(720)).as_deref(),
            Some("sparse-480-h264")
        );
    }

    #[test]
    fn target_quality_prefers_explicit_bitrate_quality_over_top_level_url() {
        let video = video_with_top_level_low_resolution_and_quality_candidates(false);

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("toplow-720-h264")
        );

        let video = video_with_top_level_low_resolution_and_quality_candidates(true);
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("toplow-1080-h264")
        );
    }

    #[test]
    fn target_quality_uses_short_side_for_portrait_2k_candidates() {
        let video = video_with_portrait_dimension_quality_candidates();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("portrait-2k-h264")
        );
        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(1080)).as_deref(),
            Some("portrait-720-h264")
        );
    }

    #[test]
    fn target_quality_uses_short_side_for_top_level_portrait_candidate() {
        let mut video = VideoInfo::default();
        video.video.play_addr = "top-portrait-2k".to_string();
        video.video.play_addr_h264 = Some("top-portrait-2k-h264".to_string());
        video.video.play_addr_lowbr = Some("top-portrait-low".to_string());
        video.video.width = 1440;
        video.video.height = 2560;

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("top-portrait-2k-h264")
        );
    }

    #[test]
    fn target_quality_keeps_portrait_1080_candidate_above_720() {
        let video = video_with_narrow_portrait_1080_candidate();

        assert_eq!(
            select_video_url(&video, DownloadQuality::TargetHeight(2160)).as_deref(),
            Some("narrow-1080-h264")
        );
    }

    #[test]
    fn downloader_uses_updated_quality_config() {
        let mut config = AppConfig {
            download_quality: "auto".to_string(),
            ..Default::default()
        };
        let mut downloader = Downloader::new(config.clone(), None).expect("downloader");
        let video = video_with_quality_candidates();

        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("bitrate-high-h264")
        );

        config.download_quality = "smallest".to_string();
        downloader
            .update_config(config.clone())
            .expect("update config");

        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("lowbr")
        );

        config.download_quality = "1080p".to_string();
        downloader.update_config(config).expect("update config");

        let video = video_with_resolution_candidates();
        assert_eq!(
            downloader
                .collect_download_media_items(&video)
                .first()
                .map(|item| item.url.as_str()),
            Some("p1080-h264")
        );
    }

    #[test]
    fn downloaded_set_parser_accepts_json_and_append_lines() {
        let parsed = parse_downloaded_set("[\"1001\",\"1002\"]\n1003\n1004");

        assert!(parsed.contains("1001"));
        assert!(parsed.contains("1002"));
        assert!(parsed.contains("1003"));
        assert!(parsed.contains("1004"));
    }

    #[test]
    fn extracts_only_protected_aweme_id_suffix() {
        assert_eq!(
            extract_downloaded_aweme_id("标题123456789012_7380011223344556677.mp4").as_deref(),
            Some("7380011223344556677")
        );
        assert_eq!(
            extract_downloaded_aweme_id("标题123456789012_7380011223344556677_02.jpg").as_deref(),
            Some("7380011223344556677")
        );
        assert!(extract_downloaded_aweme_id("标题123456789012.mp4").is_none());
    }

    #[test]
    fn ignores_partial_download_files_case_insensitively() {
        let path = PathBuf::from("标题_7380011223344556677.TMP");
        assert!(!is_complete_download_file(
            &path,
            path.file_name().and_then(|name| name.to_str()).unwrap()
        ));
    }

    #[test]
    fn filename_template_can_omit_aweme_id_suffix() {
        let config = AppConfig {
            filename_template: "{title}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let filename = generate_filename_with_config(
            &config,
            "这是 一个 完整 标题 第二段 文案",
            aweme_id,
            "作者",
            "video",
            0,
        );

        assert_eq!(filename, "这是 一个 完整 标题 第二段 文案");
    }

    #[test]
    fn long_filename_template_preserves_aweme_id_suffix() {
        let config = AppConfig {
            filename_template: "{title}_{aweme_id}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let filename = generate_filename_with_config(
            &config,
            &"很长标题".repeat(80),
            aweme_id,
            "作者",
            "image",
            0,
        );

        assert!(filename.ends_with(aweme_id));
        assert!(filename.len() <= MAX_FILENAME_BYTES);
    }

    #[test]
    fn long_filename_template_keeps_more_safe_title_text() {
        let config = AppConfig {
            filename_template: "{title}_{aweme_id}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let filename = generate_filename_with_config(
            &config,
            &"abcdefghijklmnopqrstuvwxyz".repeat(8),
            aweme_id,
            "作者",
            "video",
            0,
        );

        assert!(filename.starts_with(&"abcdefghijklmnopqrstuvwxyz".repeat(6)));
        assert!(filename.ends_with(aweme_id));
        assert!(filename.len() <= MAX_FILENAME_BYTES);
    }

    #[test]
    fn filename_template_uses_work_create_time_for_date_tokens() {
        let config = AppConfig {
            filename_template: "{date}_{time}_{title}_{aweme_id}".to_string(),
            ..Default::default()
        };
        let aweme_id = "7380011223344556677";
        let create_time = 1_704_067_205;
        let expected_prefix = Local
            .timestamp_opt(create_time, 0)
            .single()
            .unwrap()
            .format("%Y%m%d_%Y%m%d_%H%M%S")
            .to_string();
        let filename = generate_filename_with_config(
            &config,
            "跨年作品",
            aweme_id,
            "作者",
            "video",
            create_time,
        );

        assert_eq!(
            filename,
            format!("{}_跨年作品_{}", expected_prefix, aweme_id)
        );
    }

    #[test]
    fn filename_template_leaves_date_tokens_empty_without_create_time() {
        let config = AppConfig {
            filename_template: "{date}_{time}_{title}_{aweme_id}".to_string(),
            ..Default::default()
        };

        let filename = generate_filename_with_config(
            &config,
            "无发布时间作品",
            "7380011223344556677",
            "作者",
            "video",
            0,
        );

        assert_eq!(filename, "无发布时间作品_7380011223344556677");
    }

    #[test]
    fn output_dir_respects_folder_template_toggle() {
        let base = PathBuf::from("/tmp/douyin");
        let mut config = AppConfig {
            folder_name_template: "{author}_{media_type}".to_string(),
            auto_create_folder: true,
            ..Default::default()
        };

        assert_eq!(
            build_output_dir(&config, &base, "作者/名", "video", 0),
            base.join("作者_名_video")
        );

        config.auto_create_folder = false;
        assert_eq!(build_output_dir(&config, &base, "作者", "video", 0), base);
    }

    #[test]
    fn author_name_with_asterisk_is_sanitized_for_output_dir() {
        let base = PathBuf::from("/tmp/douyin");
        let config = AppConfig {
            folder_name_template: "{author}".to_string(),
            auto_create_folder: true,
            ..Default::default()
        };

        assert_eq!(
            build_output_dir(&config, &base, "作者*星号", "video", 0),
            base.join("作者_星号")
        );
    }

    #[tokio::test]
    async fn unique_output_file_does_not_overwrite_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("better-douyin-r-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let existing = dir.join("clip.mp4");
        std::fs::write(&existing, b"existing").expect("write existing file");

        let (created_path, created_file) = create_unique_output_file(&dir, "clip", 0, 1, "mp4")
            .await
            .expect("create unique file");
        drop(created_file);

        assert_ne!(created_path, existing);
        assert_eq!(
            created_path.file_name().and_then(|name| name.to_str()),
            Some("clip_2.mp4")
        );
        assert!(created_path.exists());
        assert_eq!(
            std::fs::read(&existing).expect("read existing"),
            b"existing"
        );

        std::fs::remove_dir_all(&dir).expect("cleanup test dir");
    }

    #[test]
    fn media_extension_prefers_content_type_then_url_then_media_type() {
        assert_eq!(
            media_extension("image", "https://example.test/file", Some("image/webp")),
            "webp"
        );
        assert_eq!(
            media_extension("image", "https://example.test/file.png?x=1", None),
            "png"
        );
        assert_eq!(
            media_extension("live_photo", "https://example.test/play", None),
            "mp4"
        );
    }
}
