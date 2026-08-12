//! Minecraft-independent map rendering: registry descriptors and chunk snapshots.

#![forbid(unsafe_code)]

pub mod biome;
pub mod chunk;
pub mod color;
pub mod coordinates;
pub mod region;
pub mod registry;
pub mod snapshot;
pub mod visibility;

pub use biome::{
    BiomeSource, BiomeSourceError, QuartBiomeSource, SnapshotBiomeSource, StaticBiomeSource,
};
pub use chunk::{
    ChunkPixels, NeighborDirection, RenderContext, RenderContextError, RenderError, RenderSettings,
    render_chunk, validate_neighbor_relation,
};
pub use registry::{GenerationToken, Registry, RegistryError, RegistryGeneration};
pub use snapshot::{Limits, Section, Snapshot, SnapshotError, SurfaceHeightmap};
