//! 下载模块

mod batch;
mod completion;
mod control;
mod downloaded_cache;
#[allow(clippy::module_inception)]
pub mod downloader;
mod events;
mod filename;
mod http;
mod media_group;
mod media_request;
mod quality;
mod streaming;
mod tasks;

pub use downloader::{Downloader, DownloaderEvent};
pub use quality::video_quality_diagnostic;
pub(crate) use quality::{available_video_quality_height, video_quality_candidate_count};
