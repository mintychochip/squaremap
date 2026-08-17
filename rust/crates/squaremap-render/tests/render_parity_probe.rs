#[path = "support/fixtures.rs"]
mod fixture_support;

use squaremap_render::fixture::{FixtureCorpus, render_case};
use std::path::Path;

fn corpus_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../testdata/bridge/v2/render")
}

#[test]
fn canonical_java_corpus_has_complete_denominators() {
    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    assert_eq!(corpus.cases.len(), 26);
    assert_eq!(corpus.manifest.malformed.len(), 14);
    assert!(
        corpus
            .manifest
            .valid
            .iter()
            .all(|row| row.pixels.len() == 256 && row.south_edge.len() == 16)
    );
    assert!(
        corpus
            .manifest
            .malformed
            .iter()
            .all(|row| !row.id.is_empty() && !row.classifier.is_empty())
    );
}

#[test]
fn independently_renders_every_java_valid_case_with_exact_pixels_and_edges() {
    let corpus = FixtureCorpus::load(&corpus_root()).unwrap();
    for case in &corpus.cases {
        let output = render_case(case).unwrap_or_else(|error| panic!("{}: {error:?}", case.row.id));
        let expected_pixels: [u32; 256] = case
            .row
            .pixels
            .iter()
            .map(|pixel| *pixel as u32)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let expected_edge: [i32; 16] = case.row.south_edge.clone().try_into().unwrap();
        assert_eq!(
            output.pixels, expected_pixels,
            "pixel mismatch for {}",
            case.row.id
        );
        assert_eq!(
            output.south_edge, expected_edge,
            "south edge mismatch for {}",
            case.row.id
        );
    }
}

#[test]
fn malformed_cases_match_typed_render_contracts() {
    let corpus = fixture_support::FixtureCorpus::load(&corpus_root()).unwrap();
    for row in &corpus.manifest.malformed {
        let decoded = fixture_support::malformed_error(row, &corpus);
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
            "malformed-packed-width" => assert!(
                matches!(decoded, Err(squaremap_render::SnapshotError::InvalidPacking { kind: "block", actual, .. }) if actual != 3)
            ),
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
