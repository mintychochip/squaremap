#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{FixtureCorpus, render_case};
use squaremap_render::{
    RenderError,
    region::{render_column, render_south_row},
    render_chunk,
};
use std::path::Path;

fn corpus() -> FixtureCorpus {
    FixtureCorpus::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../testdata/bridge/v2/render"),
    )
    .unwrap()
}

#[test]
fn region_render_exact_topology_and_carry_matrix() {
    let corpus = corpus();
    let north_case = corpus
        .cases
        .iter()
        .find(|case| case.row.id == "north-height-discontinuity")
        .unwrap();
    let north = north_case.north.as_deref().unwrap();
    let expected_center = render_case(north_case).unwrap();
    let output = render_column(
        &north_case.context,
        Some(north),
        &[Some(&north_case.center)],
    )
    .unwrap();
    assert_eq!(output[0].as_ref().unwrap().pixels, expected_center.pixels);
    assert_eq!(
        output[0].as_ref().unwrap().south_edge,
        expected_center.south_edge
    );
    assert_eq!(
        render_column(
            &north_case.context,
            Some(&north_case.center),
            &[Some(&north_case.center)]
        ),
        Err(RenderError::CoordinateMismatch)
    );

    let adjacent = render_column(
        &north_case.context,
        None,
        &[Some(north), Some(&north_case.center)],
    )
    .unwrap();
    let expected_north = render_chunk(&north_case.context, None, north, None).unwrap();
    assert_eq!(adjacent[0].as_ref().unwrap().pixels, expected_north.pixels);
    assert_eq!(
        adjacent[0].as_ref().unwrap().south_edge,
        expected_north.south_edge
    );
    assert_eq!(adjacent[1].as_ref().unwrap().pixels, expected_center.pixels);
    assert_eq!(
        adjacent[1].as_ref().unwrap().south_edge,
        expected_center.south_edge
    );

    let flat = corpus
        .cases
        .iter()
        .find(|case| case.row.id == "flat-solid")
        .unwrap();
    let expected_flat = render_case(flat).unwrap();
    let absent = render_column(&flat.context, None, &[Some(&flat.center)]).unwrap();
    assert_eq!(absent[0].as_ref().unwrap().pixels, expected_flat.pixels);
    assert_eq!(
        absent[0].as_ref().unwrap().south_edge,
        expected_flat.south_edge
    );

    let south_case = corpus
        .cases
        .iter()
        .find(|case| case.row.id == "south-height-discontinuity")
        .unwrap();
    let south = south_case.south.as_deref().unwrap();
    let above_output = render_column(&north_case.context, None, &[Some(north)]).unwrap();
    let expected_row = render_south_row(
        &north_case.context,
        south,
        above_output[0].as_ref().unwrap().south_edge,
    )
    .unwrap();
    let gapped = render_column(&north_case.context, Some(north), &[None, Some(south)]).unwrap();
    assert!(gapped[0].is_none());
    let gapped_row = std::array::from_fn(|x| gapped[1].as_ref().unwrap().pixels[x * 16]);
    assert_eq!(gapped_row, expected_row);

    let water = corpus
        .cases
        .iter()
        .find(|case| case.row.id == "biome-water-radius-0")
        .unwrap();
    let carried = [0; 16];
    let before = carried;
    let row = render_south_row(&water.context, &water.center, carried).unwrap();
    let expected = std::array::from_fn(|x| water.expected_pixels[x * 16]);
    assert_eq!(row, expected);
    assert_eq!(carried, before);
    assert_eq!(
        render_south_row(&water.context, &water.center, carried).unwrap(),
        row
    );
}
