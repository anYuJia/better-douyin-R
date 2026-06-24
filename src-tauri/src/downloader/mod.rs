//! 下载模块

#[allow(clippy::module_inception)]
pub mod downloader;

pub(crate) use downloader::{available_video_quality_height, video_quality_candidate_count};
pub use downloader::video_quality_diagnostic;
pub use downloader::{Downloader, DownloaderEvent};
