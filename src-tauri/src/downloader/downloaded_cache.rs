//! 已下载缓存管理

use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub(crate) async fn load_downloaded_set(dir: &Path) -> HashSet<String> {
    let record_path = dir.join(".downloaded");
    if !record_path.exists() {
        return HashSet::new();
    }
    match tokio::fs::read_to_string(&record_path).await {
        Ok(content) => parse_downloaded_set(&content),
        Err(_) => HashSet::new(),
    }
}

/// 将 aweme_id 写入作者目录的隐藏文件 `.downloaded`
pub(crate) async fn record_downloaded(
    dir: &Path,
    aweme_id: &str,
    write_lock: &Arc<Mutex<()>>,
) -> Result<()> {
    let _guard = write_lock.lock().await;
    let record_path = dir.join(".downloaded");
    tokio::fs::create_dir_all(dir).await?;

    let aweme_id = aweme_id.trim();
    if aweme_id.is_empty() {
        return Ok(());
    }

    let mut set = load_downloaded_set(dir).await;
    if set.insert(aweme_id.to_string()) {
        let temp_path = record_path.with_extension("downloaded.tmp");
        let mut lines = set.into_iter().collect::<Vec<_>>();
        lines.sort();
        let content = format!("{}\n", lines.join("\n"));
        let mut file = File::create(&temp_path).await?;
        file.write_all(content.as_bytes()).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp_path, &record_path).await?;
    }
    Ok(())
}

pub(crate) fn parse_downloaded_set(content: &str) -> HashSet<String> {
    if let Ok(set) = serde_json::from_str::<HashSet<String>>(content) {
        return set;
    }

    let mut set = HashSet::new();
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Ok(line_set) = serde_json::from_str::<HashSet<String>>(line) {
            set.extend(line_set);
        } else {
            set.insert(line.to_string());
        }
    }
    set
}

pub(crate) fn load_all_downloaded_set(root: &Path) -> HashSet<String> {
    let mut recorded_ids = HashSet::new();
    let mut file_ids = HashSet::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some(".downloaded") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    recorded_ids.extend(parse_downloaded_set(&content));
                }
            } else if let Some(filename) = path.file_name().and_then(|name| name.to_str()) {
                if !is_complete_download_file(&path, filename) {
                    continue;
                }
                if let Some(aweme_id) = extract_downloaded_aweme_id(filename) {
                    file_ids.insert(aweme_id);
                }
            }
        }
    }

    recorded_ids.intersection(&file_ids).cloned().collect()
}

pub(crate) fn is_complete_download_file(path: &Path, filename: &str) -> bool {
    let lower_filename = filename.to_ascii_lowercase();
    if filename.is_empty()
        || filename.starts_with('.')
        || lower_filename == "download_record.json"
        || lower_filename.ends_with(".tmp")
        || lower_filename.ends_with(".part")
        || lower_filename.ends_with(".download")
        || lower_filename.ends_with(".crdownload")
    {
        return false;
    }

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 4096)
        .unwrap_or(false)
}

pub(crate) fn extract_downloaded_aweme_id(filename: &str) -> Option<String> {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())?;
    let parts = stem.rsplit('_').collect::<Vec<_>>();
    let candidate = match parts.as_slice() {
        [index, aweme_id, ..]
            if index.len() == 2 && index.chars().all(|ch| ch.is_ascii_digit()) =>
        {
            *aweme_id
        }
        [aweme_id, ..] => *aweme_id,
        _ => return None,
    };

    if (10..=25).contains(&candidate.len()) && candidate.chars().all(|ch| ch.is_ascii_digit()) {
        Some(candidate.to_string())
    } else {
        None
    }
}

pub(crate) async fn ensure_downloaded_cache(
    download_path: String,
    cache: &Arc<RwLock<HashSet<String>>>,
    loaded: &Arc<AtomicBool>,
) {
    if loaded.load(Ordering::Acquire) {
        return;
    }

    let root = PathBuf::from(download_path);
    let scanned = tokio::task::spawn_blocking(move || load_all_downloaded_set(&root)).await;
    let Ok(scanned) = scanned else {
        return;
    };

    if let Ok(mut cache_lock) = cache.write() {
        cache_lock.extend(scanned);
        loaded.store(true, Ordering::Release);
    }
}

pub(crate) fn add_to_downloaded_cache(cache: &Arc<RwLock<HashSet<String>>>, aweme_id: &str) {
    let aweme_id = aweme_id.trim();
    if aweme_id.is_empty() {
        return;
    }

    if let Ok(mut cache_lock) = cache.write() {
        cache_lock.insert(aweme_id.to_string());
    }
}
