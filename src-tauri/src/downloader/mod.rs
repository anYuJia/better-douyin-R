//! 下载模块

mod downloaded_cache;
mod events;
mod filename;
mod http;
#[allow(clippy::module_inception)]
pub mod downloader;
mod media_group;
mod media_request;
mod quality;

pub(crate) use quality::{available_video_quality_height, video_quality_candidate_count};
pub use quality::video_quality_diagnostic;
pub use downloader::{Downloader, DownloaderEvent};
