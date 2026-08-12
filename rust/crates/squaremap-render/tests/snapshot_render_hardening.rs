use prost::Message;
use squaremap_render::{Limits, Registry, Snapshot, SnapshotError};
use squaremap_render::registry::{MAX_BIOME_DESCRIPTORS, MAX_BLOCK_DESCRIPTORS};
use squaremap_protocol::wire::{BiomeDescriptor, BlockStateDescriptor, ChunkSnapshot as Wire, RegistryReplace, WorldIdentity};

fn fixture_wire() -> Wire {
    Wire::decode(&include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..]).unwrap()
}

fn fixture_registry() -> RegistryReplace {
    RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap()
}

fn matching_registry() -> Registry {
    let mut replace = fixture_registry();
    replace.world = Some(WorldIdentity { namespace: "minecraft".into(), value: "overworld".into(), epoch: 3, ..Default::default() });
    replace.revision = fixture_wire().revision;
    Registry::with_replace(replace).unwrap()
}

fn body(wire: &Wire) -> squaremap_protocol::wire::ChunkSnapshotBody {
    let bytes = zstd::stream::decode_all(wire.compressed_body.as_slice()).unwrap();
    squaremap_protocol::wire::ChunkSnapshotBody::decode(bytes.as_slice()).unwrap()
}

fn rebuilt(wire: &Wire, body: &squaremap_protocol::wire::ChunkSnapshotBody, registry: &Registry) -> Result<Snapshot, SnapshotError> {
    let bytes = body.encode_to_vec();
    let mut wire = wire.clone();
    wire.compressed_body = zstd::stream::encode_all(bytes.as_slice(), 3).unwrap();
    wire.uncompressed_length = bytes.len() as u32;
    wire.crc32c = crc32c::crc32c(&bytes);
    Snapshot::decode(&wire, registry, Limits::default())
}

fn pack(values: &[u16], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; (values.len() * width + 7) / 8];
    for (index, value) in values.iter().copied().enumerate() {
        let offset = index * width;
        for bit in 0..width {
            if value & (1u16 << bit) != 0 {
                out[(offset + bit) >> 3] |= 1 << ((offset + bit) & 7);
            }
        }
    }
    out
}

#[test]
fn block_palette_up_to_4096_is_accepted() {
    let wire = fixture_wire();
    let mut replace = fixture_registry();
    replace.world = Some(WorldIdentity { namespace: "minecraft".into(), value: "overworld".into(), epoch: 3, ..Default::default() });
    replace.revision = wire.revision;
    replace.block_states = (1..=4096).map(|id| BlockStateDescriptor { id, ..Default::default() }).collect();
    let registry = Registry::with_replace(replace).unwrap();
    let mut snapshot_body = body(&wire);
    snapshot_body.sections[0].block_palette = (1..=4096).collect();
    snapshot_body.sections[0].block_indices = pack(&(0..4096).map(|value| value as u16).collect::<Vec<_>>(), 12);
    let decoded = rebuilt(&wire, &snapshot_body, &registry).expect("4096-entry block palette must decode");
    assert_eq!(decoded.sections[0].palette.len(), 4096);
    assert_eq!(decoded.sections[0].blocks[0], 1);
    assert_eq!(decoded.sections[0].blocks[4095], 4096);
}

#[test]
fn over_4096_block_palette_is_rejected() {
    let wire = fixture_wire();
    let registry = matching_registry();
    let mut snapshot_body = body(&wire);
    snapshot_body.sections[0].block_palette = (1..=4097).collect();
    let error = rebuilt(&wire, &snapshot_body, &registry).unwrap_err();
    assert!(matches!(error, SnapshotError::BodyStructure("palette too large")));
}

#[test]
fn registry_mismatch_world_is_rejected() {
    let wire = fixture_wire();
    let mut replace = fixture_registry();
    replace.world = Some(WorldIdentity { namespace: "minecraft".into(), value: "the_end".into(), epoch: 3, ..Default::default() });
    replace.revision = wire.revision;
    let error = Snapshot::decode(&wire, &Registry::with_replace(replace).unwrap(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::RegistryMismatch));
}

#[test]
fn registry_mismatch_revision_is_rejected() {
    let wire = fixture_wire();
    let mut replace = fixture_registry();
    replace.world = Some(WorldIdentity { namespace: "minecraft".into(), value: "overworld".into(), epoch: 3, ..Default::default() });
    replace.revision = wire.revision + 10;
    let error = Snapshot::decode(&wire, &Registry::with_replace(replace).unwrap(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::RegistryMismatch));
}

#[test]
fn trailing_zstd_data_is_rejected() {
    let wire = fixture_wire();
    let mut appended = wire.compressed_body.clone();
    appended.extend_from_slice(&wire.compressed_body);
    let mut rebuilt_wire = wire.clone();
    rebuilt_wire.compressed_body = appended;
    let error = Snapshot::decode(&rebuilt_wire, &matching_registry(), Limits::default()).unwrap_err();
    assert!(matches!(error, SnapshotError::Zstd(_)));
}

#[test]
fn oversized_descriptor_count_is_rejected_before_allocation() {
    let mut replace = fixture_registry();
    replace.block_states = vec![BlockStateDescriptor::default(); MAX_BLOCK_DESCRIPTORS + 1];
    let error = Registry::with_replace(replace).unwrap_err();
    assert!(matches!(error, squaremap_render::RegistryError::DescriptorCount { kind: "block", .. }));

    let mut replace = fixture_registry();
    replace.biomes = vec![BiomeDescriptor::default(); MAX_BIOME_DESCRIPTORS + 1];
    let error = Registry::with_replace(replace).unwrap_err();
    assert!(matches!(error, squaremap_render::RegistryError::DescriptorCount { kind: "biome", .. }));
}
