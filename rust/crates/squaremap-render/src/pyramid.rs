use crate::coordinates::{RegionCoord, tile_for_region, tile_origin};
use crate::png::{PngError, PngOptions, decode_rgba_png, encode_rgba_png};
use crate::tile::{RegionPixels, TILE_RGBA_BYTES, TILE_SIZE, TileError, TileStore};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplyResult {
    pub changed_paths: Vec<PathBuf>,
    pub warnings: Vec<TileWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileWarning {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone)]
pub struct StagedRegion {
    tiles: Vec<(PathBuf, Vec<u8>)>,
}
pub struct TilePyramid {
    store: Arc<dyn TileStore>,
    max_zoom: u8,
    png_options: PngOptions,
    locks: Mutex<HashMap<PathBuf, LockEntry>>,
}

impl fmt::Debug for TilePyramid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TilePyramid")
            .field("max_zoom", &self.max_zoom)
            .field("png_options", &self.png_options)
            .field("live_lock_count", &self.live_lock_count())
            .finish_non_exhaustive()
    }
}

impl TilePyramid {
    pub fn new(
        store: Arc<dyn TileStore>,
        max_zoom: u8,
        png_options: PngOptions,
    ) -> Result<Self, TileError> {
        if max_zoom > 9 {
            return Err(TileError::InvalidZoom(max_zoom));
        }
        Ok(Self {
            store,
            max_zoom,
            png_options,
            locks: Mutex::new(HashMap::new()),
        })
    }

    pub async fn apply_region(
        &self,
        region: RegionCoord,
        pixels: &RegionPixels,
    ) -> Result<ApplyResult, TileError> {
        self.apply_region_with_publish(region, pixels, true).await
    }

    pub async fn stage_region(
        &self,
        region: RegionCoord,
        pixels: &RegionPixels,
    ) -> Result<StagedRegion, TileError> {
        let mut tiles = Vec::new();
        for zoom in 0..=self.max_zoom {
            let coordinate = tile_for_region(region.x, region.z, zoom, self.max_zoom)
                .map_err(|_| TileError::InvalidZoom(zoom))?;
            let (origin_x, origin_z) =
                tile_origin(region.x, region.z, zoom).map_err(|_| TileError::InvalidZoom(zoom))?;
            let path = PathBuf::from(format!(
                "{}/{}_{}.png",
                coordinate.level, coordinate.x, coordinate.z
            ));
            let bytes = self
                .encode_one(
                    &path,
                    pixels,
                    zoom,
                    usize::from(origin_x),
                    usize::from(origin_z),
                )
                .await?;
            tiles.push((path, bytes));
        }
        Ok(StagedRegion { tiles })
    }

    pub async fn publish_staged(&self, staged: StagedRegion) -> Result<ApplyResult, TileError> {
        let mut result = ApplyResult::default();
        for (path, bytes) in staged.tiles {
            let registration = self.register_lock(&path);
            let _guard = registration.lock.clone().lock_owned().await;
            let warning = self
                .store
                .publish(&path, &bytes)
                .await
                .map(|published| published.warning)
                .map_err(|error| TileError::Publish {
                    path: path.clone(),
                    message: error.message,
                })?;
            result.changed_paths.push(path.clone());
            if let Some(message) = warning {
                result.warnings.push(TileWarning { path, message });
            }
        }
        Ok(result)
    }
    async fn encode_one(
        &self,
        path: &Path,
        pixels: &RegionPixels,
        zoom: u8,
        origin_x: usize,
        origin_z: usize,
    ) -> Result<Vec<u8>, TileError> {
        let existing = self
            .store
            .read(path)
            .await
            .map_err(|error| TileError::Store {
                path: path.to_owned(),
                message: error.message,
            })?;
        let mut rgba = match existing {
            Some(encoded) => decode_for_path(path, &encoded)?,
            None => vec![0_u8; TILE_RGBA_BYTES],
        };
        let step = 1_usize << zoom;
        for x in (0..TILE_SIZE).step_by(step) {
            for z in (0..TILE_SIZE).step_by(step) {
                let Some(color) = pixels.sample(x, z) else {
                    continue;
                };
                let offset = ((origin_z + z / step) * TILE_SIZE + origin_x + x / step) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
        encode_rgba_png(&rgba, self.png_options).map_err(|error| TileError::Encode {
            path: path.to_owned(),
            message: error.to_string(),
        })
    }
    async fn apply_region_with_publish(
        &self,
        region: RegionCoord,
        pixels: &RegionPixels,
        publish: bool,
    ) -> Result<ApplyResult, TileError> {
        let mut result = ApplyResult::default();
        for zoom in 0..=self.max_zoom {
            let coordinate = tile_for_region(region.x, region.z, zoom, self.max_zoom)
                .map_err(|_| TileError::InvalidZoom(zoom))?;
            let (origin_x, origin_z) =
                tile_origin(region.x, region.z, zoom).map_err(|_| TileError::InvalidZoom(zoom))?;
            let path = PathBuf::from(format!(
                "{}/{}_{}.png",
                coordinate.level, coordinate.x, coordinate.z
            ));
            let tile_result = {
                let registration = self.register_lock(&path);
                let _guard = registration.lock.clone().lock_owned().await;
                if publish {
                    self.apply_one(
                        &path,
                        pixels,
                        zoom,
                        usize::from(origin_x),
                        usize::from(origin_z),
                    )
                    .await
                } else {
                    self.apply_one(
                        &path,
                        pixels,
                        zoom,
                        usize::from(origin_x),
                        usize::from(origin_z),
                    )
                    .await
                }
            };
            let warning = tile_result?;
            result.changed_paths.push(path.clone());
            if let Some(message) = warning {
                result.warnings.push(TileWarning { path, message });
            }
        }
        Ok(result)
    }

    pub fn live_lock_count(&self) -> usize {
        lock_unpoisoned(&self.locks).len()
    }

    async fn apply_one(
        &self,
        path: &Path,
        pixels: &RegionPixels,
        zoom: u8,
        origin_x: usize,
        origin_z: usize,
    ) -> Result<Option<String>, TileError> {
        let existing = self
            .store
            .read(path)
            .await
            .map_err(|error| TileError::Store {
                path: path.to_owned(),
                message: error.message,
            })?;
        let mut rgba = match existing {
            Some(encoded) => decode_for_path(path, &encoded)?,
            None => vec![0_u8; TILE_RGBA_BYTES],
        };
        let step = 1_usize << zoom;
        for x in (0..TILE_SIZE).step_by(step) {
            for z in (0..TILE_SIZE).step_by(step) {
                let Some(color) = pixels.sample(x, z) else {
                    continue;
                };
                let destination_x = origin_x + x / step;
                let destination_z = origin_z + z / step;
                debug_assert!(destination_x < TILE_SIZE && destination_z < TILE_SIZE);
                let offset = (destination_z * TILE_SIZE + destination_x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
        let encoded =
            encode_rgba_png(&rgba, self.png_options).map_err(|error| TileError::Encode {
                path: path.to_owned(),
                message: error.to_string(),
            })?;
        self.store
            .publish(path, &encoded)
            .await
            .map(|published| published.warning)
            .map_err(|error| TileError::Publish {
                path: path.to_owned(),
                message: error.message,
            })
    }
    fn register_lock(&self, path: &Path) -> LockRegistration<'_> {
        let mut locks = lock_unpoisoned(&self.locks);
        let entry = locks.entry(path.to_owned()).or_insert_with(|| LockEntry {
            lock: Arc::new(AsyncMutex::new(())),
            users: 0,
        });
        entry.users += 1;
        LockRegistration {
            pyramid: self,
            path: path.to_owned(),
            lock: Arc::clone(&entry.lock),
        }
    }
}

struct LockEntry {
    lock: Arc<AsyncMutex<()>>,
    users: usize,
}

struct LockRegistration<'a> {
    pyramid: &'a TilePyramid,
    path: PathBuf,
    lock: Arc<AsyncMutex<()>>,
}

impl Drop for LockRegistration<'_> {
    fn drop(&mut self) {
        let mut locks = lock_unpoisoned(&self.pyramid.locks);
        let Some(entry) = locks.get_mut(&self.path) else {
            return;
        };
        debug_assert!(Arc::ptr_eq(&entry.lock, &self.lock));
        entry.users -= 1;
        if entry.users == 0 {
            locks.remove(&self.path);
        }
    }
}

fn decode_for_path(path: &Path, encoded: &[u8]) -> Result<Vec<u8>, TileError> {
    decode_rgba_png(encoded).map_err(|error| match error {
        PngError::InvalidDimensions { width, height } => TileError::InvalidDimensions {
            path: path.to_owned(),
            width,
            height,
        },
        PngError::InvalidColorType { actual } => TileError::InvalidColorType {
            path: path.to_owned(),
            actual,
        },
        PngError::InvalidBitDepth { actual } => TileError::InvalidBitDepth {
            path: path.to_owned(),
            actual,
        },
        other => TileError::Decode {
            path: path.to_owned(),
            message: other.to_string(),
        },
    })
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
