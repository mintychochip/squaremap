use squaremap_server::http::{DevFrontendConfig, HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use std::time::Duration;
use tempfile::tempdir;
#[cfg(unix)]
static PROCESS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
fn executable(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-bun");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_non_loopback_and_early_exit() {
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), "printf '\\033[31mhttp://192.0.2.1:1\\033[0m\\n'");
    let config = HttpConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        dev_frontend: Some(DevFrontendConfig {
            frontend_dir: dir.path().to_owned(),
            executable: fake,
            startup_timeout: Duration::from_millis(250),
        }),
    };
    assert!(HttpServer::bind(config, root).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn readiness_timeout_reaps_fake_process() {
    let dir = tempdir().unwrap();
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), "sleep 10");
    let config = HttpConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        dev_frontend: Some(DevFrontendConfig {
            frontend_dir: dir.path().to_owned(),
            executable: fake,
            startup_timeout: Duration::from_millis(100),
        }),
    };
    assert!(HttpServer::bind(config, root).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn scans_multiple_candidates_and_delayed_readiness() {
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), "sleep 0.1; printf 'http://192.0.2.1:1 http://127.0.0.1:9\\n'; sleep 10");
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_immediate_exit_after_url_and_multibyte_logs() {
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), "printf 'http://127.0.0.1:9\\n'; exit 0");
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    assert!(HttpServer::bind(config, root).await.is_err());
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let long_line = "é".repeat(9000);
    let fake = executable(dir.path(), &format!("printf '{}\\nhttp://127.0.0.1:9\\n'; sleep 10", long_line));
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn proxies_http_and_keeps_excluded_paths_local() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while stream.read_exact(&mut byte).await.is_ok() {
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") { break; }
        }
        assert!(String::from_utf8_lossy(&request).contains("GET /api?q=1"));
        stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 7\r\nX-Upstream: yes\r\nConnection: close\r\n\r\nproxied").await.unwrap();
    });
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    std::fs::create_dir_all(dir.path().join("tiles")).unwrap();
    std::fs::write(dir.path().join("tiles/local.png"), b"local").unwrap();
    let fake = executable(dir.path(), &format!("printf 'http://{}\\n'; sleep 10", upstream_addr));
    let config = HttpConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        dev_frontend: Some(DevFrontendConfig {
            frontend_dir: dir.path().to_owned(),
            executable: fake,
            startup_timeout: Duration::from_secs(1),
        }),
    };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.local_addr().unwrap());
    let proxied = client.get(format!("{base}/api?q=1")).send().await.unwrap();
    assert_eq!(proxied.status(), 201);
    assert_eq!(proxied.headers()["x-upstream"], "yes");
    assert_eq!(proxied.text().await.unwrap(), "proxied");
    let local = client.get(format!("{base}/tiles/local.png")).send().await.unwrap();
    assert_eq!(local.status(), 200);
    assert_eq!(local.bytes().await.unwrap(), b"local".as_slice());
    server.shutdown().await.unwrap();
    upstream_task.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn proxies_non_get_body_and_query() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let count = stream.read(&mut buffer).await.unwrap();
            if count == 0 { break; }
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") && request.ends_with(b"payload") { break; }
        }
        let text = String::from_utf8_lossy(&request);
        assert!(text.starts_with("POST /submit?q=1"));
        assert!(text.contains("x-client: yes"));
        stream.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
    });
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &format!("printf 'http://{}\\n'; sleep 10", upstream_addr));
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let client = reqwest::Client::new();
    let response = client.post(format!("http://{}/submit?q=1", server.local_addr().unwrap())).header("x-client", "yes").body("payload").send().await.unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(response.text().await.unwrap(), "ok");
    server.shutdown().await.unwrap();
    upstream_task.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn tunnels_websocket_echo() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        let (stream, _) = upstream.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        if let Some(Ok(message)) = socket.next().await { socket.send(message).await.unwrap(); }
    });
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &format!("printf 'http://{}\\n'; sleep 10", upstream_addr));
    let config = HttpConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }),
    };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let ws_url = format!("ws://{}/hmr", server.local_addr().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    socket.send(Message::Text("echo".into())).await.unwrap();
    assert_eq!(socket.next().await.unwrap().unwrap(), Message::Text("echo".into()));
    let _ = socket.close(None).await;
    server.shutdown().await.unwrap();
    upstream_task.await.unwrap();
}

#[cfg(not(unix))]
#[test]
fn fixture_is_platform_conditional() {
    // Process fixture details are Unix-specific; production API remains portable.
}
