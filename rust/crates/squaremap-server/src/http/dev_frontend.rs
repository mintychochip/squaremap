use super::{make_response, uri_path_with_query};
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::{handshake::derive_accept_key, protocol::Role}};

#[derive(Clone, Debug)]
pub struct DevFrontendConfig {
    pub frontend_dir: PathBuf,
    pub executable: PathBuf,
    pub startup_timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct DevFrontend {
    inner: Arc<Inner>,
}

struct Inner {
    child: Mutex<Child>,
    upstream: String,
    tunnels: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl DevFrontend {
    pub(crate) async fn start(config: DevFrontendConfig) -> std::io::Result<Self> {
        let mut command = Command::new(&config.executable);
        command.args(["run", "dev"]).current_dir(&config.frontend_dir).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("frontend stdout unavailable"))?;
        let stderr = child.stderr.take().ok_or_else(|| std::io::Error::other("frontend stderr unavailable"))?;
        let (sender, mut receiver) = mpsc::channel::<String>(32);
        let stdout_sender = sender.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = bound_line(line);
                if stdout_sender.send(line).await.is_err() { break; }
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = bound_line(line);
                if sender.send(line).await.is_err() { break; }
            }
        });
        let deadline = Instant::now() + config.startup_timeout;
        let upstream = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                terminate(&mut child).await;
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "frontend URL readiness timeout"));
            }
            match timeout(remaining.min(Duration::from_millis(50)), receiver.recv()).await {
                Ok(Some(line)) => {
                    if let Some(url) = find_loopback_url(&line) { break url; }
                }
                Ok(None) => {
                    terminate(&mut child).await;
                    return Err(std::io::Error::other("frontend exited before readiness"));
                }
                Err(_) => {
                    if Instant::now() >= deadline {
                        terminate(&mut child).await;
                        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "frontend URL readiness timeout"));
                    }
                }
            }
            if let Ok(Some(_)) = child.try_wait() {
                terminate(&mut child).await;
                return Err(std::io::Error::other("frontend exited before readiness"));
            }
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        if let Ok(Some(_)) = child.try_wait() {
            return Err(std::io::Error::other("frontend exited immediately after readiness"));
        }
        let inner = Arc::new(Inner { child: Mutex::new(child), upstream, tunnels: Mutex::new(Vec::new()) });
        Ok(Self { inner })
    }

    pub(crate) async fn proxy(&self, request: Request<Body>) -> std::io::Result<axum::response::Response<Body>> {
        let websocket = request.headers().get(header::UPGRADE).and_then(|value| value.to_str().ok()).is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
        if websocket { return self.proxy_websocket(request).await; }
        let (parts, body) = request.into_parts();
        let body = body.into_data_stream().map(|result| result.map_err(std::io::Error::other));
        let target = format!("{}{}", self.inner.upstream, uri_path_with_query(&parts.uri));
        let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
        let client = Client::new();
        let mut builder = client.request(method, target).body(reqwest::Body::wrap_stream(body));
        for (name, value) in &parts.headers {
            if super::cache::is_forwardable(name, &parts.headers) { builder = builder.header(name, value); }
        }
        let response = builder.send().await.map_err(std::io::Error::other)?;
        let status = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut headers = HeaderMap::new();
        for (name, value) in response.headers() {
            if super::cache::is_forwardable(name, response.headers()) {
                if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) { headers.append(name, value); }
            }
        }
        let body = response.bytes_stream().map(|result| result.map_err(std::io::Error::other));
        Ok(make_response(status, headers, Body::from_stream(body)))
    }

    async fn proxy_websocket(&self, request: Request<Body>) -> std::io::Result<axum::response::Response<Body>> {
        let (parts, _) = request.into_parts();
        let key = parts.headers.get("sec-websocket-key").and_then(|value| value.to_str().ok()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing websocket key"))?.to_owned();
        let target = format!("{}{}", self.inner.upstream.replace("http://", "ws://").replace("https://", "wss://"), uri_path_with_query(&parts.uri));
        let mut builder = tokio_tungstenite::tungstenite::http::Request::builder().uri(&target).header("Connection", "Upgrade").header("Upgrade", "websocket").header("Sec-WebSocket-Version", "13").header("Sec-WebSocket-Key", key.clone());
        if let Ok(url) = reqwest::Url::parse(&target) {
            if let Some(host) = url.host_str() {
                let host = format!("{}{}", host, url.port().map(|port| format!(":{port}")).unwrap_or_default());
                builder = builder.header("Host", host);
            }
        }
        for (name, value) in &parts.headers {
            if name.as_str() != "sec-websocket-key" && super::cache::is_forwardable(name, &parts.headers) { builder = builder.header(name, value); }
        }
        let custom = parts.headers.contains_key("cookie") || parts.headers.contains_key("authorization") || parts.headers.contains_key("sec-websocket-protocol");
        let (upstream, upstream_response) = if custom {
            let upstream_request = builder.body(()).map_err(std::io::Error::other)?;
            connect_async(upstream_request).await.map_err(std::io::Error::other)?
        } else {
            connect_async(target).await.map_err(std::io::Error::other)?
        };
        let upgrade = hyper::upgrade::on(Request::from_parts(parts, Body::empty()));
        let accept = derive_accept_key(key.as_bytes());
        let selected_protocol = upstream_response.headers().get("sec-websocket-protocol").cloned();
        let handle = tokio::spawn(async move {
            let Ok(upgraded) = upgrade.await else { return; };
            let client = tokio_tungstenite::WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
            let (mut upstream_sink, mut upstream_stream) = upstream.split();
            let (mut client_sink, mut client_stream) = client.split();
            loop {
                tokio::select! {
                    message = client_stream.next() => {
                        match message {
                            Some(Ok(message)) => if upstream_sink.send(message).await.is_err() { break; },
                            _ => break,
                        }
                    }
                    message = upstream_stream.next() => {
                        match message {
                            Some(Ok(message)) => if client_sink.send(message).await.is_err() { break; },
                            _ => break,
                        }
                    }
                }
            }
        });
        let mut tunnels = self.inner.tunnels.lock().await;
        tunnels.retain(|handle| !handle.is_finished());
        tunnels.push(handle);
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        headers.insert("sec-websocket-accept", HeaderValue::from_str(&accept).unwrap());
        if let Some(protocol) = selected_protocol { headers.insert("sec-websocket-protocol", HeaderValue::from_bytes(protocol.as_bytes()).unwrap()); }
        Ok(make_response(StatusCode::SWITCHING_PROTOCOLS, headers, Body::empty()))
    }
    pub(crate) async fn shutdown(self) {
        let mut tunnels = self.inner.tunnels.lock().await;
        for handle in tunnels.drain(..) { handle.abort(); let _ = handle.await; }
        drop(tunnels);
        let mut child = self.inner.child.lock().await;
        terminate(&mut child).await;
    }
}
fn find_loopback_url(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    for token in clean.split_whitespace() {
        let candidate = token.trim_matches(|character: char| ",;()[]{}".contains(character)).trim_end_matches('/');
        let parsed = match reqwest::Url::parse(candidate) { Ok(parsed) => parsed, Err(_) => continue };
        if !matches!(parsed.scheme(), "http" | "https") || parsed.port().is_none() { continue; }
        let host = match parsed.host_str() { Some(host) => host, None => continue };
        let loopback = host.parse::<IpAddr>().map(|ip| ip.is_loopback()).unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
        if loopback { return Some(candidate.to_owned()); }
    }
    None
}

fn bound_line(line: String) -> String {
    const LIMIT: usize = 16 * 1024;
    if line.len() <= LIMIT { return line; }
    let mut end = LIMIT;
    while end > 0 && !line.is_char_boundary(end) { end -= 1; }
    line[..end].to_owned()
}

fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.next() == Some('[') {
                for character in chars.by_ref() { if ('@'..='~').contains(&character) { break; } }
            }
        } else { result.push(character); }
    }
    result
}

async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() { unsafe { libc::kill(-(pid as i32), libc::SIGTERM); } }
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    #[cfg(unix)]
    if let Some(pid) = child.id() { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
    if child.id().is_some() { let _ = child.kill().await; let _ = child.wait().await; }
}
