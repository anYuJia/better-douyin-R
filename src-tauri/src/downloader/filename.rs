//! 文件名模板和路径

use crate::api::types::MediaType;
use crate::config::AppConfig;
use anyhow::{anyhow, Result};
use chrono::{Local, TimeZone};
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use url::Url;

pub(crate) const MAX_FILENAME_CHARS: usize = 180;
pub(crate) const MAX_FILENAME_BYTES: usize = 230;

pub(crate) fn template_value(
    token: &str,
    title: &str,
    aweme_id: &str,
    author: &str,
    media_type: &str,
    template_time: Option<&chrono::DateTime<Local>>,
) -> String {
    match token {
        "title" => title.to_string(),
        "aweme_id" => aweme_id.to_string(),
        "author" => author.to_string(),
        "date" => template_time
            .map(|time| time.format("%Y%m%d").to_string())
            .unwrap_or_default(),
        "time" => template_time
            .map(|time| time.format("%Y%m%d_%H%M%S").to_string())
            .unwrap_or_default(),
        "media_type" => media_type.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn template_datetime(create_time: i64) -> Option<chrono::DateTime<Local>> {
    let seconds = if create_time > 1_000_000_000_000 {
        create_time / 1000
    } else {
        create_time
    };

    if seconds > 0 {
        if let Some(datetime) = Local.timestamp_opt(seconds, 0).single() {
            return Some(datetime);
        }
    }

    None
}

pub(crate) fn render_template(
    template: &str,
    title: &str,
    aweme_id: &str,
    author: &str,
    media_type: &str,
    create_time: i64,
) -> String {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    let template_time = template_datetime(create_time);

    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }

        let mut token = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == '}' {
                closed = true;
                break;
            }
            token.push(next);
        }

        if closed {
            output.push_str(&template_value(
                &token,
                title,
                aweme_id,
                author,
                media_type,
                template_time.as_ref(),
            ));
        } else {
            output.push('{');
            output.push_str(&token);
        }
    }

    output
}
pub(crate) fn truncate_filename_text(
    value: &str,
    default: &str,
    max_chars: usize,
    max_bytes: usize,
    protected_suffix: &str,
) -> String {
    let mut text = value.trim_matches([' ', '.', '_']).to_string();
    if text.is_empty() {
        text = default.to_string();
    }

    if !protected_suffix.is_empty() && text.ends_with(protected_suffix) {
        let prefix_len = text.len().saturating_sub(protected_suffix.len());
        let mut prefix: String = text[..prefix_len]
            .chars()
            .take(max_chars.saturating_sub(protected_suffix.chars().count()))
            .collect();
        while !prefix.is_empty() && format!("{}{}", prefix, protected_suffix).len() > max_bytes {
            prefix.pop();
        }
        text = format!("{}{}", prefix, protected_suffix);
    } else {
        text = text.chars().take(max_chars).collect();
        while !text.is_empty() && text.len() > max_bytes {
            text.pop();
        }
    }

    let text = text.trim_matches([' ', '.', '_']).to_string();
    if text.is_empty() {
        default.to_string()
    } else {
        text
    }
}

pub(crate) fn media_type_name(media_type: &MediaType) -> &'static str {
    match media_type {
        MediaType::Video => "video",
        MediaType::Image => "image",
        MediaType::LivePhoto => "live_photo",
        MediaType::Mixed => "mixed",
        MediaType::Audio => "audio",
    }
}

pub(crate) fn media_type_display(media_type: &str) -> &'static str {
    match media_type {
        "video" => "视频",
        "image" => "图片",
        "live_photo" => "Live Photo",
        "audio" => "音频",
        _ => "媒体",
    }
}

pub(crate) fn media_download_success_action(media_type: &str) -> &'static str {
    match media_type {
        "video" => "下载视频",
        "image" => "下载图片",
        "live_photo" => "下载Live Photo",
        "audio" => "下载音频",
        _ => "下载媒体",
    }
}

pub(crate) fn media_extension(media_type: &str, url: &str, content_type: Option<&str>) -> String {
    let normalized_content_type = content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let from_content_type = match normalized_content_type.as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        "image/heic" => Some("heic"),
        "image/heif" => Some("heif"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" => Some("m4a"),
        "audio/aac" => Some("aac"),
        "audio/wav" => Some("wav"),
        "audio/ogg" => Some("ogg"),
        _ => None,
    };
    if let Some(extension) = from_content_type {
        return extension.to_string();
    }

    if let Some(extension) = extension_from_url(url) {
        return extension.to_string();
    }

    match media_type {
        "video" | "live_photo" => "mp4".to_string(),
        "audio" => "mp3".to_string(),
        _ => "jpg".to_string(),
    }
}

pub(crate) fn extension_from_url(url: &str) -> Option<&'static str> {
    let path = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.path_segments()?.next_back().map(str::to_string))
        .or_else(|| {
            url.split('?')
                .next()
                .and_then(|path| path.rsplit('/').next())
                .map(str::to_string)
        })?;
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();

    match extension.as_str() {
        "mp4" => Some("mp4"),
        "mov" => Some("mov"),
        "m4v" => Some("m4v"),
        "webm" => Some("webm"),
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "avif" => Some("avif"),
        "heic" => Some("heic"),
        "heif" => Some("heif"),
        "mp3" => Some("mp3"),
        "m4a" => Some("m4a"),
        "aac" => Some("aac"),
        "wav" => Some("wav"),
        "ogg" => Some("ogg"),
        _ => None,
    }
}

pub(crate) fn unique_output_path(
    save_dir: &Path,
    base_name: &str,
    index: usize,
    total: usize,
    extension: &str,
) -> PathBuf {
    let stem = if total <= 1 {
        sanitize_filename(base_name)
    } else {
        sanitize_filename(&format!("{}_{:02}", base_name, index + 1))
    };
    let extension = sanitize_extension(extension);
    save_dir.join(format!("{}.{}", stem, extension))
}

pub(crate) async fn create_unique_output_file(
    save_dir: &Path,
    base_name: &str,
    index: usize,
    total: usize,
    extension: &str,
) -> Result<(PathBuf, File)> {
    let extension = sanitize_extension(extension);
    for attempt in 0..1000 {
        let candidate = if attempt == 0 {
            unique_output_path(save_dir, base_name, index, total, &extension)
        } else {
            let stem = if total <= 1 {
                sanitize_filename(base_name)
            } else {
                sanitize_filename(&format!("{}_{:02}", base_name, index + 1))
            };
            save_dir.join(format!("{}_{}.{}", stem, attempt + 1, extension))
        };

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow!("Unable to reserve unique output file name"))
}

pub(crate) fn sanitize_extension(extension: &str) -> String {
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        && !extension.is_empty()
        && extension.len() <= 8
    {
        extension
    } else {
        "bin".to_string()
    }
}

/// 清理文件名
pub(crate) fn sanitize_filename(name: &str) -> String {
    let invalid_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let mut result = name.to_string();
    for c in invalid_chars {
        result = result.replace(c, "_");
    }
    truncate_filename_text(
        &result,
        "未命名作品",
        MAX_FILENAME_CHARS,
        MAX_FILENAME_BYTES,
        "",
    )
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn generate_filename_with_config(
    config: &AppConfig,
    desc: &str,
    aweme_id: &str,
    author: &str,
    media_type: &str,
    create_time: i64,
) -> String {
    let normalized_title = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized_aweme_id = aweme_id.trim();
    let default = "未命名作品".to_string();
    let template = if config.filename_template.trim().is_empty() {
        "{title}"
    } else {
        config.filename_template.trim()
    };
    let rendered = render_template(
        template,
        if normalized_title.is_empty() {
            "未命名作品"
        } else {
            &normalized_title
        },
        normalized_aweme_id,
        author.trim(),
        media_type,
        create_time,
    );
    let sanitized = sanitize_filename(&rendered);
    let protected_suffix = if template.contains("{aweme_id}") && !normalized_aweme_id.is_empty() {
        if sanitized.ends_with(normalized_aweme_id) {
            normalized_aweme_id.to_string()
        } else {
            format!("_{}", normalized_aweme_id)
        }
    } else {
        String::new()
    };
    let candidate = if !protected_suffix.is_empty() && !sanitized.ends_with(&protected_suffix) {
        format!("{}{}", sanitized, protected_suffix)
    } else {
        sanitized
    };

    truncate_filename_text(
        &candidate,
        &default,
        MAX_FILENAME_CHARS,
        MAX_FILENAME_BYTES,
        &protected_suffix,
    )
}

pub(crate) fn build_output_dir(
    config: &AppConfig,
    base_path: &Path,
    author: &str,
    media_type: &str,
    create_time: i64,
) -> PathBuf {
    if !config.auto_create_folder {
        return base_path.to_path_buf();
    }

    let template = if config.folder_name_template.trim().is_empty() {
        "{author}"
    } else {
        config.folder_name_template.trim()
    };
    let rendered = render_template(template, "", "", author.trim(), media_type, create_time);
    let folder = sanitize_filename(&rendered);
    if folder.is_empty() {
        base_path.join("未知作者")
    } else {
        base_path.join(folder)
    }
}
