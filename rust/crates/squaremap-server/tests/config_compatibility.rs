use squaremap_protocol::wire::{
    AdvancedSettings, ConfigReplace, GlobalSettings, LocaleSettings, RenderSettings, UiSettings,
    WorldConfig, WorldIdentity, WorldSettings,
};
use squaremap_server::config::ConfigStore;

fn valid(revision: u64) -> ConfigReplace {
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
        revision,
        global: Some(GlobalSettings { http_port: 8080, compression_ratio: 1.0, ..Default::default() }),
        advanced: Some(AdvancedSettings::default()),
        world: Some(settings.clone()),
        locale: Some(LocaleSettings { language: "lang-en.yml".into(), ..Default::default() }),
        render: Some(RenderSettings { progress_logging_interval_seconds: 1, background_interval_seconds: 1, background_max_chunks_per_interval: 1, ..Default::default() }),
        ui: Some(UiSettings { sidebar_pinned: "unpinned".into(), ..Default::default() }),
        worlds: vec![WorldConfig {
            identity: Some(WorldIdentity { namespace: "minecraft".into(), value: "overworld".into(), epoch: 1 }),
            settings: Some(settings),
        }],
        player_privacy_enabled: Some(false),
        event_capture_enabled: Some(true),
    }
}

#[test]
fn compatible_revision_replaces_active_config_atomically() {
    let mut store = ConfigStore::default();
    store.stage_and_swap(valid(1)).unwrap();
    store.stage_and_swap(valid(2)).unwrap();
    assert_eq!(store.active().unwrap().revision, 2);
}

#[test]
fn invalid_world_settings_leave_previous_config_active() {
    let mut store = ConfigStore::default();
    store.stage_and_swap(valid(1)).unwrap();
    let mut invalid = valid(2);
    invalid.worlds[0].settings.as_mut().unwrap().zoom_default = 4;
    assert!(store.stage_and_swap(invalid).is_err());
    assert_eq!(store.active().unwrap().revision, 1);
}
