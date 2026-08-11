#![forbid(unsafe_code)]

/// Generated Protobuf types for the versioned bridge contract.
pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/squaremap.bridge.v1.rs"));
}

mod frame;
mod limits;

pub use frame::{
    read_envelope, write_envelope, FrameClass, FrameError, SnapshotValidationReason,
};
pub use limits::FrameLimits;
pub use wire::{
    envelope, ChunkSection, ChunkSnapshot, ChunkSnapshotBody, Envelope, Heightmap,
};
