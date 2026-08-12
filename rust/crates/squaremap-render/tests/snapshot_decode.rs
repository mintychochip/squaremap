use prost::Message;
use squaremap_render::{Limits, Registry, Snapshot, SnapshotError};
use squaremap_protocol::wire::{BiomeDescriptor, BlockStateDescriptor, BlockTransparency, ChunkSection, ChunkSnapshotBody, FluidClass, RegistryReplace};

const VALID_FIXTURE: &[u8] = include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin");
const REGISTRY_FIXTURE: &[u8] = include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin");

fn registry() -> Registry {
    let mut replace = RegistryReplace::decode(REGISTRY_FIXTURE).unwrap();
    let wire = squaremap_protocol::wire::ChunkSnapshot::decode(VALID_FIXTURE).unwrap();
    replace.world = wire.world.clone();
    replace.revision = wire.revision;
    Registry::with_replace(replace).unwrap()
}

#[test]
fn valid_java_fixture_decodes_identically() {
    let snapshot = Snapshot::decode_bytes(VALID_FIXTURE, &registry(), Limits::default()).expect("valid fixture decodes");
    assert_eq!(snapshot.min_y, -64);
    assert_eq!(snapshot.max_y, 319);
    assert_eq!(snapshot.coordinate.x, -7);
    assert_eq!(snapshot.coordinate.z, 5);
    assert_eq!(snapshot.sections[0].section_y, -4);
    assert_eq!(snapshot.sections[0].blocks[0], 1);
    assert_eq!(snapshot.sections[0].blocks[1], 2);
    assert_eq!(snapshot.sections[0].blocks[2], 3);
    assert_eq!(snapshot.sections[0].blocks[21], 2);
    assert_eq!(snapshot.sections[0].blocks[22], 3);
    assert_eq!(snapshot.sections[0].biomes[0], 10);
    assert_eq!(snapshot.sections[0].biomes[1], 11);
    assert_eq!(snapshot.sections[0].biomes[2], 12);

    assert_eq!(snapshot.sections[0].biomes[3], 13);
    assert_eq!(snapshot.surface.heightmap.len(), 256);
    let fixture_registry = RegistryReplace::decode(REGISTRY_FIXTURE).unwrap();
    let air = fixture_registry.block_states.iter().find(|descriptor| descriptor.id == 1).expect("fixture air descriptor");
    assert_eq!(air.map_color, 0);
    assert_eq!(air.transparency, BlockTransparency::Invisible as i32);
    assert_eq!(air.fluid, FluidClass::None as i32);
    assert!(!air.glass);
    let water = fixture_registry.block_states.iter().find(|descriptor| descriptor.id == 2).expect("fixture water descriptor");
    assert_eq!(water.fluid, FluidClass::Water as i32);
    let glass = fixture_registry.block_states.iter().find(|descriptor| descriptor.id == 3).expect("fixture glass descriptor");
    assert!(glass.glass);
    assert_eq!(glass.glass_alpha_percent, 25);
    assert_eq!(snapshot.sections[1].palette, vec![1]);
    assert_eq!(snapshot.sections[1].blocks[0], 1);
}
#[test]
fn overwidth_top_level_scalar_is_rejected_before_prost_cast() {
    let mut bytes = VALID_FIXTURE.to_vec();
    bytes.extend_from_slice(&[0x18, 0x80, 0x80, 0x80, 0x80, 0x10]);
    let error = Snapshot::decode_bytes(&bytes, &registry(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::BodyStructure("scalar exceeds declared 32-bit width")));
}

#[test]
fn unknown_descriptor_is_rejected() {
    let error = Snapshot::decode_bytes(VALID_FIXTURE, &registry_without_descriptors(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::UnknownDescriptor { .. }));
}

fn registry_without_descriptors() -> Registry {
    let mut replace = RegistryReplace::decode(REGISTRY_FIXTURE).unwrap();
    let wire = squaremap_protocol::wire::ChunkSnapshot::decode(VALID_FIXTURE).unwrap();
    replace.world = wire.world;
    replace.revision = wire.revision;
    replace.block_states.clear();
    replace.biomes.clear();
    Registry::with_replace(replace).unwrap()
}

#[test]
fn oversized_section_count_is_rejected() {
    let wire = wire();
    let mut body = body(&wire);
    for _ in 0..64 { body.sections.push(ChunkSection::default()); }
    let error = rebuilt(&wire, &body, &registry()).unwrap_err();
    assert!(matches!(error, SnapshotError::SectionCountMismatch { .. }));
}

#[test]
fn invalid_packed_width_is_rejected() {
    let wire = wire();
    let mut body = body(&wire);
    body.sections[0].block_indices = vec![0];
    let error = rebuilt(&wire, &body, &registry()).unwrap_err();
    assert!(matches!(error, SnapshotError::InvalidPacking { .. }));
}

#[test]
fn repeated_palette_is_rejected_before_prost_materialization() {
    let wire = wire();
    let mut body = body(&wire);
    body.sections[0].block_palette = (1..=4097).collect();
    let error = rebuilt(&wire, &body, &registry()).unwrap_err();
    assert!(matches!(error, SnapshotError::BodyStructure("palette too large")));
}

#[test]
fn crc_mismatch_is_rejected() {
    let mut wire = wire();
    wire.crc32c ^= 1;
    let error = Snapshot::decode(&wire, &registry(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::Crc { .. }));
}

#[test]
fn decompression_bomb_is_rejected() {
    let mut wire = wire();
    wire.uncompressed_length = u32::MAX;
    let error = Snapshot::decode(&wire, &registry(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::DecompressionLimit { .. }));
}

fn wire() -> squaremap_protocol::wire::ChunkSnapshot { squaremap_protocol::wire::ChunkSnapshot::decode(VALID_FIXTURE).unwrap() }
fn body(wire: &squaremap_protocol::wire::ChunkSnapshot) -> ChunkSnapshotBody {
    let bytes = zstd::stream::decode_all(wire.compressed_body.as_slice()).unwrap();
    ChunkSnapshotBody::decode(bytes.as_slice()).unwrap()
}
fn rebuilt(wire: &squaremap_protocol::wire::ChunkSnapshot, body: &ChunkSnapshotBody, registry: &Registry) -> Result<Snapshot, SnapshotError> {
    let bytes = body.encode_to_vec();
    let mut wire = wire.clone();
    wire.compressed_body = zstd::stream::encode_all(bytes.as_slice(), 3).unwrap();
    wire.uncompressed_length = bytes.len() as u32;
    wire.crc32c = crc32c::crc32c(&bytes);
    Snapshot::decode(&wire, registry, Limits::default())
}
