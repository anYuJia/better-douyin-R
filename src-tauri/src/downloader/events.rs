//! 下载事件和进度辅助

use anyhow::Result;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::control::DownloadControl;
use super::downloader::DownloaderEvent;

pub(crate) const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(650);

pub(crate) async fn emit_event(
    sender: &Option<mpsc::Sender<DownloaderEvent>>,
    name: &'static str,
    payload: serde_json::Value,
) {
    if let Some(tx) = sender {
        let _ = tx.send(DownloaderEvent { name, payload }).await;
    }
}

pub(crate) async fn wait_if_control_paused(control: &DownloadControl) -> Result<()> {
    control.wait_if_paused().await
}

pub(crate) fn estimate_batch_eta(
    processed_count: usize,
    total_count: usize,
    started_at: Instant,
) -> Option<u64> {
    if processed_count == 0 || total_count == 0 || processed_count >= total_count {
        return None;
    }

    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
    let remaining = total_count.saturating_sub(processed_count) as f64;
    Some(
        ((remaining * elapsed) / processed_count as f64)
            .ceil()
            .max(1.0) as u64,
    )
}
