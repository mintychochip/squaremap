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

#[cfg(unix)]
#[tokio::test]
async fn drains_sustained_logs_after_readiness() {
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempdir().unwrap();
    let marker = dir.path().join("drained");
    let script = format!("printf 'http://127.0.0.1:9\\n'; i=0; while [ $i -lt 20000 ]; do printf 'log-%s-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' \"$i\"; i=$((i+1)); done; printf done > '{}'; sleep 10", marker.display());
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &script);
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(2) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    for _ in 0..100 {
        if marker.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(marker.exists());
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_cancels_infinite_proxy_response() {
    use futures_util::StreamExt;
    use tokio::sync::oneshot;
    use tokio::io::AsyncWriteExt;
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.unwrap();
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n").await;
        loop {
            if stream.write_all(b"4\r\nspam\r\n").await.is_err() { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &format!("printf 'http://{}\\n'; sleep 10", upstream_addr));
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let (ready_tx, ready_rx) = oneshot::channel();
    let client_task = tokio::spawn({
        let url = format!("http://{}/stream", server.local_addr().unwrap());
        async move {
            let response = reqwest::get(url).await.unwrap();
            let _ = ready_tx.send(());
            let mut body = response.bytes_stream();
            while body.next().await.is_some() {}
        }
    });
    tokio::time::timeout(Duration::from_secs(1), ready_rx).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), server.shutdown()).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), client_task).await.unwrap().unwrap();
    let _ = upstream_task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_cancels_stalled_websocket_handshake() {
    use tokio::io::AsyncReadExt;
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let upstream_task = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.unwrap();
        let _ = accepted_tx.send(());
        let mut byte = [0_u8; 1];
        let _ = stream.read(&mut byte).await;
        let _ = release_rx.await;
    });
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &format!("printf 'http://{}\\n'; sleep 10", upstream_addr));
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let public = format!("ws://{}/hmr", server.local_addr().unwrap());
    let client_task = tokio::spawn(async move { tokio_tungstenite::connect_async(public).await });
    tokio::time::timeout(Duration::from_secs(1), accepted_rx).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), server.shutdown()).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), client_task).await.unwrap().unwrap();
    let _ = release_tx.send(());
    let _ = upstream_task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn websocket_forwards_headers_and_selected_protocol() {
    use tokio_tungstenite::tungstenite::handshake::server::{Request as WsRequest, Response as WsResponse};
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        let (stream, _) = upstream.accept().await.unwrap();
        let callback = |request: &WsRequest, mut response: WsResponse| {
            assert_eq!(request.headers().get("origin").unwrap(), "https://example.test");
            assert_eq!(request.headers().get("cookie").unwrap(), "sid=abc");
            assert_eq!(request.headers().get("authorization").unwrap(), "Bearer token");
            assert_eq!(request.headers().get("sec-websocket-protocol").unwrap(), "chat");
            response.headers_mut().insert("sec-websocket-protocol", "chat".parse().unwrap());
            Ok(response)
        };
        let _ = tokio_tungstenite::accept_hdr_async(stream, callback).await.unwrap();
    });
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &format!("printf 'http://{}\\n'; sleep 10", upstream_addr));
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(format!("ws://{}/hmr", server.local_addr().unwrap()))
        .header("Host", server.local_addr().unwrap().to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Origin", "https://example.test")
        .header("Cookie", "sid=abc")
        .header("Authorization", "Bearer token")
        .header("Sec-WebSocket-Protocol", "chat")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
.body(())
.unwrap();
    let (_, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.headers().get("sec-websocket-protocol").unwrap(), "chat");
    server.shutdown().await.unwrap();
    upstream_task.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_kills_descendant_process_group() {
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempdir().unwrap();
    let pid_file = dir.path().join("descendant.pid");
    let script = format!("sleep 30 & child=$!; echo $child > '{}'; printf 'http://127.0.0.1:9\\n'; wait", pid_file.display());
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &script);
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    let mut server = HttpServer::bind(config, root).await.unwrap();
    let pid = loop {
        if let Ok(value) = std::fs::read_to_string(&pid_file) {
            break value.trim().parse::<u32>().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    server.shutdown().await.unwrap();
    for _ in 0..100 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() { return; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("descendant process survived shutdown");
}

#[cfg(unix)]
#[tokio::test]
async fn startup_failure_kills_saved_process_group_descendant() {
    let _guard = PROCESS_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempdir().unwrap();
    let pid_file = dir.path().join("failed-descendant.pid");
    let script = format!("sleep 30 & child=$!; echo $child > '{}'; printf 'http://127.0.0.1:9\\n'; exit 0", pid_file.display());
    let root = OutputRoot::new(dir.path()).unwrap();
    let fake = executable(dir.path(), &script);
    let config = HttpConfig { bind: "127.0.0.1:0".parse().unwrap(), enabled: true, dev_frontend: Some(DevFrontendConfig { frontend_dir: dir.path().to_owned(), executable: fake, startup_timeout: Duration::from_secs(1) }) };
    assert!(HttpServer::bind(config, root).await.is_err());
    let pid = loop {
        if let Ok(value) = std::fs::read_to_string(&pid_file) {
            break value.trim().parse::<u32>().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    for _ in 0..100 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() { return; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("startup descendant survived failure cleanup");
}

#[cfg(windows)]
#[test]
fn windows_output_root_writes_nested_without_reparse_following() {
    let dir = tempdir().unwrap();
    let root = OutputRoot::new(dir.path()).unwrap();
    root.atomic_write("nested/file.txt", b"ok").unwrap();
    assert_eq!(std::fs::read(dir.path().join("nested/file.txt")).unwrap(), b"ok");
}
