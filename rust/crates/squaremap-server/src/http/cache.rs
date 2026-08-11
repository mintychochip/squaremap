use axum::http::header::HeaderName;

pub(crate) fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(name.as_str(), "connection" | "keep-alive" | "proxy-authenticate" | "proxy-authorization" | "te" | "trailer" | "transfer-encoding" | "upgrade")
}
