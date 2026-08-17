use reqwest::StatusCode;
use squaremap_render::{MAX_ENCODED_TILE_BYTES, TileStore};
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use std::net::{SocketAddr, TcpListener};
use tempfile::tempdir;

async fn server(root: &std::path::Path) -> HttpServer {
    let output = OutputRoot::new(root).unwrap();
    HttpServer::bind(HttpConfig::loopback(), output)
        .await
        .unwrap()
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

    let json = client
        .get(format!("{base}/tiles/settings.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        json.headers()[reqwest::header::CONTENT_TYPE],
        "application/json"
    );
    assert_eq!(
        json.headers()[reqwest::header::CACHE_CONTROL],
        "max-age=0, must-revalidate, no-cache"
    );
    let etag = json.headers()[reqwest::header::ETAG]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'));
    assert_eq!(json.content_length(), Some(11));
    let head_json = client
        .head(format!("{base}/tiles/settings.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(head_json.status(), StatusCode::OK);
    assert_eq!(head_json.headers()[reqwest::header::ETAG], etag);
    assert_eq!(
        head_json.headers()[reqwest::header::CACHE_CONTROL],
        "max-age=0, must-revalidate, no-cache"
    );
    assert!(head_json.bytes().await.unwrap().is_empty());

    let head = client
        .head(format!("{base}/tiles/existing.png"))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()[reqwest::header::CONTENT_LENGTH], "3");
    assert!(head.bytes().await.unwrap().is_empty());

    let not_modified = client
        .get(format!("{base}/tiles/settings.json"))
        .header(reqwest::header::IF_NONE_MATCH, etag.clone())
        .send()
        .await
        .unwrap();

    let wildcard = client
        .get(format!("{base}/tiles/settings.json"))
        .header(reqwest::header::IF_NONE_MATCH, "*")
        .send()
        .await
        .unwrap();
    assert_eq!(wildcard.status(), StatusCode::NOT_MODIFIED);
    let weak = client
        .get(format!("{base}/tiles/settings.json"))
        .header(reqwest::header::IF_NONE_MATCH, format!("W/{etag}"))
        .send()
        .await
        .unwrap();
    assert_eq!(weak.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
    assert!(not_modified.bytes().await.unwrap().is_empty());

    let missing_tile = client
        .get(format!("{base}/tiles/missing.png"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_tile.status(), StatusCode::OK);
    let tile_cache = missing_tile.headers()[reqwest::header::CACHE_CONTROL].to_owned();
    assert!(missing_tile.bytes().await.unwrap().is_empty());
    assert_eq!(tile_cache, "max-age=0, must-revalidate, no-cache");

    let missing = client
        .get(format!("{base}/missing.json"))
        .send()
        .await
        .unwrap();
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
    for path in [
        "/../secret",
        "/%2e%2e/secret",
        "/%252e%252e/secret",
        "/a%2fb",
        "/a\\b",
        "/a%00b",
    ] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert!(
            response.status().is_client_error() || response.status() == StatusCode::NOT_FOUND,
            "{path}: {}",
            response.status()
        );
    }
    let post = client.post(format!("{base}/")).send().await.unwrap();
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn reserved_temp_namespace_is_not_served_or_writable() {
    let dir = tempdir().unwrap();
    let output = OutputRoot::new(dir.path()).unwrap();
    std::fs::write(dir.path().join(".squaremap-tmp-123-456"), b"partial").unwrap();
    assert!(
        output
            .atomic_write(".squaremap-tmp-3-4", b"blocked")
            .is_err()
    );
    let mut server = HttpServer::bind(HttpConfig::loopback(), output)
        .await
        .unwrap();
    let response = reqwest::get(format!(
        "http://{}/.squaremap-tmp-123-456",
        server.local_addr().unwrap()
    ))
    .await
    .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
    server.shutdown().await.unwrap();
}
#[test]
fn atomic_writes_are_root_confined() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    root.atomic_write("nested/file.json", b"one").unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("nested/file.json")).unwrap(),
        b"one"
    );
    for path in [
        "../escape",
        "/tmp/escape",
        "nested/../escape",
        "nested\\escape",
        "nested/a\0b",
    ] {
        assert!(root.atomic_write(path, b"bad").is_err(), "{path:?}");
    }

    assert!(dir.path().join("nested").read_dir().unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with('.')
    }));
}

#[tokio::test]
async fn output_root_is_a_confined_bounded_tile_store() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let published = root
        .publish(std::path::Path::new("tiles/0/0_0.png"), b"png")
        .await
        .unwrap();
    assert!(published.warning.is_none());
    assert_eq!(
        root.read(std::path::Path::new("tiles/0/0_0.png"))
            .await
            .unwrap()
            .unwrap(),
        b"png"
    );
    assert!(
        root.publish(std::path::Path::new("../escape"), b"bad")
            .await
            .is_err()
    );
    assert!(root.read(std::path::Path::new("../escape")).await.is_err());
    assert!(
        root.publish(
            std::path::Path::new("tiles/oversized.png"),
            &vec![0; MAX_ENCODED_TILE_BYTES as usize + 1]
        )
        .await
        .is_err()
    );
}
#[test]
fn removes_stale_temp_siblings_recursively() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join(".squaremap-tmp-999999-456"), b"old").unwrap();
    std::fs::write(dir.path().join(".target.squaremap-not-a-temp"), b"keep").unwrap();
    std::fs::write(dir.path().join(".squaremap-tmp-999-456-extra"), b"keep").unwrap();
    std::fs::write(dir.path().join(".squaremap-tmp-x-1"), b"keep").unwrap();
    std::fs::create_dir_all(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("nested/.squaremap-tmp-999999-456"), b"old").unwrap();
    std::fs::write(dir.path().join("nested/.squaremap-old"), b"old").unwrap();
    std::fs::write(dir.path().join("keep"), b"keep").unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    assert!(!dir.path().join(".squaremap-tmp-999999-456").exists());
    assert!(dir.path().join(".squaremap-tmp-999-456-extra").exists());
    assert!(dir.path().join(".squaremap-tmp-x-1").exists());
    assert!(!dir.path().join("nested/.squaremap-tmp-999999-456").exists());
    assert!(dir.path().join(".target.squaremap-not-a-temp").exists());
    assert!(dir.path().join("keep").exists());
    root.atomic_write("nested/file", b"new").unwrap();
}

#[tokio::test]
async fn disabled_mode_does_not_bind_but_writes() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let config = HttpConfig {
        bind: SocketAddr::from(([127, 0, 0, 1], 0)),
        enabled: false,
        dev_frontend: None,
    };
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

#[cfg(unix)]
#[test]
fn rejects_symlink_root_and_owner_lock() {
    use std::os::unix::fs::symlink;
    let real = tempdir().unwrap();
    let parent = tempdir().unwrap();
    symlink(real.path(), parent.path().join("root-link")).unwrap();
    assert!(OutputRoot::new(parent.path().join("root-link")).is_err());
    let missing_parent = tempdir().unwrap();
    let missing_target = tempdir().unwrap();
    symlink(missing_target.path(), missing_parent.path().join("link")).unwrap();
    assert!(OutputRoot::new(missing_parent.path().join("link/new-root")).is_err());
    assert!(!missing_target.path().join("new-root").exists());

    let locked = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::write(outside.path().join("lock"), b"lock").unwrap();
    symlink(
        outside.path().join("lock"),
        locked.path().join(".squaremap-owner.lock"),
    )
    .unwrap();
    assert!(OutputRoot::new(locked.path()).is_err());
}

#[test]
fn output_root_has_single_cross_process_owner() {
    let dir = tempdir().unwrap();
    let first = OutputRoot::new(dir.path()).unwrap();
    assert!(OutputRoot::new(dir.path()).is_err());
    drop(first);
    assert!(OutputRoot::new(dir.path()).is_ok());
}

#[test]
fn concurrent_atomic_writes_never_expose_partial_or_unrelated_temp_files() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let first = root.clone();
    let second = root.clone();
    let left = std::thread::spawn(move || {
        for _ in 0..32 {
            first.atomic_write("nested/shared.bin", b"left").unwrap();
        }
    });
    let right = std::thread::spawn(move || {
        for _ in 0..32 {
            second.atomic_write("nested/shared.bin", b"right").unwrap();
        }
    });
    left.join().unwrap();
    right.join().unwrap();
    let bytes = std::fs::read(dir.path().join("nested/shared.bin")).unwrap();
    assert!(bytes == b"left" || bytes == b"right");
    assert!(
        std::fs::read_dir(dir.path().join("nested"))
            .unwrap()
            .all(|entry| {
                let name = entry.unwrap().file_name();
                !name.to_string_lossy().contains(".squaremap-")
            })
    );
}

#[cfg(windows)]
#[test]
fn windows_root_rejects_reparse_output_parent() {
    use std::os::windows::fs::symlink_dir;
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    if let Err(error) = symlink_dir(outside.path(), dir.path().join("link")) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("symlink setup failed: {error}");
    }
    let root = OutputRoot::new(dir.path()).unwrap();
    assert!(root.atomic_write("link/escape.txt", b"blocked").is_err());
    let root_link = dir.path().join("root-link");
    if let Err(error) = symlink_dir(outside.path(), &root_link) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("root symlink setup failed: {error}");
    }
    assert!(OutputRoot::new(&root_link).is_err());

    let locked = tempdir().unwrap();
    let lock_target = outside.path().join("lock-target");
    std::fs::write(&lock_target, b"lock").unwrap();
    if let Err(error) = std::os::windows::fs::symlink_file(
        &lock_target,
        locked.path().join(".squaremap-owner.lock"),
    ) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            return;
        }
        panic!("owner lock symlink setup failed: {error}");
    }
    assert!(OutputRoot::new(locked.path()).is_err());
    root.atomic_write("nested/replace.txt", b"one").unwrap();
    root.atomic_write("nested/replace.txt", b"two").unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("nested/replace.txt")).unwrap(),
        b"two"
    );
}
