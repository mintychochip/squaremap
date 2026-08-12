//! Minecraft-independent map rendering: registry descriptors and chunk snapshots.

#![forbid(unsafe_code)]

pub mod registry;
pub mod snapshot;
pub mod coordinates;
pub mod visibility;
pub mod color;

pub use registry::{Registry, RegistryError};
pub use snapshot::{Limits, Snapshot, SnapshotError, Section, SurfaceHeightmap};
