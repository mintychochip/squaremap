use crate::registry::{GenerationToken, Registry};
use prost::Message;
use squaremap_protocol::wire::{ChunkSnapshot as WireSnapshot, ChunkSnapshotBody};
use std::fmt;
use std::io::Read;
use std::sync::Arc;

const BLOCK_ENTRIES: usize = 4096;
const BIOME_ENTRIES: usize = 64;
const HEIGHTMAP_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_snapshot_bytes: usize,
    pub max_uncompressed_bytes: usize,
    pub max_ratio: u64,
    pub min_y: i32,
    pub max_y: i32,
    pub max_abs_chunk_coordinate: i32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_snapshot_bytes: 67_108_864,
            max_uncompressed_bytes: 134_217_728,
            max_ratio: 4096,
            min_y: -2032,
            max_y: 2031,
            max_abs_chunk_coordinate: 1 << 22,
        }
    }
}

#[derive(Debug)]
pub enum SnapshotError {
    Protobuf(String),
    EmptyCompressedBody,
    DecompressionLimit {
        declared: u32,
        max: usize,
    },
    RatioLimit {
        declared: u32,
        compressed: usize,
    },
    Zstd(String),
    DecompressedLength {
        expected: usize,
        actual: usize,
    },
    Crc {
        expected: u32,
        actual: u32,
    },
    CoordinateRange,
    VerticalBounds,
    RegistryMismatch,
    SectionCountMismatch {
        expected: usize,
        actual: usize,
    },
    SectionYOutOfRange {
        section_y: i32,
    },
    InvalidPacking {
        kind: &'static str,
        expected: usize,
        actual: usize,
    },
    UnknownDescriptor {
        kind: &'static str,
        id: u32,
    },
    HeightmapLength {
        actual: usize,
    },
    HeightOutOfRange {
        value: i32,
    },
    DuplicatePalette {
        kind: &'static str,
    },
    PaletteLength {
        kind: &'static str,
    },
    BodyStructure(&'static str),
}
impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SnapshotError {}

#[derive(Debug)]
pub struct Snapshot {
    pub world: World,
    pub coordinate: Coordinate,
    pub min_y: i32,
    pub max_y: i32,
    pub ceiling: bool,
    pub revision: u64,
    pub sections: Vec<Section>,
    pub surface: SurfaceHeightmap,
    pub registry_generation: GenerationToken,
}

#[derive(Debug)]
pub struct World {
    pub namespace: String,
    pub value: String,
    pub epoch: u64,
}
#[derive(Debug)]
pub struct Coordinate {
    pub x: i32,
    pub z: i32,
}
#[derive(Debug)]
pub struct Section {
    pub section_y: i32,
    pub palette: Vec<u32>,
    pub blocks: Vec<u32>,
    pub biome_palette: Vec<u32>,
    pub biomes: Vec<u32>,
}
#[derive(Debug)]
pub struct SurfaceHeightmap {
    pub heightmap: Vec<i32>,
}

impl Snapshot {
    pub fn decode(
        wire: &WireSnapshot,
        registry: &Registry,
        limits: Limits,
    ) -> Result<Self, SnapshotError> {
        let generation = registry.generation();
        Self::decode_for(wire, &generation, limits)
    }
    pub fn decode_for(
        wire: &WireSnapshot,
        generation: &GenerationToken,
        limits: Limits,
    ) -> Result<Self, SnapshotError> {
        Self::decode_generation(wire, generation, limits, true)
    }

    /// Live dirty events retag the same palette with a new request revision.
    /// Bind on world identity; do not require generation.revision == snapshot.revision.
    pub fn decode_for_live(
        wire: &WireSnapshot,
        generation: &GenerationToken,
        limits: Limits,
    ) -> Result<Self, SnapshotError> {
        Self::decode_generation(wire, generation, limits, false)
    }
    pub fn decode_bytes(
        bytes: &[u8],
        registry: &Registry,
        limits: Limits,
    ) -> Result<Self, SnapshotError> {
        if bytes.len() > limits.max_snapshot_bytes.saturating_add(65_536) {
            return Err(SnapshotError::DecompressionLimit {
                declared: bytes.len().min(u32::MAX as usize) as u32,
                max: limits.max_snapshot_bytes,
            });
        }
        preflight_snapshot(bytes)?;
        let wire =
            WireSnapshot::decode(bytes).map_err(|e| SnapshotError::Protobuf(e.to_string()))?;
        Self::decode(&wire, registry, limits)
    }
    fn decode_generation(
        wire: &WireSnapshot,
        generation: &GenerationToken,
        limits: Limits,
        match_revision: bool,
    ) -> Result<Self, SnapshotError> {
        let world = wire
            .world
            .as_ref()
            .ok_or_else(|| SnapshotError::Protobuf("missing world".into()))?;
        let coordinate = wire
            .coordinate
            .as_ref()
            .ok_or_else(|| SnapshotError::Protobuf("missing coordinate".into()))?;
        let registry_world = generation.world().ok_or(SnapshotError::RegistryMismatch)?;
        if (match_revision && generation.revision() != wire.revision)
            || registry_world.namespace != world.namespace
            || registry_world.value != world.value
            || registry_world.epoch != world.epoch
        {
            return Err(SnapshotError::RegistryMismatch);
        }
        if coordinate.x.unsigned_abs() > limits.max_abs_chunk_coordinate.unsigned_abs()
            || coordinate.z.unsigned_abs() > limits.max_abs_chunk_coordinate.unsigned_abs()
        {
            return Err(SnapshotError::CoordinateRange);
        }
        let span = i64::from(wire.max_y) - i64::from(wire.min_y) + 1;
        if span <= 0 || wire.min_y < limits.min_y || wire.max_y > limits.max_y || span % 16 != 0 {
            return Err(SnapshotError::VerticalBounds);
        }
        let compressed = &wire.compressed_body;
        if compressed.is_empty() {
            return Err(SnapshotError::EmptyCompressedBody);
        }
        if compressed.len() > limits.max_snapshot_bytes {
            return Err(SnapshotError::DecompressionLimit {
                declared: compressed.len().min(u32::MAX as usize) as u32,
                max: limits.max_snapshot_bytes,
            });
        }
        let declared = wire.uncompressed_length as usize;
        if declared > limits.max_uncompressed_bytes {
            return Err(SnapshotError::DecompressionLimit {
                declared: wire.uncompressed_length,
                max: limits.max_uncompressed_bytes,
            });
        }
        if declared as u64 > (compressed.len() as u64).saturating_mul(limits.max_ratio) {
            return Err(SnapshotError::RatioLimit {
                declared: wire.uncompressed_length,
                compressed: compressed.len(),
            });
        }
        let mut decoder = zstd::stream::read::Decoder::new(compressed.as_slice())
            .map_err(|e| SnapshotError::Zstd(e.to_string()))?
            .single_frame();
        decoder
            .window_log_max(squaremap_protocol::FrameLimits::MAX_ZSTD_WINDOW_LOG)
            .map_err(|e| SnapshotError::Zstd(e.to_string()))?;
        let mut uncompressed = Vec::with_capacity(declared);
        let mut chunk = [0u8; 8192];
        loop {
            let n = decoder
                .read(&mut chunk)
                .map_err(|e| SnapshotError::Zstd(e.to_string()))?;
            if n == 0 {
                break;
            }
            if uncompressed.len() + n > declared {
                return Err(SnapshotError::DecompressedLength {
                    expected: declared,
                    actual: uncompressed.len() + n,
                });
            }
            uncompressed.extend_from_slice(&chunk[..n]);
        }
        let buffered_remaining = decoder.get_ref().buffer().len();
        let inner_remaining = decoder.get_ref().get_ref().len();
        if buffered_remaining != 0 || inner_remaining != 0 {
            return Err(SnapshotError::Zstd(
                "body contains trailing zstd data".into(),
            ));
        }
        let _ = decoder.finish();
        if uncompressed.len() != declared {
            return Err(SnapshotError::DecompressedLength {
                expected: declared,
                actual: uncompressed.len(),
            });
        }
        let actual = crc32c::crc32c(&uncompressed);
        if actual != wire.crc32c {
            return Err(SnapshotError::Crc {
                expected: wire.crc32c,
                actual,
            });
        }
        let expected_sections =
            usize::try_from(span / 16).map_err(|_| SnapshotError::VerticalBounds)?;
        preflight_body(&uncompressed, expected_sections)?;
        let body = ChunkSnapshotBody::decode(uncompressed.as_slice())
            .map_err(|e| SnapshotError::Protobuf(e.to_string()))?;
        if body.sections.len() != expected_sections {
            return Err(SnapshotError::SectionCountMismatch {
                expected: expected_sections,
                actual: body.sections.len(),
            });
        }
        let heights_field = body
            .surface_heightmap
            .as_ref()
            .ok_or(SnapshotError::HeightmapLength { actual: 0 })?;
        if heights_field.heights.len() != HEIGHTMAP_ENTRIES {
            return Err(SnapshotError::HeightmapLength {
                actual: heights_field.heights.len(),
            });
        }
        let max_height = i64::from(wire.max_y) + 1;
        let heights = heights_field.heights.clone();
        if let Some(value) = heights.iter().find(|value| {
            i64::from(**value) < i64::from(wire.min_y) || i64::from(**value) > max_height
        }) {
            return Err(SnapshotError::HeightOutOfRange { value: *value });
        }
        let mut sections = Vec::with_capacity(expected_sections);
        for (index, section) in body.sections.iter().enumerate() {
            let expected_y = i64::from(wire.min_y).div_euclid(16) + index as i64;
            if i64::from(section.section_y) != expected_y {
                return Err(SnapshotError::SectionYOutOfRange {
                    section_y: section.section_y,
                });
            }
            let blocks = decode_palette(
                "block",
                &section.block_palette,
                &section.block_indices,
                BLOCK_ENTRIES,
                generation,
                true,
            )?;
            let biomes = decode_palette(
                "biome",
                &section.biome_palette,
                &section.biome_indices,
                BIOME_ENTRIES,
                generation,
                false,
            )?;
            sections.push(Section {
                section_y: section.section_y,
                palette: section.block_palette.clone(),
                blocks,
                biome_palette: section.biome_palette.clone(),
                biomes,
            });
        }
        Ok(Self {
            world: World {
                namespace: world.namespace.clone(),
                value: world.value.clone(),
                epoch: world.epoch,
            },
            coordinate: Coordinate {
                x: coordinate.x,
                z: coordinate.z,
            },
            min_y: wire.min_y,
            max_y: wire.max_y,
            ceiling: wire.ceiling,
            revision: wire.revision,
            sections,
            surface: SurfaceHeightmap { heightmap: heights },
            registry_generation: Arc::clone(generation),
        })
    }
}
fn preflight_snapshot(bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut offset = 0;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        let field = key >> 3;
        let wire = (key & 7) as u8;
        match field {
            1 | 2 if wire == 2 => {
                let nested = read_bytes(bytes, &mut offset)?;
                if field == 2 {
                    preflight_coordinate(nested)?;
                }
            }
            3 | 4 if wire == 0 => {
                checked_scalar(&read_varint(bytes, &mut offset)?)?;
            }
            5 if wire == 0 => {
                if read_varint(bytes, &mut offset)? > 1 {
                    return Err(SnapshotError::BodyStructure(
                        "boolean exceeds declared width",
                    ));
                }
            }
            6 if wire == 0 => {
                read_varint(bytes, &mut offset)?;
            }
            7 | 8 if wire == 0 => {
                checked_scalar(&read_varint(bytes, &mut offset)?)?;
            }
            9 if wire == 2 => {
                read_bytes(bytes, &mut offset)?;
            }
            _ => skip_value(bytes, &mut offset, wire)?,
        }
    }
    Ok(())
}

fn preflight_coordinate(bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut offset = 0;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        let field = key >> 3;
        let wire = (key & 7) as u8;
        if (field == 1 || field == 2) && wire == 0 {
            checked_scalar(&read_varint(bytes, &mut offset)?)?;
        } else {
            skip_value(bytes, &mut offset, wire)?;
        }
    }
    Ok(())
}

fn preflight_body(bytes: &[u8], max_sections: usize) -> Result<(), SnapshotError> {
    let mut offset = 0;
    let mut sections = 0usize;
    let mut heightmaps = 0usize;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        let field = key >> 3;
        let wire = (key & 7) as u8;
        match field {
            1 if wire == 2 => {
                sections = sections
                    .checked_add(1)
                    .ok_or(SnapshotError::BodyStructure("section count overflow"))?;
                if sections > max_sections {
                    return Err(SnapshotError::SectionCountMismatch {
                        expected: max_sections,
                        actual: sections,
                    });
                }
                let nested = read_bytes(bytes, &mut offset)?;
                preflight_section(nested)?;
            }
            2 if wire == 2 => {
                heightmaps = heightmaps
                    .checked_add(1)
                    .ok_or(SnapshotError::BodyStructure("heightmap count overflow"))?;
                if heightmaps > 1 {
                    return Err(SnapshotError::BodyStructure("multiple heightmaps"));
                }
                preflight_heightmap(read_bytes(bytes, &mut offset)?)?;
            }
            _ => skip_value(bytes, &mut offset, wire)?,
        }
    }
    Ok(())
}

fn preflight_section(bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut offset = 0;
    let mut block_palette = 0usize;
    let mut biome_palette = 0usize;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        let field = key >> 3;
        let wire = (key & 7) as u8;
        match field {
            1 if wire == 0 => {
                checked_scalar(&read_varint(bytes, &mut offset)?)?;
            }
            2 => {
                block_palette = checked_repeated_u32(bytes, &mut offset, wire, block_palette, 4096)?
            }
            3 if wire == 2 => {
                if read_bytes(bytes, &mut offset)?.len() > BLOCK_ENTRIES * 8 {
                    return Err(SnapshotError::BodyStructure("block indices too large"));
                }
            }
            4 => biome_palette = checked_repeated_u32(bytes, &mut offset, wire, biome_palette, 64)?,
            5 if wire == 2 => {
                if read_bytes(bytes, &mut offset)?.len() > BIOME_ENTRIES * 8 {
                    return Err(SnapshotError::BodyStructure("biome indices too large"));
                }
            }
            _ => skip_value(bytes, &mut offset, wire)?,
        }
    }
    Ok(())
}

fn preflight_heightmap(bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut offset = 0;
    let mut count = 0usize;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        let field = key >> 3;
        let wire = (key & 7) as u8;
        if field == 1 {
            count = checked_repeated_i32(bytes, &mut offset, wire, count)?;
            if count > HEIGHTMAP_ENTRIES {
                return Err(SnapshotError::BodyStructure("heightmap too large"));
            }
        } else {
            skip_value(bytes, &mut offset, wire)?;
        }
    }
    Ok(())
}

fn checked_repeated_u32(
    bytes: &[u8],
    offset: &mut usize,
    wire: u8,
    current: usize,
    max: usize,
) -> Result<usize, SnapshotError> {
    let mut count = 0usize;
    match wire {
        0 => {
            checked_scalar(&read_varint(bytes, offset)?)?;
            count = 1;
        }
        2 => {
            let packed = read_bytes(bytes, offset)?;
            let mut inner = 0usize;
            while inner < packed.len() {
                checked_scalar(&read_varint(packed, &mut inner)?)?;
                count = count
                    .checked_add(1)
                    .ok_or(SnapshotError::BodyStructure("palette count overflow"))?;
            }
        }
        _ => {
            return Err(SnapshotError::BodyStructure(
                "invalid repeated scalar wire type",
            ));
        }
    }
    let total = current
        .checked_add(count)
        .ok_or(SnapshotError::BodyStructure("palette count overflow"))?;
    if total > max {
        return Err(SnapshotError::BodyStructure("palette too large"));
    }
    Ok(total)
}

fn checked_scalar(value: &u64) -> Result<(), SnapshotError> {
    if *value > u64::from(u32::MAX) {
        return Err(SnapshotError::BodyStructure(
            "scalar exceeds declared 32-bit width",
        ));
    }
    Ok(())
}

fn checked_repeated_i32(
    bytes: &[u8],
    offset: &mut usize,
    wire: u8,
    current: usize,
) -> Result<usize, SnapshotError> {
    let mut count = 0usize;
    match wire {
        0 => {
            checked_scalar(&read_varint(bytes, offset)?)?;
            count = 1;
        }
        2 => {
            let packed = read_bytes(bytes, offset)?;
            let mut inner = 0usize;
            while inner < packed.len() {
                checked_scalar(&read_varint(packed, &mut inner)?)?;
                count = count
                    .checked_add(1)
                    .ok_or(SnapshotError::BodyStructure("heightmap count overflow"))?;
            }
        }
        _ => return Err(SnapshotError::BodyStructure("invalid heightmap wire type")),
    }
    current
        .checked_add(count)
        .ok_or(SnapshotError::BodyStructure("heightmap count overflow"))
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> Result<u64, SnapshotError> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        if *offset >= bytes.len() {
            return Err(SnapshotError::BodyStructure("truncated varint"));
        }
        let byte = bytes[*offset];
        *offset += 1;
        if shift == 63 && byte > 1 {
            return Err(SnapshotError::BodyStructure("varint overflow"));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        if shift == 63 {
            return Err(SnapshotError::BodyStructure("varint overflow"));
        }
    }
    Err(SnapshotError::BodyStructure("varint overflow"))
}

fn read_bytes<'a>(bytes: &'a [u8], offset: &mut usize) -> Result<&'a [u8], SnapshotError> {
    let length = usize::try_from(read_varint(bytes, offset)?)
        .map_err(|_| SnapshotError::BodyStructure("length overflow"))?;
    let end = (*offset)
        .checked_add(length)
        .ok_or(SnapshotError::BodyStructure("length overflow"))?;
    if end > bytes.len() {
        return Err(SnapshotError::BodyStructure("length exceeds body"));
    }
    let result = &bytes[*offset..end];
    *offset = end;
    Ok(result)
}

fn skip_value(bytes: &[u8], offset: &mut usize, wire: u8) -> Result<(), SnapshotError> {
    match wire {
        0 => {
            read_varint(bytes, offset)?;
            Ok(())
        }
        1 => {
            let end = (*offset)
                .checked_add(8)
                .ok_or(SnapshotError::BodyStructure("fixed width overflow"))?;
            if end > bytes.len() {
                return Err(SnapshotError::BodyStructure("truncated fixed width"));
            }
            *offset = end;
            Ok(())
        }
        2 => read_bytes(bytes, offset).map(|_| ()),
        5 => {
            let end = (*offset)
                .checked_add(4)
                .ok_or(SnapshotError::BodyStructure("fixed width overflow"))?;
            if end > bytes.len() {
                return Err(SnapshotError::BodyStructure("truncated fixed width"));
            }
            *offset = end;
            Ok(())
        }
        _ => Err(SnapshotError::BodyStructure("unsupported wire type")),
    }
}

fn decode_palette(
    kind: &'static str,
    palette: &[u32],
    bytes: &[u8],
    entries: usize,
    generation: &GenerationToken,
    block: bool,
) -> Result<Vec<u32>, SnapshotError> {
    let max_palette = if block { 4096 } else { 64 };
    if palette.is_empty() || palette.len() > max_palette {
        return Err(SnapshotError::PaletteLength { kind });
    }
    let mut seen = std::collections::HashSet::with_capacity(palette.len());
    for id in palette {
        if !seen.insert(*id) {
            return Err(SnapshotError::DuplicatePalette { kind });
        }
        if (block && generation.block(*id).is_none()) || (!block && generation.biome(*id).is_none())
        {
            return Err(SnapshotError::UnknownDescriptor { kind, id: *id });
        }
    }
    let bits = if palette.len() == 1 {
        0
    } else {
        (usize::BITS - (palette.len() - 1).leading_zeros()) as usize
    };
    let expected = (entries * bits + 7) / 8;
    if bytes.len() != expected {
        return Err(SnapshotError::InvalidPacking {
            kind,
            expected,
            actual: bytes.len(),
        });
    }
    if bits == 0 {
        return Ok(vec![palette[0]; entries]);
    }
    let used_bits = entries * bits;
    let trailing = bytes.len() * 8 - used_bits;
    if trailing > 0
        && bytes
            .last()
            .is_some_and(|last| (*last & (0xff << (8 - trailing))) != 0)
    {
        return Err(SnapshotError::InvalidPacking {
            kind,
            expected: used_bits,
            actual: bytes.len(),
        });
    }
    let mut out = Vec::with_capacity(entries);
    for i in 0..entries {
        let offset = i * bits;
        let mut value = 0u32;
        for b in 0..bits {
            if (bytes[(offset + b) >> 3] & (1 << ((offset + b) & 7))) != 0 {
                value |= 1 << b;
            }
        }
        let index = value as usize;
        if index >= palette.len() {
            return Err(SnapshotError::InvalidPacking {
                kind,
                expected: palette.len(),
                actual: index,
            });
        }
        out.push(palette[index]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use squaremap_protocol::wire::{BlockStateDescriptor, RegistryReplace};

    #[test]
    fn rejects_nonzero_unused_packed_bits() {
        let registry = Registry::with_replace(RegistryReplace {
            block_states: vec![
                BlockStateDescriptor {
                    id: 1,
                    transparency: 1,
                    fluid: 1,
                    ..Default::default()
                },
                BlockStateDescriptor {
                    id: 2,
                    transparency: 1,
                    fluid: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
        let generation = registry.generation();
        let error =
            decode_palette("block", &[1, 2], &[0b1000_0000], 3, &generation, true).unwrap_err();
        assert!(matches!(error, SnapshotError::InvalidPacking { .. }));
    }
    #[test]
    fn rejects_overwidth_palette_scalar() {
        let bytes = [0x12, 0x06, 0x81, 0x80, 0x80, 0x80, 0x80, 0x01];
        let error = preflight_section(&bytes).unwrap_err();
        assert!(matches!(
            error,
            SnapshotError::BodyStructure("scalar exceeds declared 32-bit width")
        ));
    }
}
