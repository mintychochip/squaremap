use reqwest::StatusCode;
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use std::net::{SocketAddr, TcpListener};
use tempfile::tempdir;

async fn server(root: &std::path::Path) -> HttpServer {
    let output = OutputRoot::new(root).unwrap();
    HttpServer::bind(HttpConfig::loopback(), output).await.unwrap()
}

#[tokio::test]
async fn serves_files_headers_and_missing_tile_contract() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"index").unwrap();
    std::fs::create_dir_all(dir.path().join("tiles")).unwrap();
    std::fs::write(dir.path().join("tiles/settings.json"), br#"{"ok":true}"#).unwrap();
    std::fs::write(dir.path().join("tiles/existing.png"), b"png").unwrap();
    let mut server = server(dir.path()).await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.local_addr().unwrap());

    let index = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(index.status(), StatusCode::OK);
    assert_eq!(index.text().await.unwrap(), "index");

    let json = client.get(format!("{base}/tiles/settings.json")).send().await.unwrap();
    assert_eq!(json.headers()[reqwest::header::CONTENT_TYPE], "application/json");
    assert_eq!(json.headers()[reqwest::header::CACHE_CONTROL], "max-age=0, must-revalidate, no-cache");
    let etag = json.headers()[reqwest::header::ETAG].to_str().unwrap().to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'));
    assert_eq!(json.content_length(), Some(11));

    let head = client.head(format!("{base}/tiles/existing.png")).send().await.unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()[reqwest::header::CONTENT_LENGTH], "3");
    assert!(head.bytes().await.unwrap().is_empty());

    let not_modified = client.get(format!("{base}/tiles/settings.json")).header(reqwest::header::IF_NONE_MATCH, etag).send().await.unwrap();
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
    assert!(not_modified.bytes().await.unwrap().is_empty());

    let missing_tile = client.get(format!("{base}/tiles/missing.png")).send().await.unwrap();
    assert_eq!(missing_tile.status(), StatusCode::OK);
    let tile_cache = missing_tile.headers()[reqwest::header::CACHE_CONTROL].to_owned();
    assert!(missing_tile.bytes().await.unwrap().is_empty());
    assert_eq!(tile_cache, "max-age=0, must-revalidate, no-cache");

    let missing = client.get(format!("{base}/missing.json")).send().await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejects_methods_and_confined_paths() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"index").unwrap();
    let mut server = server(dir.path()).await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.local_addr().unwrap());
    for path in ["/../secret", "/%2e%2e/secret", "/%252e%252e/secret", "/a%2fb", "/a\\b", "/a%00b"] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert!(response.status().is_client_error() || response.status() == StatusCode::NOT_FOUND, "{path}: {}", response.status());
    }
    let post = client.post(format!("{base}/")).send().await.unwrap();
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
    server.shutdown().await.unwrap();
}

#[test]
fn atomic_writes_are_root_confined() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    root.atomic_write("nested/file.json", b"one").unwrap();
    assert_eq!(std::fs::read(dir.path().join("nested/file.json")).unwrap(), b"one");
    for path in ["../escape", "/tmp/escape", "nested/../escape", "nested\\escape", "nested/a\0b"] {
        assert!(root.atomic_write(path, b"bad").is_err(), "{path:?}");
    }
    assert!(dir.path().join("nested").read_dir().unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with('.')));
}

#[tokio::test]
async fn disabled_mode_does_not_bind_but_writes() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let config = HttpConfig { bind: SocketAddr::from(([127, 0, 0, 1], 0)), enabled: false, dev_frontend: None };
    let mut server = HttpServer::bind(config, root.clone()).await.unwrap();
    assert!(server.local_addr().is_none());
    root.atomic_write("index.html", b"still works").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    drop(listener);
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlink_escapes() {
    use std::os::unix::fs::symlink;
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    symlink(outside.path(), dir.path().join("link")).unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    assert!(root.atomic_write("link/escape", b"x").is_err());
}
