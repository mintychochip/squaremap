use super::{make_response, uri_path_with_query};
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{handshake::derive_accept_key, protocol::Role},
};
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtResumeProcess(process: windows_sys::Win32::Foundation::HANDLE) -> i32;
}

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

#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for JobHandle {}
#[cfg(windows)]
unsafe impl Sync for JobHandle {}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
struct Inner {
    child: Mutex<Child>,
    upstream: String,
    tunnels: Mutex<Vec<JoinHandle<()>>>,
    log_tasks: Mutex<Vec<JoinHandle<()>>>,
    cancel: CancellationToken,
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: JobHandle,
}

impl DevFrontend {
    pub(crate) async fn start(config: DevFrontendConfig) -> std::io::Result<Self> {
        let mut command = Command::new(&config.executable);
        command
            .args(["run", "dev"])
            .current_dir(&config.frontend_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        let job = create_job()?;
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        #[cfg(unix)]
        let pgid = child
            .id()
            .map(|id| id as i32)
            .ok_or_else(|| std::io::Error::other("frontend process has no id"))?;
        #[cfg(windows)]
        if let Err(error) = assign_job(&job, &child) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        #[cfg(windows)]
        if let Err(error) = resume_process(&child) {
            terminate(&mut child, &job).await;
            return Err(error);
        }
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate(
                    &mut child,
                    #[cfg(unix)]
                    pgid,
                    #[cfg(windows)]
                    &job,
                )
                .await;
                return Err(std::io::Error::other("frontend stdout unavailable"));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate(
                    &mut child,
                    #[cfg(unix)]
                    pgid,
                    #[cfg(windows)]
                    &job,
                )
                .await;
                return Err(std::io::Error::other("frontend stderr unavailable"));
            }
        };
        let (sender, mut receiver) = mpsc::channel::<String>(32);
        let stdout_task = tokio::spawn(drain_log(stdout, sender.clone()));
        let stderr_task = tokio::spawn(drain_log(stderr, sender.clone()));
        drop(sender);
        let deadline = Instant::now() + config.startup_timeout;
        let upstream = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                terminate(
                    &mut child,
                    #[cfg(unix)]
                    pgid,
                    #[cfg(windows)]
                    &job,
                )
                .await;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "frontend URL readiness timeout",
                ));
            }
            match timeout(remaining.min(Duration::from_millis(50)), receiver.recv()).await {
                Ok(Some(line)) => {
                    if let Some(url) = find_loopback_url(&line) {
                        break url;
                    }
                }
                Ok(None) => {
                    terminate(
                        &mut child,
                        #[cfg(unix)]
                        pgid,
                        #[cfg(windows)]
                        &job,
                    )
                    .await;
                    return Err(std::io::Error::other("frontend exited before readiness"));
                }
                Err(_) => {
                    if Instant::now() >= deadline {
                        terminate(
                            &mut child,
                            #[cfg(unix)]
                            pgid,
                            #[cfg(windows)]
                            &job,
                        )
                        .await;
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "frontend URL readiness timeout",
                        ));
                    }
                }
            }
            if let Ok(Some(_)) = child.try_wait() {
                terminate(
                    &mut child,
                    #[cfg(unix)]
                    pgid,
                    #[cfg(windows)]
                    &job,
                )
                .await;
                return Err(std::io::Error::other("frontend exited before readiness"));
            }
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        if let Ok(Some(_)) = child.try_wait() {
            terminate(
                &mut child,
                #[cfg(unix)]
                pgid,
                #[cfg(windows)]
                &job,
            )
            .await;
            return Err(std::io::Error::other(
                "frontend exited immediately after readiness",
            ));
        }
        let drain_task = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
        let inner = Arc::new(Inner {
            child: Mutex::new(child),
            upstream,
            tunnels: Mutex::new(Vec::new()),
            log_tasks: Mutex::new(vec![stdout_task, stderr_task, drain_task]),
            cancel: CancellationToken::new(),
            #[cfg(unix)]
            pgid,
            #[cfg(windows)]
            job,
        });
        Ok(Self { inner })
    }

    pub(crate) async fn proxy(
        &self,
        request: Request<Body>,
    ) -> std::io::Result<axum::response::Response<Body>> {
        let websocket = request
            .headers()
            .get(header::UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
        if websocket {
            return self.proxy_websocket(request).await;
        }
        let (parts, body) = request.into_parts();
        let body = body
            .into_data_stream()
            .map(|result| result.map_err(std::io::Error::other));
        let target = format!("{}{}", self.inner.upstream, uri_path_with_query(&parts.uri));
        let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
            .unwrap_or(reqwest::Method::GET);
        let client = Client::new();
        let mut builder = client
            .request(method, target)
            .body(reqwest::Body::wrap_stream(body));
        for (name, value) in &parts.headers {
            if super::cache::is_forwardable(name, &parts.headers) {
                builder = builder.header(name, value);
            }
        }
        let response = tokio::select! {
            _ = self.inner.cancel.cancelled() => return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "frontend proxy cancelled")),
            result = builder.send() => result.map_err(std::io::Error::other)?,
        };
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut headers = HeaderMap::new();
        for (name, value) in response.headers() {
            if super::cache::is_forwardable(name, response.headers()) {
                if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
                    headers.append(name, value);
                }
            }
        }
        let cancel = self.inner.cancel.clone();
        let body = response
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other))
            .take_until(cancel.cancelled_owned());
        Ok(make_response(status, headers, Body::from_stream(body)))
    }

    async fn proxy_websocket(
        &self,
        request: Request<Body>,
    ) -> std::io::Result<axum::response::Response<Body>> {
        let (parts, _) = request.into_parts();
        let key = parts
            .headers
            .get("sec-websocket-key")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing websocket key")
            })?
            .to_owned();
        let target = format!(
            "{}{}",
            self.inner
                .upstream
                .replace("http://", "ws://")
                .replace("https://", "wss://"),
            uri_path_with_query(&parts.uri)
        );
        let mut builder = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&target)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", key.clone());
        if let Ok(url) = reqwest::Url::parse(&target) {
            if let Some(host) = url.host_str() {
                let host = format!(
                    "{}{}",
                    host,
                    url.port()
                        .map(|port| format!(":{port}"))
                        .unwrap_or_default()
                );
                builder = builder.header("Host", host);
            }
        }
        for (name, value) in &parts.headers {
            if !matches!(
                name.as_str(),
                "sec-websocket-key" | "sec-websocket-version" | "connection" | "upgrade" | "host"
            ) && super::cache::is_forwardable(name, &parts.headers)
            {
                builder = builder.header(name, value);
            }
        }
        let upstream_request = builder.body(()).map_err(std::io::Error::other)?;
        let handshake = timeout(Duration::from_secs(10), connect_async(upstream_request));
        let (upstream, upstream_response) = tokio::select! {
            _ = self.inner.cancel.cancelled() => return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "frontend websocket cancelled")),
            result = handshake => result
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "websocket upstream handshake timeout"))?
                .map_err(std::io::Error::other)?,
        };
        let accept = derive_accept_key(key.as_bytes());
        let upgrade = hyper::upgrade::on(Request::from_parts(parts, Body::empty()));
        let selected_protocol = upstream_response
            .headers()
            .get("sec-websocket-protocol")
            .cloned();
        let cancel = self.inner.cancel.clone();
        let handle = tokio::spawn(async move {
            let upgraded = tokio::select! {
                _ = cancel.cancelled() => return,
                result = upgrade => match result { Ok(value) => value, Err(_) => return },
            };
            let client = tokio_tungstenite::WebSocketStream::from_raw_socket(
                TokioIo::new(upgraded),
                Role::Server,
                None,
            )
            .await;
            let (mut upstream_sink, mut upstream_stream) = upstream.split();
            let (mut client_sink, mut client_stream) = client.split();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
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
        let mut active = Vec::with_capacity(tunnels.len() + 1);
        for handle in tunnels.drain(..) {
            if handle.is_finished() {
                let _ = handle.await;
            } else {
                active.push(handle);
            }
        }
        active.push(handle);
        *tunnels = active;
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        headers.insert(
            "sec-websocket-accept",
            HeaderValue::from_str(&accept).unwrap(),
        );
        if let Some(protocol) = selected_protocol {
            headers.insert(
                "sec-websocket-protocol",
                HeaderValue::from_bytes(protocol.as_bytes()).unwrap(),
            );
        }
        Ok(make_response(
            StatusCode::SWITCHING_PROTOCOLS,
            headers,
            Body::empty(),
        ))
    }
    pub(crate) async fn shutdown(self) {
        self.inner.cancel.cancel();
        let mut tunnels = self.inner.tunnels.lock().await;
        for handle in tunnels.drain(..) {
            handle.abort();
            let _ = handle.await;
        }
        drop(tunnels);
        let mut child = self.inner.child.lock().await;
        terminate(
            &mut child,
            #[cfg(unix)]
            self.inner.pgid,
            #[cfg(windows)]
            &self.inner.job,
        )
        .await;
        drop(child);
        let mut logs = self.inner.log_tasks.lock().await;
        for mut handle in logs.drain(..) {
            if timeout(Duration::from_secs(1), &mut handle).await.is_err() {
                handle.abort();
                let _ = handle.await;
            }
        }
    }
}
fn find_loopback_url(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    for token in clean.split_whitespace() {
        let candidate = token
            .trim_matches(|character: char| ",;()[]{}".contains(character))
            .trim_end_matches('/');
        let parsed = match reqwest::Url::parse(candidate) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if !matches!(parsed.scheme(), "http" | "https") || parsed.port().is_none() {
            continue;
        }
        let host = match parsed.host_str() {
            Some(host) => host,
            None => continue,
        };
        let loopback = host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
        if loopback {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn bound_line(line: String) -> String {
    const LIMIT: usize = 16 * 1024;
    if line.len() <= LIMIT {
        return line;
    }
    let mut end = LIMIT;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_owned()
}

fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.next() == Some('[') {
                for character in chars.by_ref() {
                    if ('@'..='~').contains(&character) {
                        break;
                    }
                }
            }
        } else {
            result.push(character);
        }
    }
    result
}
async fn drain_log<R>(mut reader: R, sender: mpsc::Sender<String>)
where
    R: AsyncRead + Unpin,
{
    const LIMIT: usize = 16 * 1024;
    let mut chunk = [0_u8; 4096];
    let mut line = Vec::with_capacity(LIMIT);
    loop {
        let count = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                let text = String::from_utf8_lossy(&line).into_owned();
                if sender.send(bound_line(text)).await.is_err() {
                    return;
                }
                line.clear();
            } else if line.len() < LIMIT {
                line.push(*byte);
            }
        }
    }
    if !line.is_empty() {
        let text = String::from_utf8_lossy(&line).into_owned();
        let _ = sender.send(bound_line(text)).await;
    }
}

#[cfg(windows)]
fn create_job() -> std::io::Result<JobHandle> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let set = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&mut info as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        unsafe {
            CloseHandle(job);
        }
        return Err(std::io::Error::last_os_error());
    }
    Ok(JobHandle(job))
}

#[cfg(windows)]
fn assign_job(job: &JobHandle, child: &Child) -> std::io::Result<()> {
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    let process = child
        .raw_handle()
        .ok_or_else(|| std::io::Error::other("frontend process handle unavailable"))?;
    let assigned = unsafe { AssignProcessToJobObject(job.0, process as _) };
    if assigned == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn resume_process(child: &Child) -> std::io::Result<()> {
    let process = child
        .raw_handle()
        .ok_or_else(|| std::io::Error::other("frontend process handle unavailable"))?;
    let status = unsafe { NtResumeProcess(process as _) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status))
    }
}

#[cfg(unix)]
async fn terminate(child: &mut Child, pgid: i32) {
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }
    let waited = timeout(Duration::from_secs(2), child.wait()).await;
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    if waited.is_err() {
        let _ = child.wait().await;
    }
}

#[cfg(windows)]
async fn terminate(child: &mut Child, job: &JobHandle) {
    use windows_sys::Win32::System::JobObjects::TerminateJobObject;
    let _ = unsafe { TerminateJobObject(job.0, 1) };
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
}
