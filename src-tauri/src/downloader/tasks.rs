use crate::api::types::{DownloadMediaItem, DownloadStatus, DownloadTask, MediaType, VideoInfo};
use crate::api::DouyinClient;
use crate::downloader::downloader::Downloader;
use crate::downloader::events::emit_event;
use crate::downloader::filename::{
    build_output_dir, generate_filename_with_config, media_type_name,
};
use crate::downloader::quality::{ordered_video_urls, DownloadQuality};
use crate::media_utils::{filter_live_photo_media_items, push_image_like_items};
use anyhow::{anyhow, Result};
use chrono::Local;
use std::path::PathBuf;

impl Downloader {
    /// 添加视频下载任务
    pub async fn add_task(&self, video: &VideoInfo, save_path: Option<PathBuf>) -> Result<String> {
        let base_path = save_path.unwrap_or_else(|| PathBuf::from(&self.config.download_path));

        let media_urls = self.collect_download_media_items(video);
        self.add_media_task(
            video.aweme_id.clone(),
            video.desc.clone(),
            video.author.nickname.clone(),
            video.author.uid.clone(),
            video.video.cover.clone(),
            video.media_type.clone(),
            media_urls,
            video.create_time,
            Some(base_path),
        )
        .await
    }

    /// 添加媒体组下载任务
    #[allow(clippy::too_many_arguments)]
    pub async fn add_media_task(
        &self,
        aweme_id: String,
        title: String,
        author: String,
        author_id: String,
        cover: String,
        media_type: MediaType,
        media_urls: Vec<DownloadMediaItem>,
        published_at: i64,
        save_path: Option<PathBuf>,
    ) -> Result<String> {
        if media_urls.is_empty() {
            return Err(anyhow!("No media URLs"));
        }

        let task_id = uuid::Uuid::new_v4().to_string();
        let base_path = save_path.unwrap_or_else(|| PathBuf::from(&self.config.download_path));
        let author_dir = build_output_dir(
            &self.config,
            &base_path,
            &author,
            media_type_name(&media_type),
            published_at,
        );
        let filename = generate_filename_with_config(
            &self.config,
            &title,
            &aweme_id,
            &author,
            media_type_name(&media_type),
            published_at,
        );

        let task = DownloadTask {
            id: task_id.clone(),
            aweme_id,
            url: media_urls
                .first()
                .map(|item| item.url.clone())
                .unwrap_or_default(),
            media_urls: media_urls.clone(),
            title,
            author,
            author_id,
            cover,
            save_path: author_dir.to_string_lossy().to_string(),
            filename,
            media_type,
            total_files: media_urls.len() as u32,
            completed_files: 0,
            status: DownloadStatus::Pending,
            progress: 0.0,
            total_size: 0,
            downloaded_size: 0,
            error_msg: None,
            create_time: Local::now().timestamp(),
            complete_time: None,
            image_urls: None,
        };

        self.tasks.lock().await.push(task);
        Ok(task_id)
    }

    pub(crate) fn collect_download_media_items(&self, video: &VideoInfo) -> Vec<DownloadMediaItem> {
        let mut items = Vec::new();

        if let Some(urls) = &video.live_photo_urls {
            push_image_like_items(
                &mut items,
                urls,
                video.live_photo_url_candidates.as_ref(),
                "live_photo",
            );
        }

        if let Some(urls) = &video.image_urls {
            push_image_like_items(
                &mut items,
                urls,
                video.image_url_candidates.as_ref(),
                "image",
            );
        }

        if items.is_empty() {
            let video_urls = ordered_video_urls(
                video,
                DownloadQuality::from_config(&self.config.download_quality),
            );
            if let Some(url) = video_urls.first() {
                items.push(DownloadMediaItem {
                    r#type: "video".to_string(),
                    url: url.clone(),
                    fallback_urls: video_urls.iter().skip(1).cloned().collect(),
                });
            } else if let Some(url) = DouyinClient::get_no_watermark_url(video) {
                items.push(DownloadMediaItem {
                    r#type: "video".to_string(),
                    url,
                    fallback_urls: Vec::new(),
                });
            }
        }

        filter_live_photo_media_items(items, &self.config)
    }

    pub async fn get_tasks(&self) -> Vec<DownloadTask> {
        self.tasks.lock().await.clone()
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<()> {
        let mut tokens = self.cancel_tokens.lock().await;
        tokens.insert(task_id.to_string(), true);

        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = DownloadStatus::Cancelled;
        }

        Ok(())
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<()> {
        let mut tokens = self.pause_tokens.lock().await;
        tokens.insert(task_id.to_string(), true);

        let mut progress = 0.0;
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = DownloadStatus::Paused;
            progress = task.progress;
        }
        drop(tasks);

        emit_event(
            &self.progress_tx,
            "download-progress",
            serde_json::json!({
                "task_id": task_id,
                "progress": progress,
                "status": "paused",
                "speed_bps": 0
            }),
        )
        .await;

        Ok(())
    }

    pub async fn resume_task(&self, task_id: &str) -> Result<()> {
        let mut tokens = self.pause_tokens.lock().await;
        tokens.insert(task_id.to_string(), false);

        let mut progress = 0.0;
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            if task.status == DownloadStatus::Paused {
                task.status = DownloadStatus::Downloading;
            }
            progress = task.progress;
        }
        drop(tasks);

        emit_event(
            &self.progress_tx,
            "download-progress",
            serde_json::json!({
                "task_id": task_id,
                "progress": progress,
                "status": "downloading"
            }),
        )
        .await;

        Ok(())
    }

    pub async fn remove_task(&self, task_id: &str) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|t| t.id != task_id);

        let mut tokens = self.cancel_tokens.lock().await;
        tokens.remove(task_id);

        let mut pause_tokens = self.pause_tokens.lock().await;
        pause_tokens.remove(task_id);

        Ok(())
    }

    /// 发送批量下载开始事件
    pub async fn emit_batch_started(&self, task_id: &str, nickname: &str, total_videos: usize) {
        emit_event(
            &self.progress_tx,
            "batch-download-started",
            serde_json::json!({
                "task_id": task_id,
                "nickname": nickname,
                "total_videos": total_videos,
                "message": format!("开始下载 {} 个视频", total_videos)
            }),
        )
        .await;
    }
}
