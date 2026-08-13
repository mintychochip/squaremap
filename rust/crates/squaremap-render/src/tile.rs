use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

pub const TILE_SIZE: usize = 512;
pub const TILE_RGBA_BYTES: usize = TILE_SIZE * TILE_SIZE * 4;
pub const MAX_ENCODED_TILE_BYTES: u64 = 8 * 1024 * 1024;
const PRESENT_WORDS: usize = TILE_SIZE * TILE_SIZE / 64;

#[derive(Clone, Debug)]
pub struct RegionPixels {
    rgba: Box<[u8]>,
    present: Box<[u64]>,
}

impl RegionPixels {
    pub fn empty() -> Self {
        Self {
            rgba: vec![0; TILE_RGBA_BYTES].into_boxed_slice(),
            present: vec![0; PRESENT_WORDS].into_boxed_slice(),
        }
    }

    pub fn full_rgba(color: [u8; 4]) -> Self {
        let mut pixels = Self::empty();
        for value in pixels.rgba.chunks_exact_mut(4) {
            value.copy_from_slice(&color);
        }
        pixels.present.fill(u64::MAX);
        pixels
    }

    pub fn set_rgba(&mut self, x: usize, z: usize, color: [u8; 4]) -> Result<(), TileError> {
        let index = pixel_index(x, z)?;
        self.rgba[index * 4..index * 4 + 4].copy_from_slice(&color);
        self.present[index / 64] |= 1_u64 << (index % 64);
        Ok(())
    }

    pub fn set_argb(&mut self, x: usize, z: usize, argb: u32) -> Result<(), TileError> {
        self.set_rgba(
            x,
            z,
            [
                (argb >> 16) as u8,
                (argb >> 8) as u8,
                argb as u8,
                (argb >> 24) as u8,
            ],
        )
    }

    pub(crate) fn sample(&self, x: usize, z: usize) -> Option<[u8; 4]> {
        let index = z * TILE_SIZE + x;
        if self.present[index / 64] & (1_u64 << (index % 64)) == 0 {
            return None;
        }
        Some(
            self.rgba[index * 4..index * 4 + 4]
                .try_into()
                .expect("four-byte pixel"),
        )
    }
}

fn pixel_index(x: usize, z: usize) -> Result<usize, TileError> {
    if x >= TILE_SIZE || z >= TILE_SIZE {
        Err(TileError::PixelCoordinate { x, z })
    } else {
        Ok(z * TILE_SIZE + x)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TileError {
    InvalidZoom(u8),
    PixelCoordinate {
        x: usize,
        z: usize,
    },
    InvalidRgbaLength {
        actual: usize,
    },
    Decode {
        path: PathBuf,
        message: String,
    },
    InvalidDimensions {
        path: PathBuf,
        width: u32,
        height: u32,
    },
    InvalidColorType {
        path: PathBuf,
        actual: String,
    },
    InvalidBitDepth {
        path: PathBuf,
        actual: String,
    },
    Encode {
        path: PathBuf,
        message: String,
    },
    Store {
        path: PathBuf,
        message: String,
    },
    Publish {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for TileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidZoom(zoom) => write!(formatter, "invalid tile zoom {zoom}; maximum is 9"),
            Self::PixelCoordinate { x, z } => write!(
                formatter,
                "pixel coordinate ({x}, {z}) is outside a 512x512 region"
            ),
            Self::InvalidRgbaLength { actual } => write!(
                formatter,
                "RGBA tile has {actual} bytes; expected {TILE_RGBA_BYTES}"
            ),
            Self::Decode { path, message } => {
                write!(formatter, "could not decode {}: {message}", path.display())
            }
            Self::InvalidDimensions {
                path,
                width,
                height,
            } => write!(
                formatter,
                "{} is {width}x{height}; expected 512x512",
                path.display()
            ),
            Self::InvalidColorType { path, actual } => write!(
                formatter,
                "{} has color type {actual}; expected Rgba",
                path.display()
            ),
            Self::InvalidBitDepth { path, actual } => write!(
                formatter,
                "{} has bit depth {actual}; expected Eight",
                path.display()
            ),
            Self::Encode { path, message } => {
                write!(formatter, "could not encode {}: {message}", path.display())
            }
            Self::Store { path, message } => {
                write!(formatter, "could not read {}: {message}", path.display())
            }
            Self::Publish { path, message } => {
                write!(formatter, "could not publish {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for TileError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileStoreError {
    pub message: String,
}

impl TileStoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TileStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TileStoreError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PublishResult {
    pub warning: Option<String>,
}

#[async_trait]
pub trait TileStore: Send + Sync {
    async fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, TileStoreError>;
    async fn publish(&self, path: &Path, bytes: &[u8]) -> Result<PublishResult, TileStoreError>;
}

#[derive(Clone, Debug, Default)]
pub struct MemoryTileStore {
    state: Arc<Mutex<MemoryTileState>>,
}

#[derive(Debug, Default)]
struct MemoryTileState {
    files: HashMap<PathBuf, Vec<u8>>,
    fail_before_publish: HashSet<PathBuf>,
    warn_directory_sync: HashSet<PathBuf>,
}

impl MemoryTileStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, path: PathBuf, bytes: Vec<u8>) {
        lock_unpoisoned(&self.state).files.insert(path, bytes);
    }

    pub fn fail_next_before_publish(&self, path: PathBuf) {
        lock_unpoisoned(&self.state)
            .fail_before_publish
            .insert(path);
    }

    pub fn warn_next_directory_sync(&self, path: PathBuf) {
        lock_unpoisoned(&self.state)
            .warn_directory_sync
            .insert(path);
    }
    pub fn file_count(&self) -> usize {
        lock_unpoisoned(&self.state).files.len()
    }
}

#[async_trait]
impl TileStore for MemoryTileStore {
    async fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, TileStoreError> {
        Ok(lock_unpoisoned(&self.state).files.get(path).cloned())
    }

    async fn publish(&self, path: &Path, bytes: &[u8]) -> Result<PublishResult, TileStoreError> {
        let mut state = lock_unpoisoned(&self.state);
        if state.fail_before_publish.remove(path) {
            return Err(TileStoreError::new("injected pre-publish failure"));
        }
        state.files.insert(path.to_owned(), bytes.to_vec());
        let warning = state
            .warn_directory_sync
            .remove(path)
            .then(|| "injected directory sync failure".to_owned());
        Ok(PublishResult { warning })
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
