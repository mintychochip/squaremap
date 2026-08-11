use super::{make_response, uri_path_with_query};
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
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
}

impl DevFrontend {
    pub(crate) async fn start(config: DevFrontendConfig) -> std::io::Result<Self> {
        let mut command = Command::new(&config.executable);
        command.args(["run", "dev"]).current_dir(&config.frontend_dir).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
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
                let line = if line.len() > 16 * 1024 { line[..16 * 1024].to_owned() } else { line };
                if stdout_sender.send(line).await.is_err() { break; }
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = if line.len() > 16 * 1024 { line[..16 * 1024].to_owned() } else { line };
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
                    terminate(&mut child).await;
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "frontend URL readiness timeout"));
                }
            }
            if let Ok(Some(_)) = child.try_wait() {
                terminate(&mut child).await;
                return Err(std::io::Error::other("frontend exited before readiness"));
            }
        };
        let inner = Arc::new(Inner { child: Mutex::new(child), upstream });
        Ok(Self { inner })
    }

    pub(crate) async fn proxy(&self, request: Request<Body>) -> std::io::Result<axum::response::Response<Body>> {
        let websocket = request.headers().get(header::UPGRADE).and_then(|value| value.to_str().ok()).is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
        if websocket { return self.proxy_websocket(request).await; }
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, 16 * 1024 * 1024).await.map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let target = format!("{}{}", self.inner.upstream, uri_path_with_query(&parts.uri));
        let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
        let client = Client::new();
        let mut builder = client.request(method, target).body(body.to_vec());
        for (name, value) in &parts.headers {
            if !super::cache::is_hop_by_hop(name) { builder = builder.header(name, value); }
        }
        let response = builder.send().await.map_err(std::io::Error::other)?;
        let status = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut headers = HeaderMap::new();
        for (name, value) in response.headers() {
            if !super::cache::is_hop_by_hop(name) {
                if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) { headers.append(name, value); }
            }
        }
        let body = response.bytes().await.map_err(std::io::Error::other)?.to_vec();
        Ok(make_response(status, headers, Body::from(body)))
    }

    async fn proxy_websocket(&self, request: Request<Body>) -> std::io::Result<axum::response::Response<Body>> {
        let (parts, _) = request.into_parts();
        let key = parts.headers.get("sec-websocket-key").and_then(|value| value.to_str().ok()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing websocket key"))?.to_owned();
        let target = format!("{}{}", self.inner.upstream.replace("http://", "ws://").replace("https://", "wss://"), uri_path_with_query(&parts.uri));
        let upgrade = hyper::upgrade::on(Request::from_parts(parts, Body::empty()));
        let accept = derive_accept_key(key.as_bytes());
        tokio::spawn(async move {
            let Ok(upgraded) = upgrade.await else { return; };
            let Ok((upstream, _)) = connect_async(target).await else { return; };
            let mut client = tokio_tungstenite::WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
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
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        headers.insert("sec-websocket-accept", HeaderValue::from_str(&accept).unwrap());
        Ok(make_response(StatusCode::SWITCHING_PROTOCOLS, headers, Body::empty()))
    }
    pub(crate) async fn shutdown(self) {
        let mut child = self.inner.child.lock().await;
        terminate(&mut child).await;
    }
}


fn find_loopback_url(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    for scheme in ["http://", "https://"] {
        let Some(start) = clean.find(scheme) else { continue };
        let rest = &clean[start..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let candidate = rest[..end].trim_end_matches('/');
        let parsed = reqwest::Url::parse(candidate).ok()?;
        let host = parsed.host_str()?;
        let loopback = host.parse::<IpAddr>().map(|ip| ip.is_loopback()).unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
        if loopback && parsed.port().is_some() { return Some(candidate.to_owned()); }
    }
    None
}

fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == 0x1b {
            if bytes.next() == Some(b'[') {
                for byte in bytes.by_ref() { if (b'@'..=b'~').contains(&byte) { break; } }
            }
        } else { result.push(byte as char); }
    }
    result
}

async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() { unsafe { libc::kill(-(pid as i32), libc::SIGTERM); } }
    }
    let _ = child.start_kill();
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    if child.id().is_some() { let _ = child.kill().await; }
}
