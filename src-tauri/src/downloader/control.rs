use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub(crate) struct DownloadControl {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl DownloadControl {
    pub(crate) fn new(cancelled: bool, paused: bool) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(cancelled)),
            paused: Arc::new(AtomicBool::new(paused)),
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub(crate) async fn wait_if_paused(&self) -> Result<()> {
        loop {
            if self.is_cancelled() {
                return Err(anyhow!("Download cancelled"));
            }
            if !self.is_paused() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

pub(crate) type DownloadControls = Arc<Mutex<HashMap<String, DownloadControl>>>;

pub(crate) async fn ensure_control(controls: &DownloadControls, task_id: &str) -> DownloadControl {
    let mut controls_lock = controls.lock().await;
    controls_lock
        .entry(task_id.to_string())
        .or_insert_with(|| DownloadControl::new(false, false))
        .clone()
}

pub(crate) async fn get_control(
    controls: &DownloadControls,
    task_id: &str,
) -> Option<DownloadControl> {
    controls.lock().await.get(task_id).cloned()
}

pub(crate) async fn remove_control(controls: &DownloadControls, task_id: &str) {
    controls.lock().await.remove(task_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn control_tracks_pause_resume_and_cancel_without_map_lock() {
        let control = DownloadControl::new(false, false);
        assert!(!control.is_cancelled());
        assert!(!control.is_paused());

        control.set_paused(true);
        assert!(control.is_paused());
        control.set_paused(false);
        assert!(!control.is_paused());

        control.cancel();
        assert!(control.is_cancelled());
        assert!(control.wait_if_paused().await.is_err());
    }

    #[tokio::test]
    async fn ensure_control_reuses_existing_handle() {
        let controls: DownloadControls = Arc::new(Mutex::new(HashMap::new()));
        let first = ensure_control(&controls, "task").await;
        first.set_paused(true);

        let second = ensure_control(&controls, "task").await;
        assert!(second.is_paused());
        second.cancel();
        assert!(first.is_cancelled());

        remove_control(&controls, "task").await;
        assert!(get_control(&controls, "task").await.is_none());
    }
}
