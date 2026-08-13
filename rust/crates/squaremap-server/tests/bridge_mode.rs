use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use tempfile::tempdir;

#[tokio::test]
async fn rust_http_binds_on_loopback_and_releases_port_on_shutdown() {
    let directory = tempdir().unwrap();
    let root = OutputRoot::new(directory.path()).unwrap();
    let mut server = HttpServer::bind(HttpConfig::loopback(), root).await.unwrap();
    let address = server.local_addr().unwrap();
    assert!(address.ip().is_loopback());
    server.shutdown().await.unwrap();
    assert!(std::net::TcpListener::bind(address).is_ok());
}

#[tokio::test]
async fn separate_output_roots_can_bind_independent_servers() {
    let java = tempdir().unwrap();
    let rust = tempdir().unwrap();
    let mut first = HttpServer::bind(HttpConfig::loopback(), OutputRoot::new(java.path()).unwrap()).await.unwrap();
    let mut second = HttpServer::bind(HttpConfig::loopback(), OutputRoot::new(rust.path()).unwrap()).await.unwrap();
    assert_ne!(first.local_addr(), second.local_addr());
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}
