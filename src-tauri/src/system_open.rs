//! 系统打开/剪贴板/路径校验相关 helper。

use std::path::{Path, PathBuf};
use url::Url;

pub(crate) fn canonical_existing_file(raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("文件路径不能为空".to_string());
    }

    let path = Path::new(trimmed);
    let canonical = path
        .canonicalize()
        .map_err(|_| "文件不存在或无法访问".to_string())?;

    if !canonical.is_file() {
        return Err("只能操作文件".to_string());
    }

    Ok(canonical)
}

pub(crate) fn canonical_existing_directory(raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("目录路径不能为空".to_string());
    }

    let path = Path::new(trimmed);
    let canonical = path
        .canonicalize()
        .map_err(|_| "目录不存在或无法访问".to_string())?;

    if !canonical.is_dir() {
        return Err("只能打开目录".to_string());
    }

    Ok(canonical)
}

pub(crate) fn open_file_with_system(target: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub(crate) fn open_directory_with_system(target: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    Command::new("explorer")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub(crate) fn reveal_file_with_system(target: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg("-R")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    {
        let mut select_arg = std::ffi::OsString::from("/select,");
        select_arg.push(target);
        Command::new("explorer")
            .arg(select_arg)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target.parent().unwrap_or(Path::new(".")))
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub(crate) fn open_external_url_with_system(url: &str) -> Result<(), String> {
    use std::process::Command;

    let parsed = Url::parse(url).map_err(|_| "链接格式不正确".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("仅支持打开 http/https 链接".to_string());
    }

    let target = parsed.as_str();

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub(crate) fn write_text_to_command(mut command: std::process::Command, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = command
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("系统剪贴板命令执行失败".to_string())
    }
}

pub(crate) fn write_text_to_clipboard(text: &str) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    {
        return write_text_to_command(Command::new("pbcopy"), text);
    }

    #[cfg(target_os = "windows")]
    {
        return write_text_to_command(Command::new("clip"), text);
    }

    #[cfg(target_os = "linux")]
    {
        let candidates: [(&str, &[&str]); 3] = [
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];

        for (program, args) in candidates {
            let mut command = Command::new(program);
            command.args(args);
            if write_text_to_command(command, text).is_ok() {
                return Ok(());
            }
        }

        return Err("当前系统缺少可用的剪贴板工具".to_string());
    }

    #[allow(unreachable_code)]
    Err("当前平台暂不支持系统剪贴板".to_string())
}
