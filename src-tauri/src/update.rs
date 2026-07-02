//! 更新器相关 helper。

use std::time::Duration;

#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use tauri::{Emitter, Manager};
use url::Url;

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

pub(crate) fn configure_updater_builder(
    builder: tauri_plugin_updater::UpdaterBuilder,
    manual_proxy: Option<&str>,
) -> tauri_plugin_updater::UpdaterBuilder {
    let builder = builder.timeout(Duration::from_secs(30));
    if let Some(proxy) = updater_proxy_url(manual_proxy) {
        log::info!("using updater proxy {}", redact_proxy_url(&proxy));
        builder.proxy(proxy)
    } else {
        builder
    }
}

fn updater_proxy_url(manual_proxy: Option<&str>) -> Option<Url> {
    manual_proxy
        .and_then(parse_proxy_url)
        .or_else(proxy_from_env)
        .or_else(windows_proxy_from_system)
}

fn proxy_from_env() -> Option<Url> {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .filter_map(|key| std::env::var(key).ok())
    .find_map(|value| parse_proxy_url(&value))
}

fn parse_proxy_url(value: &str) -> Option<Url> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if value.is_empty() || value.eq_ignore_ascii_case("direct") {
        return None;
    }

    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };

    let url = Url::parse(&candidate).ok()?;
    match url.scheme() {
        "http" | "https" => Some(url),
        unsupported => {
            log::warn!("unsupported updater proxy scheme: {unsupported}");
            None
        }
    }
}

fn redact_proxy_url(url: &Url) -> String {
    let mut redacted = url.clone();
    if !redacted.username().is_empty() {
        let _ = redacted.set_username("***");
    }
    if redacted.password().is_some() {
        let _ = redacted.set_password(Some("***"));
    }
    redacted.to_string()
}

#[cfg(windows)]
fn windows_proxy_from_system() -> Option<Url> {
    windows_proxy_from_registry().or_else(windows_proxy_from_winhttp)
}

#[cfg(not(windows))]
fn windows_proxy_from_system() -> Option<Url> {
    None
}

#[cfg(windows)]
fn windows_proxy_from_registry() -> Option<Url> {
    use std::process::Command;

    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyEnable",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.split_whitespace().any(|part| part == "0x1") {
        return None;
    }

    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyServer",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let proxy_server = registry_value_data(&stdout, "ProxyServer")?;
    parse_windows_proxy_server(&proxy_server)
}

#[cfg(windows)]
fn windows_proxy_from_winhttp() -> Option<Url> {
    use std::process::Command;

    let output = Command::new("netsh")
        .args(["winhttp", "show", "proxy"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some((_, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case("direct access (no proxy server)")
            || value.contains("直接访问")
        {
            continue;
        }
        if let Some(proxy) = parse_windows_proxy_server(value) {
            return Some(proxy);
        }
    }
    None
}

#[cfg(windows)]
fn registry_value_data(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with(name) {
            return None;
        }
        let data_start = trimmed.find("REG_SZ").map(|index| index + "REG_SZ".len())?;
        trimmed
            .get(data_start..)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

#[cfg(windows)]
fn parse_windows_proxy_server(value: &str) -> Option<Url> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let mut fallback = None;
    for part in value
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some((scheme, proxy)) = part.split_once('=') {
            let scheme = scheme.trim();
            let proxy = proxy.trim();
            if scheme.eq_ignore_ascii_case("https") {
                return parse_proxy_url(proxy);
            }
            if scheme.eq_ignore_ascii_case("http") && fallback.is_none() {
                fallback = parse_proxy_url(proxy);
            }
        } else if fallback.is_none() {
            fallback = parse_proxy_url(part);
        }
    }

    fallback
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
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| "无法确定当前安装目录".to_string())?;

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
    let install_dir = powershell_quote_path(install_dir);
    let installer_file = powershell_quote_path(&installer_path);
    let script_file = powershell_quote_path(&script_path);
    let pid = std::process::id();
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$pidToWait = {pid}
$target = {target}
$installDir = {install_dir}
$installer = {installer_file}
$log = "$installer.log"
try {{
  Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 1500
  $proc = Start-Process -FilePath $installer -ArgumentList "/S /D=$installDir" -PassThru -Wait
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
