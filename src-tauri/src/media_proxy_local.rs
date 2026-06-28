use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::AppState;
use crate::media_proxy_security::allowed_request_origin;
use crate::media_proxy_headers::build_error_response;
use crate::media_proxy_cache::{
    parse_byte_range, LOCAL_MEDIA_INITIAL_RANGE_BYTES, LOCAL_MEDIA_MAX_RANGE_BYTES,
};

#[derive(Debug, Deserialize)]
pub(crate) struct LocalMediaQuery {
    pub(crate) path: String,
}

pub(crate) fn frontend_dist_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist")
}

pub(crate) fn local_media_content_type(path: &Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "avif" => Some("image/avif"),
        "heic" => Some("image/heic"),
        "heif" => Some("image/heif"),
        "mp4" | "m4v" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        "webm" => Some("video/webm"),
        "mkv" => Some("video/x-matroska"),
        "avi" => Some("video/x-msvideo"),
        "flv" => Some("video/x-flv"),
        "mp3" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        "aac" => Some("audio/aac"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "ogg" => Some("audio/ogg"),
        _ => None,
    }
}

pub(crate) fn local_media_kind(path: &Path) -> Option<&'static str> {
    let content_type = local_media_content_type(path)?;
    if content_type.starts_with("image/") {
        Some("image")
    } else if content_type.starts_with("video/") {
        Some("video")
    } else if content_type.starts_with("audio/") {
        Some("audio")
    } else {
        None
    }
}

pub(crate) async fn allowed_local_media_path(state: &AppState, raw_path: &str) -> Result<PathBuf, StatusCode> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let target = Path::new(trimmed)
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;

    if !target.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }

    if local_media_kind(&target).is_none() {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    let download_path = {
        let config = state.config.lock().await;
        config.download_path.clone()
    };

    if download_path.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let download_root = Path::new(download_path.trim())
        .canonicalize()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    if target.starts_with(download_root) {
        return Ok(target);
    }

    let history_paths = {
        let history = state.history.lock().await;
        history
            .get_all()
            .into_iter()
            .map(|item| item.file_path)
            .collect::<Vec<_>>()
    };

    let is_history_file = history_paths.iter().any(|path| {
        Path::new(path)
            .canonicalize()
            .map(|history_path| history_path == target)
            .unwrap_or(false)
    });

    if is_history_file {
        Ok(target)
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

pub(crate) async fn local_media(
    State(state): State<AppState>,
    Query(query): Query<LocalMediaQuery>,
    request_headers: HeaderMap,
) -> Response<Body> {
    let allow_origin = match allowed_request_origin(&request_headers) {
        Some(origin) => origin,
        None => return build_error_response(StatusCode::FORBIDDEN, "Forbidden"),
    };

    let path = match allowed_local_media_path(&state, &query.path).await {
        Ok(path) => path,
        Err(StatusCode::BAD_REQUEST) => {
            return build_error_response(StatusCode::BAD_REQUEST, "Invalid path")
        }
        Err(StatusCode::NOT_FOUND) => {
            return build_error_response(StatusCode::NOT_FOUND, "File not found")
        }
        Err(StatusCode::UNSUPPORTED_MEDIA_TYPE) => {
            return build_error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Unsupported media type",
            )
        }
        Err(status) => return build_error_response(status, "Forbidden"),
    };

    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return build_error_response(StatusCode::NOT_FOUND, "File not found"),
    };
    let file_size = metadata.len();
    let content_type = local_media_content_type(&path).unwrap_or("application/octet-stream");
    let media_kind = local_media_kind(&path).unwrap_or_default();
    let request_range = request_headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_byte_range(value, file_size));

    let should_seed_range = request_range.is_none()
        && file_size > LOCAL_MEDIA_INITIAL_RANGE_BYTES
        && media_kind != "image";

    let (status, start, end) = if let Some((start, end)) = request_range {
        (StatusCode::PARTIAL_CONTENT, start, end)
    } else if should_seed_range {
        (
            StatusCode::PARTIAL_CONTENT,
            0,
            (LOCAL_MEDIA_INITIAL_RANGE_BYTES - 1).min(file_size.saturating_sub(1)),
        )
    } else if file_size == 0 {
        (StatusCode::OK, 0, 0)
    } else {
        (StatusCode::OK, 0, file_size - 1)
    };

    let read_length = if file_size == 0 { 0 } else { end - start + 1 };
    let capped_length = if status == StatusCode::PARTIAL_CONTENT {
        read_length.min(LOCAL_MEDIA_MAX_RANGE_BYTES)
    } else {
        read_length
    };
    let capped_end = if capped_length == 0 {
        start
    } else {
        start + capped_length - 1
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return build_error_response(StatusCode::NOT_FOUND, "File not found"),
    };

    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error");
    }

    let mut buffer = vec![0u8; capped_length as usize];
    if capped_length > 0 && file.read_exact(&mut buffer).await.is_err() {
        return build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error");
    }

    let mut response_builder = Response::builder().status(status);
    let headers = match response_builder.headers_mut() {
        Some(headers) => headers,
        None => {
            return build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error")
        }
    };

    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        allow_origin.unwrap_or_else(|| HeaderValue::from_static("*")),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&capped_length.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );

    if status == StatusCode::PARTIAL_CONTENT {
        let content_range = format!("bytes {}-{}/{}", start, capped_end, file_size);
        if let Ok(value) = HeaderValue::from_str(&content_range) {
            headers.insert(header::CONTENT_RANGE, value);
        }
    }

    response_builder
        .body(Body::from(buffer))
        .unwrap_or_else(|_| {
            build_error_response(StatusCode::INTERNAL_SERVER_ERROR, "Local media error")
        })
}
