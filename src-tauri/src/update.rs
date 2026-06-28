//! 更新器相关 helper。

#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use tauri::{Emitter, Manager};

#[cfg(windows)]
pub(crate) fn is_windows_portable_runtime() -> bool {
    tauri::utils::platform::bundle_type().is_none()
}

#[cfg(not(windows))]
pub(crate) fn is_windows_portable_runtime() -> bool {
    false
}

pub(crate) fn updater_install_mode() -> &'static str {
    if is_windows_portable_runtime() {
        "portable"
    } else {
        "bundled"
    }
}

pub(crate) async fn update_content_length(url: &url::Url) -> Option<u64> {
    reqwest::Client::new()
        .head(url.as_str())
        .send()
        .await
        .ok()?
        .content_length()
}

#[cfg(windows)]
pub(crate) fn powershell_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[cfg(windows)]
pub(crate) async fn download_portable_update(
    app_handle: tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> Result<serde_json::Value, String> {
    use std::process::Command;

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "无法确定当前便携版程序目录".to_string())?;
    let exe_stem = current_exe
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("better-douyin-R");
    let update_path = exe_dir.join(format!("{exe_stem}.update.exe"));
    let script_path = exe_dir.join(format!("{exe_stem}.update.ps1"));

    let progress_app = app_handle.clone();
    let mut downloaded = 0u64;
    let started_at = std::time::Instant::now();
    let bytes = update
        .download(
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
            || {},
        )
        .await
        .map_err(|e| format!("下载便携版更新失败: {e}"))?;

    std::fs::write(&update_path, bytes)
        .map_err(|e| format!("写入新版便携程序失败: {} ({})", update_path.display(), e))?;

    let target = powershell_quote_path(&current_exe);
    let update_file = powershell_quote_path(&update_path);
    let script_file = powershell_quote_path(&script_path);
    let pid = std::process::id();
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$pidToWait = {pid}
$target = {target}
$update = {update_file}
$backup = "$target.bak"
$log = "$target.update.log"
try {{
  Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 800
  if (Test-Path -LiteralPath $backup) {{
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
  }}
  if (Test-Path -LiteralPath $target) {{
    Move-Item -LiteralPath $target -Destination $backup -Force
  }}
  Move-Item -LiteralPath $update -Destination $target -Force
  Start-Process -FilePath $target
  Start-Sleep -Seconds 2
  Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
}} catch {{
  if ((Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $target)) {{
    Move-Item -LiteralPath $backup -Destination $target -Force -ErrorAction SilentlyContinue
  }}
  Add-Content -LiteralPath $log -Value $_.Exception.ToString()
}} finally {{
  Remove-Item -LiteralPath {script_file} -Force -ErrorAction SilentlyContinue
}}
"#
    );
    std::fs::write(&script_path, script).map_err(|e| format!("写入便携版替换脚本失败: {}", e))?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .spawn()
        .map_err(|e| format!("启动便携版替换脚本失败: {}", e))?;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app_handle.exit(0);
    });

    Ok(serde_json::json!({
        "success": true,
        "portable": true,
        "message": "便携版更新已下载，应用即将关闭并自动替换重启"
    }))
}

#[cfg(windows)]
pub(crate) async fn download_nsis_update(
    app_handle: tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> Result<serde_json::Value, String> {
    use std::process::Command;

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let temp_dir = std::env::temp_dir();
    let exe_stem = current_exe
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("better-douyin-R");
    let installer_path = temp_dir.join(format!("{exe_stem}-installer.exe"));
    let script_path = temp_dir.join(format!("{exe_stem}-installer.ps1"));

    let progress_app = app_handle.clone();
    let mut downloaded = 0u64;
    let started_at = std::time::Instant::now();
    let bytes = update
        .download(
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
            || {},
        )
        .await
        .map_err(|e| format!("下载安装版更新失败: {e}"))?;

    std::fs::write(&installer_path, bytes)
        .map_err(|e| format!("写入新版安装包失败: {} ({})", installer_path.display(), e))?;

    let target = powershell_quote_path(&current_exe);
    let installer_file = powershell_quote_path(&installer_path);
    let script_file = powershell_quote_path(&script_path);
    let pid = std::process::id();
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$pidToWait = {pid}
$target = {target}
$installer = {installer_file}
$log = "$installer.log"
try {{
  Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 1500
  $proc = Start-Process -FilePath $installer -ArgumentList '/S' -PassThru -Wait
  if ($proc.ExitCode -ne 0) {{
    throw "Installer exited with code $($proc.ExitCode)"
  }}
  Start-Process -FilePath $target
}} catch {{
  Add-Content -LiteralPath $log -Value $_.Exception.ToString()
}} finally {{
  Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath {script_file} -Force -ErrorAction SilentlyContinue
}}
"#
    );
    std::fs::write(&script_path, script).map_err(|e| format!("写入安装版替换脚本失败: {}", e))?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .spawn()
        .map_err(|e| format!("启动安装版替换脚本失败: {}", e))?;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        app_handle.exit(0);
    });

    Ok(serde_json::json!({
        "success": true,
        "portable": false,
        "message": "更新已下载，应用即将关闭并自动安装重启"
    }))
}
