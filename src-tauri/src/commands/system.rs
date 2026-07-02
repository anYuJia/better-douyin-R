use std::path::{Path, PathBuf};
use tauri::State;

use crate::system_open::{
    canonical_existing_directory, canonical_existing_file, open_directory_with_system,
    open_external_url_with_system, open_file_with_system, reveal_file_with_system,
    write_text_to_clipboard,
};
use crate::AppState;

pub(crate) async fn allowed_existing_file_path(
    state: &State<'_, AppState>,
    raw_path: &str,
) -> Result<PathBuf, String> {
    let target = canonical_existing_file(raw_path)?;

    let download_path = {
        let config = state.config.lock().await;
        config.download_path.clone()
    };

    if !download_path.trim().is_empty() {
        if let Ok(download_root) = Path::new(&download_path).canonicalize() {
            if target.starts_with(download_root) {
                return Ok(target);
            }
        }
    }

    let history_items = {
        let history = state.history.lock().await;
        history.get_all()
    };

    let is_history_file = history_items.iter().any(|item| {
        Path::new(&item.file_path)
            .canonicalize()
            .map(|history_path| history_path == target)
            .unwrap_or(false)
    });

    if is_history_file {
        Ok(target)
    } else {
        Err("仅允许操作下载目录或下载历史中的文件".to_string())
    }
}

pub(crate) async fn configured_download_directory(
    state: &State<'_, AppState>,
) -> Result<PathBuf, String> {
    let download_path = {
        let config = state.config.lock().await;
        config.download_path.clone()
    };

    let trimmed = download_path.trim();
    if trimmed.is_empty() {
        return Err("下载目录未设置".to_string());
    }

    std::fs::create_dir_all(trimmed).map_err(|e| format!("创建下载目录失败: {e}"))?;
    canonical_existing_directory(trimmed)
}

pub(crate) async fn allowed_existing_download_directory_path(
    state: &State<'_, AppState>,
    raw_path: &str,
) -> Result<PathBuf, String> {
    let target = canonical_existing_directory(raw_path)?;
    let download_root = configured_download_directory(state).await?;

    if target.starts_with(download_root) {
        Ok(target)
    } else {
        Err("仅允许打开下载目录下的文件夹".to_string())
    }
}

#[tauri::command]
pub(crate) async fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    if text.is_empty() {
        return Err("复制内容不能为空".to_string());
    }
    write_text_to_clipboard(&text)
}

#[tauri::command]
pub(crate) async fn open_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let target = allowed_existing_file_path(&state, &path).await?;
    open_file_with_system(&target)
}

#[tauri::command]
pub(crate) async fn open_download_directory(state: State<'_, AppState>) -> Result<(), String> {
    let target = configured_download_directory(&state).await?;
    open_directory_with_system(&target)
}

#[tauri::command]
pub(crate) async fn open_file_location(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let canonical = Path::new(path.trim())
        .canonicalize()
        .map_err(|_| "文件或目录不存在或无法访问".to_string())?;

    if canonical.is_dir() {
        let target = allowed_existing_download_directory_path(&state, &path).await?;
        return open_directory_with_system(&target);
    }

    let target = allowed_existing_file_path(&state, &path).await?;
    reveal_file_with_system(&target)
}

#[tauri::command]
pub(crate) async fn open_external_url(url: String) -> Result<(), String> {
    let target = url.trim();
    if target.is_empty() {
        return Err("链接不能为空".to_string());
    }
    open_external_url_with_system(target)
}

#[tauri::command]
pub(crate) async fn delete_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let target = allowed_existing_file_path(&state, &path).await?;
    std::fs::remove_file(target).map_err(|e| e.to_string())?;
    *state.download_file_index.lock().await = None;
    Ok(())
}
