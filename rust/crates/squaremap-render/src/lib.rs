//! Minecraft-independent map rendering: registry descriptors and chunk snapshots.

#![forbid(unsafe_code)]

pub mod registry;
pub mod snapshot;

pub use registry::{Registry, RegistryError};
pub use snapshot::{Limits, Snapshot, SnapshotError, Section, SurfaceHeightmap};
