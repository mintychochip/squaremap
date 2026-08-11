mod cache;
mod dev_frontend;
mod static_files;

pub use dev_frontend::DevFrontendConfig;
use crate::output::OutputRoot;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub bind: SocketAddr,
    pub enabled: bool,
    pub dev_frontend: Option<DevFrontendConfig>,
}

impl HttpConfig {
    pub fn loopback() -> Self { Self { bind: SocketAddr::from(([127, 0, 0, 1], 0)), enabled: true, dev_frontend: None } }
}

pub struct HttpServer {
    addr: Option<SocketAddr>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
    dev: Option<dev_frontend::DevFrontend>,
}

impl HttpServer {
    pub async fn bind(config: HttpConfig, root: OutputRoot) -> std::io::Result<Self> {
        if !config.enabled {
            return Ok(Self { addr: None, stop: None, task: None, dev: None });
        }
        let dev = match config.dev_frontend {
            Some(dev_config) => Some(dev_frontend::DevFrontend::start(dev_config).await?),
            None => None,
        };
        let listener = match TcpListener::bind(config.bind).await {
            Ok(listener) => listener,
            Err(error) => {
                if let Some(dev) = dev { dev.shutdown().await; }
                return Err(error);
            }
        };
        let addr = listener.local_addr()?;
        let (stop_tx, stop_rx) = oneshot::channel();
        let state = Arc::new(AppState { root, dev: dev.as_ref().map(|frontend| frontend.clone()) });
        let router = Router::new().fallback(handle_request).with_state(state);
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, router).with_graceful_shutdown(async { let _ = stop_rx.await; }).await;
            let _ = result;
        });
        Ok(Self { addr: Some(addr), stop: Some(stop_tx), task: Some(task), dev })
    }

    pub fn local_addr(&self) -> Option<SocketAddr> { self.addr }

    pub async fn shutdown(&mut self) -> std::io::Result<()> {
        if let Some(stop) = self.stop.take() { let _ = stop.send(()); }
        if let Some(task) = self.task.take() { let _ = task.await; }
        if let Some(dev) = self.dev.take() { dev.shutdown().await; }
        Ok(())
    }
}

struct AppState { root: OutputRoot, dev: Option<dev_frontend::DevFrontend> }

async fn handle_request(state: axum::extract::State<Arc<AppState>>, request: Request<Body>) -> Response<Body> {
    let method = request.method().clone();
    let raw_path = request.uri().path();
    let decoded = match static_files::decode_path(raw_path) {
        Ok(path) => path,
        Err(_) => return response(StatusCode::BAD_REQUEST, HeaderMap::new(), Body::empty()),
    };
    let proxy = state.dev.as_ref().is_some_and(|_| !is_static_exclusion(&decoded));
    if proxy {
        if let Some(dev) = state.dev.as_ref() {
            return dev.proxy(request).await.unwrap_or_else(|_| response(StatusCode::BAD_GATEWAY, HeaderMap::new(), Body::empty()));
        }
    }
    if method != Method::GET && method != Method::HEAD {
        return response(StatusCode::METHOD_NOT_ALLOWED, HeaderMap::new(), Body::empty());
    }
    static_files::serve(&state.root, &decoded, &method, request.headers())
}

fn is_static_exclusion(path: &std::path::Path) -> bool {
    let mut components = path.components();
    let Some(std::path::Component::Normal(first)) = components.next() else { return false };
    if first == "tiles" { return true; }
    if first == "images" && components.next() == Some(std::path::Component::Normal("icon".as_ref())) && components.next() == Some(std::path::Component::Normal("registered".as_ref())) { return true; }
    false
}

fn response(status: StatusCode, headers: HeaderMap, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

pub(crate) fn make_response(status: StatusCode, headers: HeaderMap, body: Body) -> Response<Body> { response(status, headers, body) }


pub(crate) fn uri_path_with_query(uri: &Uri) -> String {
    let mut value = uri.path().to_owned();
    if let Some(query) = uri.query() { value.push('?'); value.push_str(query); }
    value
}
