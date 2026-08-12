//! Overflow-safe map coordinate conversions.

pub const BLOCKS_PER_CHUNK: i32 = 16;
pub const CHUNKS_PER_REGION: i32 = 32;
pub const BLOCKS_PER_REGION: i32 = 512;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CoordinateError {
    Overflow,
    InvalidZoom,
}

fn checked_mul(value: i32, factor: i32) -> Result<i32, CoordinateError> {
    i32::try_from(i64::from(value) * i64::from(factor)).map_err(|_| CoordinateError::Overflow)
}

pub fn block_to_chunk(value: i32) -> i32 { value.div_euclid(BLOCKS_PER_CHUNK) }
pub fn block_to_region(value: i32) -> i32 { value.div_euclid(BLOCKS_PER_REGION) }
pub fn chunk_to_region(value: i32) -> i32 { value.div_euclid(CHUNKS_PER_REGION) }
pub fn chunk_to_block(value: i32) -> Result<i32, CoordinateError> { checked_mul(value, BLOCKS_PER_CHUNK) }
pub fn region_to_chunk(value: i32) -> Result<i32, CoordinateError> { checked_mul(value, CHUNKS_PER_REGION) }
pub fn region_to_block(value: i32) -> Result<i32, CoordinateError> { checked_mul(value, BLOCKS_PER_REGION) }

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ChunkCoord { pub x: i32, pub z: i32 }

impl ChunkCoord {
    pub fn region(self) -> RegionCoord {
        RegionCoord { x: chunk_to_region(self.x), z: chunk_to_region(self.z) }
    }
    pub fn block_origin(self) -> Result<(i32, i32), CoordinateError> {
        Ok((chunk_to_block(self.x)?, chunk_to_block(self.z)?))
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RegionCoord { pub x: i32, pub z: i32 }

impl RegionCoord {
    pub fn region(self) -> Self { self }
    pub fn chunk_origin(self) -> Result<(i32, i32), CoordinateError> {
        Ok((region_to_chunk(self.x)?, region_to_chunk(self.z)?))
    }
    pub fn block_origin(self) -> Result<(i32, i32), CoordinateError> {
        Ok((region_to_block(self.x)?, region_to_block(self.z)?))
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TileCoord { pub level: u8, pub x: i32, pub z: i32 }

fn validate_zoom(zoom: u8, max_zoom: u8) -> Result<(), CoordinateError> {
    if zoom > max_zoom || max_zoom > 9 { Err(CoordinateError::InvalidZoom) } else { Ok(()) }
}

pub fn tile_for_region(region_x: i32, region_z: i32, zoom: u8, max_zoom: u8) -> Result<TileCoord, CoordinateError> {
    validate_zoom(zoom, max_zoom)?;
    let step = 1_i64 << zoom;
    Ok(TileCoord {
        level: max_zoom - zoom,
        x: (i64::from(region_x).div_euclid(step)) as i32,
        z: (i64::from(region_z).div_euclid(step)) as i32,
    })
}

pub fn tile_origin(region_x: i32, region_z: i32, zoom: u8) -> Result<(u16, u16), CoordinateError> {
    if zoom > 9 { return Err(CoordinateError::InvalidZoom); }
    let step = 1_i64 << zoom;
    let size = 512_i64 / step;
    let x = (i64::from(region_x) * size).rem_euclid(512);
    let z = (i64::from(region_z) * size).rem_euclid(512);
    Ok((x as u16, z as u16))
}
