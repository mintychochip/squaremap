use axum::http::{header::HeaderName, HeaderMap};

pub(crate) fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(name.as_str(), "connection" | "keep-alive" | "proxy-authenticate" | "proxy-authorization" | "te" | "trailer" | "transfer-encoding" | "upgrade")
}

pub(crate) fn is_forwardable(name: &HeaderName, headers: &HeaderMap) -> bool {
    if is_hop_by_hop(name) || name == axum::http::header::HOST { return false; }
    !headers.get_all(axum::http::header::CONNECTION).iter().filter_map(|value| value.to_str().ok()).flat_map(|value| value.split(',')).any(|token| token.trim().eq_ignore_ascii_case(name.as_str()))
}
