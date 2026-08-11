/// Bounded sizes shared by the Java and Rust bridge codecs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLimits {
    pub max_control_bytes: u32,
    pub max_snapshot_bytes: u32,
    pub max_uncompressed_snapshot_bytes: u32,
}

impl FrameLimits {
    pub const MAX_CONTROL_BYTES: u32 = 1_048_576;
    pub const MAX_SNAPSHOT_BYTES: u32 = 67_108_864;
    pub const MAX_UNCOMPRESSED_SNAPSHOT_BYTES: u32 = 134_217_728;
    pub const MAX_SNAPSHOT_DECOMPRESSION_RATIO: u64 = 4096;
}

impl Default for FrameLimits {
    fn default() -> Self {
        Self {
            max_control_bytes: Self::MAX_CONTROL_BYTES,
            max_snapshot_bytes: Self::MAX_SNAPSHOT_BYTES,
            max_uncompressed_snapshot_bytes: Self::MAX_UNCOMPRESSED_SNAPSHOT_BYTES,
        }
    }
}
