use squaremap_protocol::wire::{BridgePolicyReplace, ConfigReplace, VisibilityLimit, VisibilityLimitKind, WorldConfig, WorldSettings};
use std::collections::HashSet;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveConfig { pub revision: u64, pub config: ConfigReplace }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    MissingGroup(&'static str),
    MissingWorldIdentity,
    DuplicateWorldIdentity,
    InvalidRevision,
    InvalidBounds(&'static str),
    InvalidValue(&'static str),
    InvalidGeometry(&'static str),
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingGroup(name) => write!(f, "{name} settings are required"),
            Self::MissingWorldIdentity => f.write_str("every world config requires an identity"),
            Self::DuplicateWorldIdentity => f.write_str("world identities must be unique"),
            Self::InvalidRevision => f.write_str("config revision must be positive"),
            Self::InvalidBounds(name) => write!(f, "invalid bounds for {name}"),
            Self::InvalidValue(name) => write!(f, "invalid value for {name}"),
            Self::InvalidGeometry(name) => write!(f, "invalid visibility geometry for {name}"),
        }
    }
}
impl std::error::Error for ConfigError {}

#[derive(Default)]
pub struct ConfigStore { active: Option<ActiveConfig> }
impl ConfigStore {
    pub fn active(&self) -> Option<&ActiveConfig> { self.active.as_ref() }
    pub fn stage_and_swap(&mut self, candidate: ConfigReplace) -> Result<BridgePolicyReplace, ConfigError> {
        let (staged, policy) = validate_and_stage(candidate)?;
        self.active = Some(staged);
        Ok(policy)
    }
}

pub fn validate_and_stage(candidate: ConfigReplace) -> Result<(ActiveConfig, BridgePolicyReplace), ConfigError> {
    if candidate.revision == 0 { return Err(ConfigError::InvalidRevision); }
    let global = candidate.global.as_ref().ok_or(ConfigError::MissingGroup("global"))?;
    candidate.advanced.as_ref().ok_or(ConfigError::MissingGroup("advanced"))?;
    let default_world = candidate.world.as_ref().ok_or(ConfigError::MissingGroup("world"))?;
    candidate.locale.as_ref().ok_or(ConfigError::MissingGroup("locale"))?;
    let render = candidate.render.as_ref().ok_or(ConfigError::MissingGroup("render"))?;
    let ui = candidate.ui.as_ref().ok_or(ConfigError::MissingGroup("ui"))?;
    let player_privacy_enabled = candidate.player_privacy_enabled.ok_or(ConfigError::InvalidValue("player_privacy_enabled"))?;
    let event_capture_enabled = candidate.event_capture_enabled.ok_or(ConfigError::InvalidValue("event_capture_enabled"))?;
    if global.http_port == 0 || global.http_port > 65_535 { return Err(ConfigError::InvalidBounds("http_port")); }
    if !global.compression_ratio.is_finite() || !(0.0..=1.0).contains(&global.compression_ratio) { return Err(ConfigError::InvalidBounds("compression_ratio")); }
    validate_world_settings(default_world)?;
    if render.progress_logging_interval_seconds == 0 || render.background_interval_seconds == 0 { return Err(ConfigError::InvalidBounds("render interval")); }
    if render.background_max_chunks_per_interval == 0 || render.background_max_threads < -1 { return Err(ConfigError::InvalidBounds("render background")); }
    if ui.sidebar_pinned.is_empty() { return Err(ConfigError::InvalidValue("sidebar_pinned")); }
    let mut identities = HashSet::new();
    for world in &candidate.worlds {
        validate_world(world)?;
        let identity = world.identity.as_ref().ok_or(ConfigError::MissingWorldIdentity)?;
        if !identities.insert((identity.namespace.clone(), identity.value.clone(), identity.epoch)) { return Err(ConfigError::DuplicateWorldIdentity); }
    }
    if candidate.worlds.is_empty() { return Err(ConfigError::InvalidValue("worlds")); }
    let policy = BridgePolicyReplace {
        revision: candidate.revision,
        max_control_frame_bytes: 1 << 20,
        max_snapshot_frame_bytes: 16 << 20,
        max_uncompressed_snapshot_bytes: 64 << 20,
        max_pending_dirty_chunks: 65_536,
        snapshot_credits: 64,
        player_privacy_enabled,
        event_capture_enabled,
    };
    Ok((ActiveConfig { revision: candidate.revision, config: candidate }, policy))
}

fn validate_world(world: &WorldConfig) -> Result<(), ConfigError> {
    let identity = world.identity.as_ref().ok_or(ConfigError::MissingWorldIdentity)?;
    if identity.namespace.is_empty() || identity.value.is_empty() || identity.epoch == 0 { return Err(ConfigError::InvalidValue("world identity")); }
    let settings = world.settings.as_ref().ok_or(ConfigError::MissingGroup("world settings"))?;
    validate_world_settings(settings)
}

fn validate_world_settings(settings: &WorldSettings) -> Result<(), ConfigError> {
    if settings.map_biomes_blend > 15 { return Err(ConfigError::InvalidBounds("map_biomes_blend")); }
    if settings.zoom_max < 0 || settings.zoom_default < 0 || settings.zoom_extra < 0 || settings.zoom_default > settings.zoom_max { return Err(ConfigError::InvalidBounds("zoom")); }
    if settings.max_render_threads < -1 || settings.background_render_max_threads < -1 { return Err(ConfigError::InvalidBounds("render threads")); }
    if settings.background_render_interval_seconds == 0 || settings.background_render_max_chunks_per_interval == 0 || settings.player_tracker_update_interval == 0 || settings.marker_api_update_interval_seconds == 0 { return Err(ConfigError::InvalidBounds("interval")); }
    for visibility in &settings.visibility_limits { validate_visibility(visibility)?; }
    Ok(())
}

fn validate_visibility(limit: &VisibilityLimit) -> Result<(), ConfigError> {
    let kind = VisibilityLimitKind::try_from(limit.kind).map_err(|_| ConfigError::InvalidValue("visibility kind"))?;
    match kind {
        VisibilityLimitKind::WorldBorder => {
            if !limit.points.is_empty() || limit.radius != 0.0 { return Err(ConfigError::InvalidGeometry("world_border")); }
        }
        VisibilityLimitKind::Circle => {
            if !limit.points.is_empty() || !limit.radius.is_finite() || limit.radius <= 0.0 { return Err(ConfigError::InvalidGeometry("circle")); }
        }
        VisibilityLimitKind::Rectangle => {
            if limit.points.len() != 2 {
                return Err(ConfigError::InvalidGeometry("rectangle"));
            }
            let min = &limit.points[0];
            let max = &limit.points[1];
            if min.x >= max.x || min.z >= max.z {
                return Err(ConfigError::InvalidGeometry("rectangle"));
            }
        }
        VisibilityLimitKind::Polygon => {
            if limit.points.len() < 3 { return Err(ConfigError::InvalidGeometry("polygon")); }
        }
        VisibilityLimitKind::Unspecified => return Err(ConfigError::InvalidValue("visibility kind")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use squaremap_protocol::wire::{AdvancedSettings, GlobalSettings, LocaleSettings, Point, RenderSettings, UiSettings, VisibilityLimit, WorldIdentity, WorldSettings};

    fn valid(revision: u64) -> ConfigReplace {
        let settings = WorldSettings { zoom_max: 3, zoom_default: 3, background_render_interval_seconds: 1, background_render_max_chunks_per_interval: 1, player_tracker_update_interval: 1, marker_api_update_interval_seconds: 1, ..Default::default() };
        ConfigReplace {
            revision,
            global: Some(GlobalSettings { http_port: 8080, compression_ratio: 1.0, ..Default::default() }),
            advanced: Some(AdvancedSettings::default()),
            world: Some(settings.clone()),
            locale: Some(LocaleSettings { language: "lang-en.yml".into(), ..Default::default() }),
            render: Some(RenderSettings { progress_logging_interval_seconds: 1, background_interval_seconds: 1, background_max_chunks_per_interval: 1, ..Default::default() }),
            ui: Some(UiSettings { sidebar_pinned: "unpinned".into(), ..Default::default() }),
            worlds: vec![WorldConfig { identity: Some(WorldIdentity { namespace: "minecraft".into(), value: "overworld".into(), epoch: 1 }), settings: Some(settings) }],
            player_privacy_enabled: Some(false),
            event_capture_enabled: Some(true),
        }
    }

    #[test]
    fn validates_two_worlds_and_swaps_atomically() {
        let mut store = ConfigStore::default();
        let mut candidate = valid(1);
        candidate.worlds.push(WorldConfig { identity: Some(WorldIdentity { namespace: "minecraft".into(), value: "nether".into(), epoch: 2 }), settings: candidate.worlds[0].settings.clone() });
        let policy = store.stage_and_swap(candidate).unwrap();
        assert_eq!(policy.revision, 1);
        assert!(!policy.player_privacy_enabled);
        assert!(store.active().unwrap().config.worlds.len() == 2);
    }

    #[test]
    fn rejects_invalid_without_mutating_active_state() {
        let mut store = ConfigStore::default();
        store.stage_and_swap(valid(1)).unwrap();
        let before = store.active().unwrap().clone();
        let mut invalid = valid(2);
        invalid.worlds[0].identity = None;
        assert_eq!(store.stage_and_swap(invalid), Err(ConfigError::MissingWorldIdentity));
        assert_eq!(store.active(), Some(&before));
    }

    #[test]
    fn rejects_missing_groups_visibility_geometry_and_duplicate_identity() {
        let mut missing = valid(1);
        missing.advanced = None;
        assert_eq!(validate_and_stage(missing), Err(ConfigError::MissingGroup("advanced")));
        let mut circle = valid(2);
        circle.worlds[0].settings.as_mut().unwrap().visibility_limits.push(VisibilityLimit { kind: VisibilityLimitKind::Circle as i32, radius: 0.0, ..Default::default() });
        assert_eq!(validate_and_stage(circle), Err(ConfigError::InvalidGeometry("circle")));
        let mut duplicate = valid(3);
        duplicate.worlds.push(duplicate.worlds[0].clone());
        assert_eq!(validate_and_stage(duplicate), Err(ConfigError::DuplicateWorldIdentity));
    }
    #[test]
    fn rejects_reversed_rectangle_without_mutating_active_state() {
        let mut store = ConfigStore::default();
        store.stage_and_swap(valid(1)).unwrap();
        let before = store.active().unwrap().clone();
        let mut invalid = valid(2);
        invalid.worlds[0].settings.as_mut().unwrap().visibility_limits.push(VisibilityLimit {
            kind: VisibilityLimitKind::Rectangle as i32,
            points: vec![Point { x: 5, z: 0 }, Point { x: 5, z: 10 }],
            ..Default::default()
        });
        assert_eq!(store.stage_and_swap(invalid), Err(ConfigError::InvalidGeometry("rectangle")));
        assert_eq!(store.active(), Some(&before));
    }
}
