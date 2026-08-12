#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{FixtureCorpus, render_case};
use prost::Message;
use serde_json::Value;
use squaremap_protocol::wire::ChunkSnapshot as WireSnapshot;
use squaremap_render::BiomeSource;
use std::path::{Path, PathBuf};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../testdata/bridge/v2/render")
}

#[test]
fn all_valid_cases_match_java_pixels_and_edges() {
    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    assert_eq!(corpus.cases.len(), 26);
    for case in &corpus.cases {
        let output = render_case(case).unwrap_or_else(|error| panic!("{}: {error:?}", case.row.id));
        assert_eq!(
            output.pixels, case.expected_pixels,
            "pixel mismatch for {}",
            case.row.id
        );
        assert_eq!(
            output.south_edge, case.expected_edge,
            "south edge mismatch for {}",
            case.row.id
        );
    }
}

#[test]
fn biome_grass_tuples_select_exact_ids() {
    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    for case in &corpus.cases {
        if !case.row.id.starts_with("biome-grass") {
            continue;
        }
        let source = case.biome_source.as_ref().unwrap();
        for sample in &case.row.grass_samples {
            let selected = source
                .sample_block(sample.block_x, sample.block_y, sample.block_z)
                .unwrap_or_else(|error| {
                    panic!(
                        "{} ({},{},{}): source error {error:?}",
                        case.row.id, sample.block_x, sample.block_y, sample.block_z
                    )
                });
            assert_eq!(
                selected,
                sample.biome_id,
                "{} ({},{},{}): selected {}, expected {}",
                case.row.id,
                sample.block_x,
                sample.block_y,
                sample.block_z,
                selected,
                sample.biome_id
            );
            assert_eq!(
                source.resolved_grass(sample.block_x, sample.block_y, sample.block_z, selected),
                Ok(sample.resolved_grass_argb as u32)
            );
        }
    }
}

#[test]
fn malformed_fixture_errors_are_typed() {
    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    for row in &corpus.manifest.malformed {
        let decoded = fixtures::malformed_error(row, &corpus);
        match row.id.as_str() {
            "malformed-neighbor-coordinate" => {
                let snapshot = decoded.expect("neighbor mutation must decode");
                let center = corpus
                    .cases
                    .iter()
                    .find(|case| case.row.id == "north-height-discontinuity")
                    .unwrap();
                assert!(matches!(
                    squaremap_render::validate_neighbor_relation(
                        &center.context,
                        &snapshot,
                        &center.center,
                        squaremap_render::NeighborDirection::North
                    ),
                    Err(squaremap_render::RenderError::CoordinateMismatch)
                ));
            }
            "malformed-unknown-descriptor" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::UnknownDescriptor { kind: "block", .. })
            )),
            "malformed-heightmap" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::HeightOutOfRange { .. })
            )),
            "malformed-palette" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::PaletteLength { kind: "block" })
            )),
            "malformed-bounds" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::VerticalBounds)
            )),
            "malformed-crc" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::Crc { .. })
            )),
            "malformed-length" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::DecompressedLength { .. })
            )),
            "malformed-section-count" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::SectionCountMismatch { .. })
            )),
            "malformed-section-y" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::SectionYOutOfRange { .. })
            )),
            "malformed-packed-width" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::InvalidPacking {
                    kind: "block",
                    expected: _,
                    actual
                }) if actual != 3
            )),
            "malformed-trailing-bits" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::InvalidPacking {
                    kind: "block",
                    expected: 3,
                    actual: 3
                })
            )),
            "malformed-duplicate-palette" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::DuplicatePalette { kind: "block" })
            )),
            "malformed-protobuf-body" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::Protobuf(_))
            )),
            "malformed-generation" => assert!(matches!(
                decoded,
                Err(squaremap_render::SnapshotError::RegistryMismatch)
            )),
            id => panic!("unhandled malformed case {id}"),
        }
    }
}

type JsonMutation = fn(&mut Value);

#[test]
fn manifest_mutations_fail_closed_before_rendering() {
    let mutations: &[(&str, JsonMutation, &str)] = &[
        ("schema", |v| v["schema_version"] = 9.into(), "metadata"),
        ("generator", |v| v["generator"] = "wrong".into(), "metadata"),
        ("seed", |v| v["biome_zoom_seed"] = 1.into(), "metadata"),
        (
            "valid-id",
            |v| v["valid"][0]["id"] = "unknown".into(),
            "valid ID",
        ),
        (
            "valid-missing",
            |v| {
                v["valid"].as_array_mut().unwrap().remove(0);
            },
            "count",
        ),
        (
            "valid-duplicate",
            |v| v["valid"][1]["id"] = v["valid"][0]["id"].clone(),
            "valid ID",
        ),
        ("valid-reordered", swap_valid_rows, "valid ID"),
        (
            "malformed-id",
            |v| v["malformed"][0]["id"] = "unknown".into(),
            "malformed ID",
        ),
        (
            "malformed-missing",
            |v| {
                v["malformed"].as_array_mut().unwrap().remove(0);
            },
            "count",
        ),
        (
            "malformed-duplicate",
            |v| v["malformed"][1]["id"] = v["malformed"][0]["id"].clone(),
            "malformed ID",
        ),
        ("malformed-reordered", swap_malformed_rows, "malformed ID"),
        (
            "valid-unknown-field",
            |v| v["valid"][0]["unknown"] = true.into(),
            "unknown field",
        ),
        (
            "biome-unknown-field",
            |v| v["valid"][20]["biome_sources"][0]["unknown"] = true.into(),
            "unknown field",
        ),
        (
            "grass-unknown-field",
            |v| v["valid"][20]["grass_samples"][0]["unknown"] = true.into(),
            "unknown field",
        ),
        (
            "malformed-unknown-field",
            |v| v["malformed"][0]["unknown"] = true.into(),
            "unknown field",
        ),
        (
            "pixel-short",
            |v| v["valid"][0]["pixels"] = Value::Array(vec![0.into(); 255]),
            "oracle vector",
        ),
        (
            "pixel-long",
            |v| v["valid"][0]["pixels"] = Value::Array(vec![0.into(); 257]),
            "oracle vector",
        ),
        (
            "pixel-range",
            |v| v["valid"][0]["pixels"][0] = (u64::from(u32::MAX) + 1).into(),
            "oracle vector",
        ),
        (
            "nonempty-all-zero",
            |v| v["valid"][0]["pixels"] = Value::Array(vec![0.into(); 256]),
            "all-zero",
        ),
        (
            "edge-short",
            |v| v["valid"][0]["south_edge"] = Value::Array(vec![0.into(); 15]),
            "oracle vector",
        ),
        (
            "settings-max-height",
            |v| v["valid"][0]["max_height"] = (-2).into(),
            "settings mismatch",
        ),
        (
            "settings-iterate",
            |v| v["valid"][0]["iterate_up"] = true.into(),
            "settings mismatch",
        ),
        (
            "settings-fluid",
            |v| v["valid"][0]["water_clear"] = true.into(),
            "settings mismatch",
        ),
        (
            "settings-blend",
            |v| v["valid"][0]["biome_blend"] = 1.into(),
            "settings mismatch",
        ),
        (
            "biome-disabled-source",
            copy_biome_sources_to_flat,
            "biome source topology",
        ),
        (
            "biome-enabled-missing",
            |v| v["valid"][20]["biome_sources"] = Value::Array(vec![]),
            "biome source topology",
        ),
        (
            "source-order",
            swap_first_biome_sources,
            "biome source topology",
        ),
        (
            "source-coordinate",
            |v| v["valid"][20]["biome_sources"][0]["x"] = 7.into(),
            "biome source topology",
        ),
        (
            "source-path",
            |v| {
                v["valid"][20]["biome_sources"][0]["path"] =
                    "chunks/biome-grass-radius-0/center.bin".into()
            },
            "duplicate referenced",
        ),
        (
            "grass-count",
            |v| {
                v["valid"][20]["grass_samples"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            },
            "grass tuple count",
        ),
        (
            "grass-order",
            swap_first_grass_samples,
            "grass tuple coordinate/order",
        ),
        (
            "grass-coordinate",
            |v| v["valid"][20]["grass_samples"][0]["block_x"] = 1.into(),
            "grass tuple coordinate/order",
        ),
        (
            "grass-y",
            |v| v["valid"][20]["grass_samples"][0]["block_y"] = 14.into(),
            "grass tuple coordinate/order",
        ),
        (
            "grass-known-wrong-biome",
            use_different_known_biome,
            "grass selected biome mismatch",
        ),
        (
            "grass-unknown-biome",
            |v| v["valid"][20]["grass_samples"][0]["biome_id"] = 999.into(),
            "grass selected biome mismatch",
        ),
        (
            "grass-range",
            |v| {
                v["valid"][20]["grass_samples"][0]["resolved_grass_argb"] =
                    (u64::from(u32::MAX) + 1).into()
            },
            "grass oracle range",
        ),
        (
            "malformed-path",
            |v| v["malformed"][0]["path"] = "malformed/crc.bin".into(),
            "malformed path mismatch",
        ),
        (
            "malformed-mutation",
            |v| v["malformed"][0]["mutation"] = "wrong".into(),
            "mutation mismatch",
        ),
        (
            "malformed-classifier",
            |v| v["malformed"][0]["classifier"] = "wrong".into(),
            "classifier mismatch",
        ),
        (
            "unsafe-path",
            |v| v["valid"][0]["chunk"] = "../escape".into(),
            "unsafe",
        ),
        (
            "absolute-path",
            |v| v["valid"][0]["chunk"] = "/escape".into(),
            "unsafe",
        ),
        (
            "missing-path",
            |v| v["valid"][0]["chunk"] = "missing.bin".into(),
            "No such",
        ),
        (
            "directory-path",
            |v| v["valid"][0]["chunk"] = "chunks".into(),
            "regular file",
        ),
        (
            "wrong-registry",
            |v| v["valid"][1]["registry"] = "chunks/flat-solid/center.bin".into(),
            "registry",
        ),
        (
            "duplicate-ref",
            |v| v["valid"][1]["chunk"] = v["valid"][0]["chunk"].clone(),
            "duplicate referenced",
        ),
        (
            "neighbor-presence",
            |v| v["valid"][2]["north"] = Value::Null,
            "neighbor topology",
        ),
    ];

    for (name, mutate, needle) in mutations {
        assert_manifest_rejected(name, *mutate, needle);
    }
}

#[test]
fn snapshot_metadata_and_filesystem_mutations_fail_closed() {
    assert_wire_mutation_rejected(
        "center-coordinate",
        0,
        "chunk",
        "chunks/flat-solid/center.bin",
        |wire| wire.coordinate.as_mut().unwrap().x = 1,
        "center metadata mismatch",
    );
    assert_wire_mutation_rejected(
        "north-coordinate",
        2,
        "north",
        "chunks/north-height-discontinuity/north.bin",
        |wire| wire.coordinate.as_mut().unwrap().z = -2,
        "neighbor coordinate mismatch",
    );
    assert_wire_mutation_rejected(
        "north-metadata",
        2,
        "north",
        "chunks/north-height-discontinuity/north.bin",
        |wire| wire.ceiling = !wire.ceiling,
        "neighbor metadata mismatch",
    );
    assert_wire_mutation_rejected(
        "biome-source-metadata",
        20,
        "biome_sources.0.path",
        "chunks/biome-grass-radius-0/biome--1-0.bin",
        |wire| wire.coordinate.as_mut().unwrap().x = -2,
        "biome source metadata mismatch",
    );

    let temp = copied_corpus();
    std::fs::write(temp.path().join("unexpected.bin"), b"unexpected").unwrap();
    assert_load_error(temp.path(), "inventory", "fixture file inventory mismatch");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let temp = copied_corpus();
        let original = temp.path().join("chunks/flat-solid/center.bin");
        std::fs::remove_file(&original).unwrap();
        symlink("../empty-chunk/center.bin", &original).unwrap();
        assert_load_error(temp.path(), "canonical-alias", "duplicate referenced");

        let temp = copied_corpus();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let original = temp.path().join("chunks/flat-solid/center.bin");
        std::fs::remove_file(&original).unwrap();
        symlink(outside.path(), &original).unwrap();
        assert_load_error(temp.path(), "symlink-escape", "path escapes root");
    }
}

#[test]
fn render_context_rejects_invalid_settings_and_bindings() {
    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    let generation = corpus.registry.generation();
    let mut settings = squaremap_render::RenderSettings::default();
    settings.map_max_height = -2;
    assert!(matches!(
        squaremap_render::RenderContext::try_new(
            generation.clone(),
            settings,
            [],
            [],
            None,
            || false
        ),
        Err(squaremap_render::RenderContextError::InvalidMaxHeight)
    ));
    let mut settings = squaremap_render::RenderSettings::default();
    settings.biome_blend = 16;
    assert!(matches!(
        squaremap_render::RenderContext::try_new(
            generation.clone(),
            settings,
            [],
            [],
            None,
            || false
        ),
        Err(squaremap_render::RenderContextError::InvalidBlend)
    ));
    let mut settings = squaremap_render::RenderSettings::default();
    settings.biomes_enabled = true;
    assert!(matches!(
        squaremap_render::RenderContext::try_new(
            generation.clone(),
            settings,
            [],
            [],
            None,
            || false
        ),
        Err(squaremap_render::RenderContextError::MissingBiomeSource)
    ));
    assert!(matches!(
        squaremap_render::RenderContext::try_new(
            generation.clone(),
            squaremap_render::RenderSettings::default(),
            [u32::MAX],
            [],
            None,
            || false
        ),
        Err(squaremap_render::RenderContextError::UnknownInvisibleId(
            u32::MAX
        ))
    ));
    assert!(matches!(
        squaremap_render::RenderContext::try_new(
            generation,
            squaremap_render::RenderSettings::default(),
            [],
            [u32::MAX],
            None,
            || false
        ),
        Err(squaremap_render::RenderContextError::UnknownIterateBaseId(
            u32::MAX
        ))
    ));
}

#[test]
fn cancellation_is_checked_at_entry_and_between_columns() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    let case = &corpus.cases[0];
    let immediate = squaremap_render::RenderContext::try_new(
        corpus.registry.generation(),
        squaremap_render::RenderSettings::default(),
        [],
        [],
        None,
        || true,
    )
    .unwrap();
    assert_eq!(
        squaremap_render::render_chunk(&immediate, None, &case.center, None),
        Err(squaremap_render::RenderError::Cancelled)
    );

    let checks = Arc::new(AtomicUsize::new(0));
    let probe = checks.clone();
    let transitioning = squaremap_render::RenderContext::try_new(
        corpus.registry.generation(),
        squaremap_render::RenderSettings::default(),
        [],
        [],
        None,
        move || probe.fetch_add(1, Ordering::SeqCst) >= 3,
    )
    .unwrap();
    assert_eq!(
        squaremap_render::render_chunk(&transitioning, None, &case.center, None),
        Err(squaremap_render::RenderError::Cancelled)
    );
    assert!(checks.load(Ordering::SeqCst) >= 3);
}

fn swap_valid_rows(value: &mut Value) {
    value["valid"].as_array_mut().unwrap().swap(0, 1);
}

fn swap_malformed_rows(value: &mut Value) {
    value["malformed"].as_array_mut().unwrap().swap(0, 1);
}

fn swap_first_biome_sources(value: &mut Value) {
    value["valid"][20]["biome_sources"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
}

fn swap_first_grass_samples(value: &mut Value) {
    value["valid"][20]["grass_samples"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
}

fn copy_biome_sources_to_flat(value: &mut Value) {
    value["valid"][0]["biome_sources"] = value["valid"][20]["biome_sources"].clone();
}

fn use_different_known_biome(value: &mut Value) {
    let current = value["valid"][20]["grass_samples"][0]["biome_id"]
        .as_u64()
        .unwrap();
    value["valid"][20]["grass_samples"][0]["biome_id"] =
        (if current == 11 { 12 } else { 11 }).into();
}

fn copied_corpus() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&corpus_root(), temp.path());
    temp
}

fn manifest_value(root: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap()
}

fn write_manifest(root: &Path, value: &Value) {
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

fn assert_manifest_rejected(name: &str, mutate: JsonMutation, needle: &str) {
    let temp = copied_corpus();
    let mut value = manifest_value(temp.path());
    mutate(&mut value);
    write_manifest(temp.path(), &value);
    assert_load_error(temp.path(), name, needle);
}

fn assert_load_error(root: &Path, name: &str, needle: &str) {
    let error = match FixtureCorpus::load(root) {
        Ok(_) => panic!("{name} unexpectedly accepted"),
        Err(error) => error,
    };
    assert!(error.contains(needle), "{name}: {error}");
}

fn assert_wire_mutation_rejected(
    name: &str,
    valid_index: usize,
    manifest_field: &str,
    relative: &str,
    mutate: fn(&mut WireSnapshot),
    needle: &str,
) {
    let temp = copied_corpus();
    let mut value = manifest_value(temp.path());
    let original = temp.path().join(relative);
    let mut wire = WireSnapshot::decode(std::fs::read(&original).unwrap().as_slice()).unwrap();
    mutate(&mut wire);
    std::fs::write(&original, wire.encode_to_vec()).unwrap();
    match manifest_field {
        "chunk" => value["valid"][valid_index]["chunk"] = relative.into(),
        "north" => value["valid"][valid_index]["north"] = relative.into(),
        "biome_sources.0.path" => {
            value["valid"][valid_index]["biome_sources"][0]["path"] = relative.into()
        }
        field => panic!("unsupported manifest field {field}"),
    }
    write_manifest(temp.path(), &value);
    assert_load_error(temp.path(), name, needle);
}

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::write(destination, std::fs::read(entry.path()).unwrap()).unwrap();
        }
    }
}
