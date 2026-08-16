pub mod config;
pub mod dirty_resync;
pub mod radius;

/// Returns whether a replacement config requests ownership of the Rust HTTP listener.
pub fn should_bind_http(global: Option<&squaremap_protocol::wire::GlobalSettings>) -> bool {
    global.is_some_and(|settings| settings.http_enabled)
}
pub mod control;
pub mod http;
pub mod metrics;
pub mod output;
pub mod scheduler;
pub mod session;
pub mod snapshot_client;
pub mod views;
