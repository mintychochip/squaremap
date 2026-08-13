use super::{InstallRequest, TileInstaller};
use async_trait::async_trait;
use squaremap_render::coordinates::{ChunkCoord, CHUNKS_PER_REGION};
use squaremap_render::{
    BiomeSource, PngOptions, RegionPixels, RenderContext, RenderSettings, SnapshotBiomeSource,
    PublishResult, TilePyramid, TileStore, TileStoreError, render_chunk,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct WorldRenderConfig {
    pub settings: RenderSettings,
    pub invisible_ids: Arc<[u32]>,
    pub iterate_up_base_ids: Arc<[u32]>,
    pub biome_zoom_seed: i64,
    pub max_zoom: u8,
    pub png_options: PngOptions,
    pub tile_prefix: PathBuf,
}

#[derive(Clone)]
pub struct PrefixedTileStore {
    prefix: PathBuf,
    inner: Arc<dyn TileStore>,
}
impl PrefixedTileStore {
    pub fn new(prefix: impl Into<PathBuf>, inner: Arc<dyn TileStore>) -> Result<Self, String> {
        let prefix = prefix.into();
        if prefix.as_os_str().is_empty() || prefix.is_absolute() || prefix.components().any(|component| {
            !matches!(component, std::path::Component::Normal(_))
        }) {
            return Err("tile store prefix must be a non-empty relative path".into());
        }
        Ok(Self { prefix, inner })
    }
    fn path(&self, path: &Path) -> PathBuf { self.prefix.join(path) }
}
#[async_trait]
impl TileStore for PrefixedTileStore {
    async fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, TileStoreError> {
        self.inner.read(&self.path(path)).await
    }
    async fn publish(&self, path: &Path, bytes: &[u8]) -> Result<PublishResult, TileStoreError> {
        self.inner.publish(&self.path(path), bytes).await
    }
}

/// Production installer that renders an accepted snapshot bundle and atomically publishes its tile pyramid.
pub struct RenderTileInstaller {
    store: Arc<dyn TileStore>,
    worlds: RwLock<HashMap<squaremap_state::WorldId, WorldRenderConfig>>,
    pyramids: RwLock<HashMap<squaremap_state::WorldId, Arc<TilePyramid>>>,
}
impl RenderTileInstaller {
    pub fn new(store: Arc<dyn TileStore>) -> Self {
        Self { store, worlds: RwLock::new(HashMap::new()), pyramids: RwLock::new(HashMap::new()) }
    }

    pub fn configure_world(&self, world: squaremap_state::WorldId, config: WorldRenderConfig) -> Result<(), String> {
        let world_store = Arc::new(PrefixedTileStore::new(config.tile_prefix.clone(), self.store.clone())?);
        let pyramid = TilePyramid::new(world_store, config.max_zoom, config.png_options).map_err(|error| error.to_string())?;
        self.worlds.write().map_err(|_| "world render config lock poisoned")?.insert(world.clone(), config);
        self.pyramids.write().map_err(|_| "tile pyramid lock poisoned")?.insert(world, Arc::new(pyramid));
        Ok(())
    }
}


#[async_trait]
impl TileInstaller for RenderTileInstaller {
    async fn install(&self, request: InstallRequest) -> Result<(), String> {
        let config = self.worlds.read().map_err(|_| "world render config lock poisoned")?
            .get(&request.world).cloned().ok_or_else(|| "world render config is unavailable".to_string())?;
        let pyramid = self.pyramids.read().map_err(|_| "tile pyramid lock poisoned")?
            .get(&request.world).cloned().ok_or_else(|| "world tile pyramid is unavailable".to_string())?;
        let generation = request.snapshots.center.registry_generation.clone();
        let biome_source: Option<Arc<dyn BiomeSource>> = if config.settings.biomes_enabled {
            let mut source = SnapshotBiomeSource::new(generation.clone(), config.biome_zoom_seed)
                .with_snapshot(request.snapshots.center.clone()).map_err(|error| error.to_string())?;
            for snapshot in request.snapshots.biome_sources.clone() {
                source = source.with_snapshot(snapshot).map_err(|error| error.to_string())?;
            }
            if let Some(north) = request.snapshots.north.clone() {
                source = source.with_snapshot(north).map_err(|error| error.to_string())?;
            }
            if let Some(south) = request.snapshots.south.clone() {
                source = source.with_snapshot(south).map_err(|error| error.to_string())?;
            }
            for (x, y, z, biome, color) in request.snapshots.grass_resolutions.iter().copied() {
                source = source.with_grass_resolved(x, y, z, biome, color);
            }
            Some(Arc::new(source))
        } else {
            None
        };
        let context = RenderContext::try_new(
            generation,
            config.settings,
            config.invisible_ids.iter().copied(),
            config.iterate_up_base_ids.iter().copied(),
            biome_source,
            || false,
        ).map_err(|error| error.to_string())?;
        let rendered = render_chunk(
            &context,
            request.snapshots.north.as_deref(),
            &request.snapshots.center,
            request.snapshots.south.as_deref(),
        ).map_err(|error| error.to_string())?;
        let chunk = ChunkCoord { x: request.coordinate.x, z: request.coordinate.z };
        let region = chunk.region();
        let local_x = request.coordinate.x.rem_euclid(CHUNKS_PER_REGION) as usize * 16;
        let local_z = request.coordinate.z.rem_euclid(CHUNKS_PER_REGION) as usize * 16;
        let mut pixels = RegionPixels::empty();
        for x in 0..16 {
            for z in 0..16 {
                pixels.set_argb(local_x + x, local_z + z, rendered.pixel(x, z))
                    .map_err(|error| error.to_string())?;
            }
        }
        pyramid.apply_region(region, &pixels).await.map_err(|error| error.to_string())?;
        Ok(())
    }
}
