use squaremap_protocol::wire::{
    AdvancedSettings, ConfigReplace, GlobalSettings, LocaleSettings, RenderSettings, UiSettings,
    WorldConfig, WorldIdentity, WorldSettings,
};
use squaremap_server::config::validate_and_stage;
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use squaremap_server::should_bind_http;
use tempfile::tempdir;

fn shadow_config() -> ConfigReplace {
    let settings = WorldSettings {
        zoom_max: 3,
        zoom_default: 3,
        background_render_interval_seconds: 1,
        background_render_max_chunks_per_interval: 1,
        player_tracker_update_interval: 1,
        marker_api_update_interval_seconds: 1,
        ..Default::default()
    };
    ConfigReplace {
        revision: 1,
        global: Some(GlobalSettings {
            http_bind: "127.0.0.1".into(),
            http_port: 8080,
            compression_ratio: 1.0,
            http_enabled: false,
            ..Default::default()
        }),
        advanced: Some(AdvancedSettings::default()),
        world: Some(settings.clone()),
        locale: Some(LocaleSettings {
            language: "lang-en.yml".into(),
            ..Default::default()
        }),
        render: Some(RenderSettings {
            progress_logging_interval_seconds: 1,
            background_interval_seconds: 1,
            background_max_chunks_per_interval: 1,
            ..Default::default()
        }),
        ui: Some(UiSettings {
            sidebar_pinned: "unpinned".into(),
            ..Default::default()
        }),
        worlds: vec![WorldConfig {
            identity: Some(WorldIdentity {
                namespace: "minecraft".into(),
                value: "overworld".into(),
                epoch: 1,
            }),
            settings: Some(settings),
        }],
        player_privacy_enabled: Some(false),
        event_capture_enabled: Some(true),
    }
}

#[tokio::test]
async fn shadow_config_replacement_does_not_bind_rust_http() {
    let candidate = shadow_config();
    let (_, _) = validate_and_stage(candidate.clone()).unwrap();
    let global = candidate.global.as_ref().unwrap();
    let directory = tempdir().unwrap();
    let root = OutputRoot::new(directory.path()).unwrap();
    let mut server = HttpServer::bind(
        HttpConfig {
            web_root: None,
            bind: format!("{}:{}", global.http_bind, global.http_port)
                .parse()
                .unwrap(),
            enabled: should_bind_http(Some(global)),
            dev_frontend: None,
        },
        root,
    )
    .await
    .unwrap();
    assert!(server.local_addr().is_none());
    server.shutdown().await.unwrap();
}

#[test]
fn rust_config_requests_http_binding_only_when_enabled() {
    let disabled = GlobalSettings {
        http_enabled: false,
        ..Default::default()
    };
    let enabled = GlobalSettings {
        http_enabled: true,
        ..Default::default()
    };
    assert!(!should_bind_http(Some(&disabled)));
    assert!(should_bind_http(Some(&enabled)));
}

#[tokio::test]
async fn rust_http_binds_on_loopback_and_releases_port_on_shutdown() {
    let directory = tempdir().unwrap();
    let root = OutputRoot::new(directory.path()).unwrap();
    let mut server = HttpServer::bind(HttpConfig::loopback(), root)
        .await
        .unwrap();
    let address = server.local_addr().unwrap();
    assert!(address.ip().is_loopback());
    server.shutdown().await.unwrap();
    assert!(std::net::TcpListener::bind(address).is_ok());
}

#[tokio::test]
async fn separate_output_roots_can_bind_independent_servers() {
    let java = tempdir().unwrap();
    let rust = tempdir().unwrap();
    let mut first = HttpServer::bind(
        HttpConfig::loopback(),
        OutputRoot::new(java.path()).unwrap(),
    )
    .await
    .unwrap();
    let mut second = HttpServer::bind(
        HttpConfig::loopback(),
        OutputRoot::new(rust.path()).unwrap(),
    )
    .await
    .unwrap();
    assert_ne!(first.local_addr(), second.local_addr());
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}
