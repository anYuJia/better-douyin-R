use tauri::State;

use crate::download_files::{
    build_download_file_index, download_file_matches_query, DownloadFileEntry,
    DOWNLOAD_FILE_INDEX_TTL,
};
use crate::AppState;

use super::system::configured_download_directory;

#[tauri::command]
pub(crate) async fn list_download_files(
    state: State<'_, AppState>,
    offset: Option<usize>,
    limit: Option<usize>,
    force_refresh: Option<bool>,
    query: Option<String>,
    media_type: Option<String>,
    sort_by: Option<String>,
) -> Result<serde_json::Value, String> {
    let target = configured_download_directory(&state).await?;
    let use_cache = !force_refresh.unwrap_or(false);
    let cached_index = if use_cache {
        state.download_file_index.lock().await.clone()
    } else {
        None
    };

    let index = if let Some(cache) = cached_index {
        if cache.directory == target && cache.scanned_at.elapsed() <= DOWNLOAD_FILE_INDEX_TTL {
            cache
        } else {
            build_download_file_index(target, state.download_file_index.clone()).await?
        }
    } else {
        build_download_file_index(target, state.download_file_index.clone()).await?
    };

    let query = query.unwrap_or_default().trim().to_lowercase();
    let media_type = media_type
        .unwrap_or_else(|| "all".to_string())
        .trim()
        .to_lowercase();
    let sort_by = sort_by.unwrap_or_else(|| "date_desc".to_string());

    let mut filtered_items: Vec<DownloadFileEntry> = index
        .items
        .into_iter()
        .filter(|item| {
            download_file_matches_query(item, &query)
                && (media_type == "all" || item.media_type.to_lowercase() == media_type)
        })
        .collect();

    match sort_by.as_str() {
        "date_asc" => filtered_items.sort_by_key(|item| item.timestamp),
        "size_desc" => filtered_items.sort_by_key(|item| std::cmp::Reverse(item.size)),
        "size_asc" => filtered_items.sort_by_key(|item| item.size),
        _ => filtered_items.sort_by_key(|item| std::cmp::Reverse(item.timestamp)),
    }

    let total = filtered_items.len();
    let total_size = filtered_items.iter().map(|item| item.size).sum::<u64>();
    let latest = filtered_items.first().cloned();
    let items = match (offset, limit) {
        (Some(offset), Some(limit)) => filtered_items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect(),
        (Some(offset), None) => filtered_items.into_iter().skip(offset).collect(),
        (None, Some(limit)) => filtered_items.into_iter().take(limit).collect(),
        (None, None) => filtered_items,
    };

    Ok(serde_json::json!({
        "success": true,
        "items": items,
        "total": total,
        "total_size": total_size,
        "latest": latest
    }))
}

#[tauri::command]
pub(crate) fn check_files_exist(paths: Vec<String>) -> Result<Vec<bool>, String> {
    let result = paths
        .into_iter()
        .map(|path| std::path::Path::new(&path).exists())
        .collect();
    Ok(result)
}
