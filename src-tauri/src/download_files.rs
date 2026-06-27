//! 下载目录扫描和文件索引 helper。

use serde::Serialize;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub(crate) const DOWNLOAD_FILE_INDEX_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DownloadFileEntry {
    pub(crate) id: String,
    pub(crate) filename: String,
    pub(crate) path: String,
    pub(crate) author: String,
    pub(crate) desc: String,
    pub(crate) size: u64,
    pub(crate) timestamp: i64,
    pub(crate) file_type: String,
    pub(crate) media_type: String,
}

#[derive(Clone)]
pub(crate) struct DownloadFileIndexCache {
    pub(crate) directory: std::path::PathBuf,
    pub(crate) scanned_at: Instant,
    pub(crate) items: Vec<DownloadFileEntry>,
}

pub(crate) fn is_hidden_download_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

pub(crate) fn download_file_media_kind(path: &Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" | "flv" => Some("video"),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "heic" | "heif" => Some("image"),
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" => Some("audio"),
        _ => None,
    }
}

pub(crate) fn download_file_matches_query(item: &DownloadFileEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    [
        item.filename.as_str(),
        item.author.as_str(),
        item.desc.as_str(),
        item.id.as_str(),
        item.path.as_str(),
        item.file_type.as_str(),
        item.media_type.as_str(),
    ]
    .iter()
    .any(|value| value.to_lowercase().contains(query))
}

pub(crate) fn scan_download_directory_entries(
    dir: &Path,
    items: &mut Vec<DownloadFileEntry>,
) -> Result<(), String> {
    let read_dir = fs::read_dir(dir).map_err(|e| e.to_string())?;

    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if is_hidden_download_path(&path) {
            continue;
        }

        let metadata = entry.metadata().map_err(|e| e.to_string())?;

        if metadata.is_dir() {
            scan_download_directory_entries(&path, items)?;
            continue;
        }

        if !metadata.is_file() {
            continue;
        }

        let Some(media_kind) = download_file_media_kind(&path) else {
            continue;
        };

        let filename = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("未命名文件")
            .to_string();
        let author = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let timestamp = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let file_type = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();

        items.push(DownloadFileEntry {
            id: path.to_string_lossy().to_string(),
            filename,
            path: path.to_string_lossy().to_string(),
            author,
            desc: String::new(),
            size: metadata.len(),
            timestamp,
            file_type: file_type.clone(),
            media_type: media_kind.to_string(),
        });
    }

    Ok(())
}

pub(crate) async fn build_download_file_index(
    target: std::path::PathBuf,
    cache_store: Arc<Mutex<Option<DownloadFileIndexCache>>>,
) -> Result<DownloadFileIndexCache, String> {
    let cache = tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        scan_download_directory_entries(&target, &mut items)?;
        items.sort_by_key(|item| std::cmp::Reverse(item.timestamp));
        Ok::<_, String>(DownloadFileIndexCache {
            directory: target,
            scanned_at: Instant::now(),
            items,
        })
    })
    .await
    .map_err(|error| format!("扫描下载目录任务失败: {error}"))??;
    *cache_store.lock().await = Some(cache.clone());
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
