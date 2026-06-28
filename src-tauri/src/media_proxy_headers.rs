use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};

pub(crate) fn apply_cors_headers(response_headers: &mut HeaderMap, allow_origin: Option<HeaderValue>) {
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        allow_origin.unwrap_or_else(|| HeaderValue::from_static("*")),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Range, Content-Type, Accept, X-Douyin-Prewarm"),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Length, Content-Range, Accept-Ranges, Content-Type"),
    );
}

pub(crate) fn build_error_response(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::from(message.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from(message.to_string())))
}
