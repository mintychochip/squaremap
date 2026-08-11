use super::cache;
use crate::output::{etag_for, validate_relative, OutputRoot};
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Response, StatusCode};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub(crate) fn decode_path(raw: &str) -> io::Result<PathBuf> {
    let path = raw.strip_prefix('/').unwrap_or(raw);
    if path.is_empty() { return Ok(PathBuf::new()); }
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "malformed percent encoding")); }
            let high = hex(bytes[index + 1])?;
            let low = hex(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    if decoded.iter().any(|byte| *byte == 0) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL path")); }
    let decoded = String::from_utf8(decoded).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
    if decoded.contains('\\') || (path.contains('%') && decoded.contains('/')) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "encoded separator"));
    }
    let relative = PathBuf::from(decoded);
    validate_relative(&relative)
}

fn hex(byte: u8) -> io::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "malformed percent encoding")),
    }
}

pub(crate) fn serve(root: &OutputRoot, path: &Path, method: &Method, request_headers: &HeaderMap) -> Response<Body> {
    let path = if path.as_os_str().is_empty() { Path::new("index.html") } else { path };
    let is_tile = is_tile(path);
    let mut headers = HeaderMap::new();
    if is_tile { headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=0, must-revalidate, no-cache")); }
    let opened = match root.open_file(path) {
        Ok(Some(opened)) => opened,
        Ok(None) => {
            if is_tile && path.extension().is_some_and(|extension| extension == "png") {
                headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
                headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
                return super::make_response(StatusCode::OK, headers, Body::empty());
            }
            return super::make_response(StatusCode::NOT_FOUND, headers, Body::empty());
        }
        Err(_) => return super::make_response(StatusCode::FORBIDDEN, headers, Body::empty()),
    };
    let (mut file, metadata) = opened;
    let etag = etag_for(&metadata);
    headers.insert(header::ETAG, HeaderValue::try_from(etag.as_str()).unwrap());
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_str(&metadata.len().to_string()).unwrap());
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(path)));
    if request_headers.get(header::IF_NONE_MATCH).and_then(|value| value.to_str().ok()).is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag)) {
        return super::make_response(StatusCode::NOT_MODIFIED, headers, Body::empty());
    }
    if method == Method::HEAD { return super::make_response(StatusCode::OK, headers, Body::from(vec![0; metadata.len() as usize])); }
    let mut bytes = Vec::with_capacity(metadata.len().min(8 * 1024 * 1024) as usize);
    if metadata.len() > 8 * 1024 * 1024 { return super::make_response(StatusCode::PAYLOAD_TOO_LARGE, headers, Body::empty()); }
    if file.read_to_end(&mut bytes).is_err() { return super::make_response(StatusCode::INTERNAL_SERVER_ERROR, headers, Body::empty()); }
    super::make_response(StatusCode::OK, headers, Body::from(bytes))
}
fn is_tile(path: &Path) -> bool { path.components().next().is_some_and(|component| component == std::path::Component::Normal("tiles".as_ref())) }
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript",
        _ => "application/octet-stream",
    }
}
