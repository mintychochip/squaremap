//! Proves: `HttpServer` matches the documented Java/Undertow URL, status,
//! header, and body-class contract the web frontend relies on.
//!
//! Does not prove: a live Undertow process, HTTP/2, Paper plugin serving, or
//! that Java HTTP types still exist on the classpath.

use reqwest::StatusCode;
use serde::Deserialize;
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use std::collections::BTreeSet;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tempfile::tempdir;

const CONTRACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v1/http-java-contract.json"
);

const REQUIRED_IDS: &[&str] = &[
    "index-from-web-root",
    "asset-from-web-root",
    "favicon-ico-content-type",
    "settings-json-cache-and-etag",
    "settings-json-head",
    "etag-if-none-match-304",
    "etag-star-and-weak-304",
    "existing-tile-200",
    "missing-tile-empty-200",
    "registered-icon-from-output",
    "static-icon-from-web",
    "unknown-path-404",
    "post-not-allowed",
    "path-traversal-rejected",
    "disabled-http-does-not-bind",
    "hostname-bind-localhost",
    "players-json-cache-control",
    "json-post-405",
    "encoded-separator-rejected",
    "no-h2c",
];

#[derive(Deserialize)]
struct Contract {
    schema_version: u32,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    proves: String,
    verdict: String,
    #[serde(default)]
    java: Option<String>,
    #[serde(default)]
    rust: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    request: Option<RequestSpec>,
    #[serde(default)]
    expect: Option<ExpectSpec>,
}

#[derive(Deserialize)]
struct RequestSpec {
    #[serde(default)]
    method: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    if_none_match: Option<String>,
    #[serde(default)]
    if_none_match_variants: Vec<String>,
}

#[derive(Deserialize)]
struct ExpectSpec {
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    status_class: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    exact_body: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    cache_control: Option<String>,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    http_version: Option<String>,
    #[serde(default)]
    upgrade: Option<String>,
}

struct Fixture {
    web: tempfile::TempDir,
    output: tempfile::TempDir,
}

fn load_contract() -> Contract {
    let path = Path::new(CONTRACT);
    assert!(path.is_file(), "HTTP Java contract missing: {}", path.display());
    let contract: Contract = serde_json::from_slice(&fs::read(path).unwrap())
        .unwrap_or_else(|error| panic!("invalid {}: {error}", path.display()));
    assert_eq!(contract.schema_version, 1);
    assert!(!contract.cases.is_empty(), "HTTP contract denominator is empty");
    contract
}

fn write_fixture() -> Fixture {
    let web = tempdir().unwrap();
    let output = tempdir().unwrap();
    std::fs::create_dir_all(web.path().join("assets")).unwrap();
    std::fs::create_dir_all(web.path().join("images/icon")).unwrap();
    std::fs::write(web.path().join("index.html"), b"web-index").unwrap();
    std::fs::write(web.path().join("assets/app.js"), b"web-js").unwrap();
    std::fs::write(web.path().join("favicon.ico"), b"ico").unwrap();
    std::fs::write(web.path().join("images/icon/player.png"), b"player").unwrap();
    std::fs::write(output.path().join("index.html"), b"output-index").unwrap();
    std::fs::create_dir_all(output.path().join("tiles")).unwrap();
    std::fs::create_dir_all(output.path().join("images/icon/registered")).unwrap();
    std::fs::write(output.path().join("tiles/settings.json"), br#"{"ok":true}"#).unwrap();
    std::fs::write(output.path().join("tiles/players.json"), br#"{"players":[],"max":20}"#).unwrap();
    std::fs::write(output.path().join("tiles/existing.png"), b"png").unwrap();
    std::fs::write(output.path().join("images/icon/registered/spawn.png"), b"icon").unwrap();
    Fixture { web, output }
}

async fn bind_fixture(fixture: &Fixture) -> HttpServer {
    HttpServer::bind(
        HttpConfig {
            web_root: Some(fixture.web.path().to_path_buf()),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            enabled: true,
            dev_frontend: None,
        },
        OutputRoot::new(fixture.output.path()).unwrap(),
    )
    .await
    .unwrap()
}

async fn settings_etag(client: &reqwest::Client, base: &str) -> String {
    let response = client
        .get(format!("{base}/tiles/settings.json"))
        .send()
        .await
        .unwrap();
    response.headers()[reqwest::header::ETAG]
        .to_str()
        .unwrap()
        .to_owned()
}

async fn send(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    if_none_match: Option<&str>,
) -> reqwest::Response {
    let mut request = match method {
        "GET" => client.get(url),
        "HEAD" => client.head(url),
        "POST" => client.post(url),
        other => panic!("unsupported method {other}"),
    };
    if let Some(value) = if_none_match {
        request = request.header(reqwest::header::IF_NONE_MATCH, value);
    }
    request.send().await.unwrap()
}

fn header_or_none<'a>(response: &'a reqwest::Response, name: reqwest::header::HeaderName) -> Option<&'a str> {
    response.headers().get(name).and_then(|value| value.to_str().ok())
}

async fn assert_expect(id: &str, expect: &ExpectSpec, response: reqwest::Response) {
    if let Some(status) = expect.status {
        assert_eq!(
            response.status().as_u16(),
            status,
            "{id} status"
        );
    }
    if expect.status_class.as_deref() == Some("client_error") {
        assert!(
            response.status().is_client_error(),
            "{id} expected 4xx, got {}",
            response.status()
        );
    }
    if let Some(content_type) = expect.content_type.as_deref() {
        let actual = header_or_none(&response, reqwest::header::CONTENT_TYPE);
        if content_type == "absent" {
            assert!(actual.is_none(), "{id} content-type should be absent, got {actual:?}");
        } else {
            assert_eq!(actual, Some(content_type), "{id} content-type");
        }
    }
    if let Some(cache) = expect.cache_control.as_deref() {
        assert_eq!(
            header_or_none(&response, reqwest::header::CACHE_CONTROL),
            Some(cache),
            "{id} cache-control"
        );
    }
    if expect.etag.as_deref() == Some("quoted") {
        let etag = header_or_none(&response, reqwest::header::ETAG)
            .unwrap_or_else(|| panic!("{id} missing etag"));
        assert!(
            etag.starts_with('"') && etag.ends_with('"'),
            "{id} etag {etag} is not quoted"
        );
    }
    if expect.http_version.as_deref() == Some("HTTP/1.1") {
        assert_eq!(response.version(), reqwest::Version::HTTP_11, "{id} http version");
    }
    if expect.upgrade.as_deref() == Some("absent") {
        assert!(
            header_or_none(&response, reqwest::header::UPGRADE).is_none(),
            "{id} upgrade should be absent"
        );
    }
    let body = response.bytes().await.unwrap();
    match expect.body.as_deref() {
        Some("empty") => assert!(body.is_empty(), "{id} body should be empty"),
        Some("exact") => {
            let expected = expect
                .exact_body
                .as_ref()
                .unwrap_or_else(|| panic!("{id} exact_body missing"));
            assert_eq!(body.as_ref(), expected.as_bytes(), "{id} body");
        }
        Some("ignored") | None => {}
        other => panic!("{id} unknown body class {other:?}"),
    }
}

#[tokio::test]
async fn java_http_contract_denominator_and_rows() {
    let contract = load_contract();
    let ids: BTreeSet<&str> = contract.cases.iter().map(|case| case.id.as_str()).collect();
    let required: BTreeSet<&str> = REQUIRED_IDS.iter().copied().collect();
    assert_eq!(ids, required, "HTTP Java contract denominator drifted");
    assert_eq!(contract.cases.len(), REQUIRED_IDS.len());

    for case in &contract.cases {
        if case.verdict == "deliberate" {
            assert!(case.java.as_ref().is_some_and(|value| !value.is_empty()), "{} missing java", case.id);
            assert!(case.rust.as_ref().is_some_and(|value| !value.is_empty()), "{} missing rust", case.id);
            assert!(case.reason.as_ref().is_some_and(|value| !value.is_empty()), "{} missing reason", case.id);
        } else {
            assert_eq!(case.verdict, "match", "{} verdict", case.id);
        }
        assert!(!case.proves.is_empty(), "{} proves", case.id);
    }

    let fixture = write_fixture();
    let mut server = bind_fixture(&fixture).await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.local_addr().unwrap());
    let etag = settings_etag(&client, &base).await;

    for case in &contract.cases {
        match case.mode.as_deref() {
            Some("disabled") => {
                let dir = tempdir().unwrap();
                let root = OutputRoot::new(dir.path()).unwrap();
                let mut disabled = HttpServer::bind(
                    HttpConfig {
                        web_root: None,
                        bind: SocketAddr::from(([127, 0, 0, 1], 0)),
                        enabled: false,
                        dev_frontend: None,
                    },
                    root.clone(),
                )
                .await
                .unwrap();
                assert!(disabled.local_addr().is_none(), "{} bound a listener", case.id);
                root.atomic_write("index.html", b"still works").unwrap();
                disabled.shutdown().await.unwrap();
            }
            Some("hostname") => {
                let bind = tokio::net::lookup_host("localhost:0")
                    .await
                    .unwrap()
                    .next()
                    .expect("localhost should resolve");
                let host_output = tempdir().unwrap();
                let mut host_server = HttpServer::bind(
                    HttpConfig {
                        web_root: Some(fixture.web.path().to_path_buf()),
                        bind,
                        enabled: true,
                        dev_frontend: None,
                    },
                    OutputRoot::new(host_output.path()).unwrap(),
                )
                .await
                .unwrap_or_else(|error| panic!("{} bind localhost: {error}", case.id));
                let page = client
                    .get(format!("http://{}/", host_server.local_addr().unwrap()))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(page.status(), StatusCode::OK, "{}", case.id);
                host_server.shutdown().await.unwrap();
            }
            _ => {
                let request = case
                    .request
                    .as_ref()
                    .unwrap_or_else(|| panic!("{} missing request", case.id));
                let expect = case
                    .expect
                    .as_ref()
                    .unwrap_or_else(|| panic!("{} missing expect", case.id));
                let mut paths = request.paths.clone();
                if let Some(path) = &request.path {
                    paths.insert(0, path.clone());
                }
                assert!(!paths.is_empty(), "{} has no paths", case.id);
                let variants = if request.if_none_match_variants.is_empty() {
                    vec![request.if_none_match.clone()]
                } else {
                    request
                        .if_none_match_variants
                        .iter()
                        .map(|variant| match variant.as_str() {
                            "weak" => Some(format!("W/{etag}")),
                            other => Some(other.to_string()),
                        })
                        .collect()
                };
                for path in &paths {
                    for variant in &variants {
                        let if_none = match variant.as_deref() {
                            Some("etag") => Some(etag.as_str()),
                            Some(other) => Some(other),
                            None => None,
                        };
                        let response = send(
                            &client,
                            &request.method,
                            &format!("{base}{path}"),
                            if_none,
                        )
                        .await;
                        assert_expect(&case.id, expect, response).await;
                    }
                }
            }
        }
    }

    server.shutdown().await.unwrap();
}
