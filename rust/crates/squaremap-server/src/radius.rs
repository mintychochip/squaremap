//! Paper `/radiusrender` uses block center and block radius (`n >> 4` / Euclidean chunk).

use squaremap_render::coordinates::block_to_chunk;
use squaremap_render::visibility::VisibilityLimit;
use squaremap_state::ChunkCoordinate;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RadiusError {
    InvalidRadius,
}

/// Converts a Paper block-space radius command into the chunk set Java would schedule.
pub fn chunks_from_blocks(
    center_block_x: i32,
    center_block_z: i32,
    radius_blocks: i32,
    visibility: Option<&VisibilityLimit>,
) -> Result<Vec<ChunkCoordinate>, RadiusError> {
    if radius_blocks < 1 {
        return Err(RadiusError::InvalidRadius);
    }
    let radius = block_to_chunk(radius_blocks);
    let center_x = block_to_chunk(center_block_x);
    let center_z = block_to_chunk(center_block_z);
    let mut chunks = Vec::new();
    for x in (center_x - radius)..=(center_x + radius) {
        for z in (center_z - radius)..=(center_z + radius) {
            if visibility.is_none_or(|limit| limit.contains_chunk(x, z)) {
                chunks.push(ChunkCoordinate { x, z });
            }
        }
    }
    Ok(chunks)
}
