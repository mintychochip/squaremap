use crate::output::{OutputRoot, etag_for, validate_relative};
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, Response, StatusCode, header};
use std::io;
use std::path::{Path, PathBuf};
use tokio_util::io::ReaderStream;

pub(crate) fn decode_path(raw: &str) -> io::Result<PathBuf> {
    let path = raw.strip_prefix('/').unwrap_or(raw);
    if path.is_empty() {
        return Ok(PathBuf::new());
    }
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "malformed percent encoding",
                ));
            }
            let high = hex(bytes[index + 1])?;
            let low = hex(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    if decoded.iter().any(|byte| *byte == 0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL path"));
    }
    let decoded = String::from_utf8(decoded)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
    if decoded.contains('\\') || (path.contains('%') && decoded.contains('/')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "encoded separator",
        ));
    }
    let relative = PathBuf::from(decoded);
    validate_relative(&relative)
}

fn hex(byte: u8) -> io::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "malformed percent encoding",
        )),
    }
}

pub(crate) fn serve(
    root: &OutputRoot,
    path: &Path,
    method: &Method,
    request_headers: &HeaderMap,
) -> Response<Body> {
    let path = if path.as_os_str().is_empty() {
        Path::new("index.html")
    } else {
        path
    };
    let is_tile = is_tile(path);
    let mut headers = HeaderMap::new();
    if is_tile {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=0, must-revalidate, no-cache"),
        );
    }
    let opened = match root.open_file(path) {
        Ok(Some(opened)) => opened,
        Ok(None) => {
            if is_tile && path.extension().is_some_and(|extension| extension == "png") {
                headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
                return super::make_response(StatusCode::OK, headers, Body::empty());
            }
            return super::make_response(StatusCode::NOT_FOUND, headers, Body::empty());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return super::make_response(StatusCode::NOT_FOUND, headers, Body::empty());
        }
        Err(_) => return super::make_response(StatusCode::FORBIDDEN, headers, Body::empty()),
    };
    let (file, metadata) = opened;
    let etag = etag_for(&metadata);
    headers.insert(header::ETAG, HeaderValue::try_from(etag.as_str()).unwrap());
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&metadata.len().to_string()).unwrap(),
    );
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(path)),
    );
    if request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.trim() == "*"
                || value.split(',').any(|candidate| {
                    let candidate = candidate.trim();
                    candidate == etag
                        || candidate
                            .strip_prefix("W/")
                            .is_some_and(|weak| weak == etag)
                })
        })
    {
        return super::make_response(StatusCode::NOT_MODIFIED, headers, Body::empty());
    }
    if method == Method::HEAD {
        return super::make_response(StatusCode::OK, headers, Body::empty());
    }
    let stream = ReaderStream::new(tokio::fs::File::from_std(file));
    super::make_response(StatusCode::OK, headers, Body::from_stream(stream))
}
fn is_tile(path: &Path) -> bool {
    path.components()
        .next()
        .is_some_and(|component| component == std::path::Component::Normal("tiles".as_ref()))
}
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "application/javascript",
        Some("ico") => "image/x-icon",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("map") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
