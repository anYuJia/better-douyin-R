use tauri::Emitter;
use tauri::State;

use crate::update::{
    configure_updater_builder, is_windows_portable_runtime, update_content_length,
    updater_install_mode,
};
use crate::AppState;

#[tauri::command]
pub(crate) fn get_app_version(app_handle: tauri::AppHandle) -> String {
    app_handle.package_info().version.to_string()
}

#[tauri::command]
pub(crate) fn restart_app(app_handle: tauri::AppHandle) {
    app_handle.request_restart();
}

#[tauri::command]
pub(crate) async fn check_update(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;

    let portable = is_windows_portable_runtime();
    let manual_proxy = state.config.lock().await.proxy.clone();
    let mut updater_builder =
        configure_updater_builder(app_handle.updater_builder(), manual_proxy.as_deref());
    if portable {
        updater_builder = updater_builder.target("windows-x86_64-portable");
    }

    let updater = updater_builder.build().map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(Some(update)) => {
            let asset_size = update_content_length(&update.download_url).await;
            Ok(serde_json::json!({
                "success": true,
                "has_update": true,
                "version": update.version.clone(),
                "current_version": update.current_version.clone(),
                "notes": update.body.unwrap_or_else(|| "无更新说明".to_string()),
                "date": update.date.map(|d| d.to_string()),
                "download_url": update.download_url.to_string(),
                "asset_size": asset_size,
                "portable": portable,
                "install_mode": updater_install_mode()
            }))
        }
        Ok(None) => Ok(serde_json::json!({
            "success": true,
            "has_update": false,
            "portable": portable,
            "install_mode": updater_install_mode()
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "portable": portable,
            "install_mode": updater_install_mode(),
            "message": format!("检查更新失败: {}", e)
        })),
    }
}

#[tauri::command]
pub(crate) async fn download_update(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;

    let portable = is_windows_portable_runtime();
    let manual_proxy = state.config.lock().await.proxy.clone();
    let mut updater_builder =
        configure_updater_builder(app_handle.updater_builder(), manual_proxy.as_deref());
    if portable {
        updater_builder = updater_builder.target("windows-x86_64-portable");
    }

    let updater = updater_builder.build().map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(Some(update)) => {
            #[cfg(windows)]
            if portable {
                return crate::update::download_portable_update(app_handle, update).await;
            } else {
                return crate::update::download_nsis_update(app_handle, update).await;
            }

            let progress_app = app_handle.clone();
            let finished_app = app_handle.clone();
            let mut downloaded = 0u64;
            let started_at = std::time::Instant::now();

            match update
                .download_and_install(
                    move |chunk_len, content_len| {
                        downloaded += chunk_len as u64;
                        let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                        let speed_bps = (downloaded as f64 / elapsed) as u64;
                        let progress = content_len
                            .filter(|total| *total > 0)
                            .map(|total| downloaded as f64 / total as f64 * 100.0);
                        let _ = progress_app.emit(
                            "update-download-progress",
                            serde_json::json!({
                                "downloaded": downloaded,
                                "total": content_len,
                                "progress": progress,
                                "speed_bps": speed_bps
                            }),
                        );
                    },
                    move || {
                        let _ = finished_app.emit(
                            "update-download-finished",
                            serde_json::json!({
                                "success": true,
                                "restart_required": true,
                                "message": "更新安装完成，重启后使用新版本"
                            }),
                        );
                    },
                )
                .await
            {
                Ok(_) => Ok(serde_json::json!({
                    "success": true,
                    "restart_required": true,
                    "message": "更新安装完成，重启后使用新版本"
                })),
                Err(e) => {
                    log::error!("failed to download and install update: {}", e);
                    let _ = app_handle.emit(
                        "update-download-error",
                        serde_json::json!({
                            "success": false,
                            "message": e.to_string()
                        }),
                    );
                    Ok(serde_json::json!({
                        "success": false,
                        "message": format!("下载更新失败: {}", e)
                    }))
                }
            }
        }
        Ok(None) => Ok(serde_json::json!({
            "success": false,
            "message": "没有可用更新"
        })),
        Err(e) => Ok(serde_json::json!({
            "success": false,
            "message": format!("下载更新失败: {}", e)
        })),
    }
}
