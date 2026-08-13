//! Minecraft-independent map rendering: registry descriptors and chunk snapshots.

#![forbid(unsafe_code)]

pub mod biome;
pub mod chunk;
pub mod color;
pub mod coordinates;
pub mod png;
pub mod pyramid;
pub mod region;
pub mod registry;
pub mod snapshot;
pub mod tile;
pub mod fixture;
pub mod visibility;

pub use biome::{
    BiomeSource, BiomeSourceError, QuartBiomeSource, SnapshotBiomeSource, StaticBiomeSource,
};
pub use chunk::{
    ChunkPixels, NeighborDirection, RenderContext, RenderContextError, RenderError, RenderSettings,
    render_chunk, validate_neighbor_relation,
};
pub use png::{PngError, PngOptions, decode_rgba_png, encode_rgba_png};
pub use pyramid::{ApplyResult, TilePyramid, TileWarning};
pub use registry::{GenerationToken, Registry, RegistryError, RegistryGeneration};
pub use snapshot::{Limits, Section, Snapshot, SnapshotError, SurfaceHeightmap};
pub use tile::{
    MAX_ENCODED_TILE_BYTES, MemoryTileStore, PublishResult, RegionPixels, TILE_RGBA_BYTES,
    TILE_SIZE, TileError, TileStore, TileStoreError,
};
