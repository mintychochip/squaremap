use proptest::prelude::*;
use serde_json::{Map, Value};
use squaremap_render::{color, coordinates, visibility};

const FIXTURE: &str = include_str!("../../../../testdata/bridge/v1/render/primitives.json");

fn fixture() -> Value { serde_json::from_str(FIXTURE).expect("immutable fixture JSON") }
fn hex(value: &str) -> u32 { u32::from_str_radix(value.trim_start_matches("0x"), 16).expect("fixture hex") }
fn i32v(row: &Map<String, Value>, name: &str) -> i32 { row[name].as_i64().expect(name) as i32 }
fn boolv(row: &Map<String, Value>, name: &str) -> bool { row[name].as_bool().expect(name) }
fn row_array<'a>(row: &'a Value, name: &str) -> &'a [Value] { row[name].as_array().expect(name) }
fn query(row: &Value) -> (i32, i32) { (row["x"].as_i64().unwrap() as i32, row["z"].as_i64().unwrap() as i32) }

#[test]
fn fixture_schema_and_coordinates() {
    let f = fixture();
    assert_eq!(f["schema"], 1);
    assert_eq!(f["generator"], "render-primitives-java-v1");
    for row in row_array(&f["coordinates"], "conversions") {
        let object = row.as_object().unwrap();
        let n = i32v(object, "input");
        let tuple: Vec<i32> = row_array(row, "tuple").iter().map(|v| v.as_i64().unwrap() as i32).collect();
        assert_eq!(tuple, vec![
            coordinates::region_to_block(n).unwrap(),
            coordinates::block_to_region(n),
            coordinates::region_to_chunk(n).unwrap(),
            coordinates::chunk_to_region(n),
            coordinates::chunk_to_block(n).unwrap(),
            coordinates::block_to_chunk(n),
        ]);
    }
    for row in row_array(&f["coordinates"], "reverse") {
        let object = row.as_object().unwrap();
        let n = i32v(object, "input");
        let result = match object["function"].as_str().unwrap() {
            "region_to_block" => coordinates::region_to_block(n),
            "region_to_chunk" => coordinates::region_to_chunk(n),
            "chunk_to_block" => coordinates::chunk_to_block(n),
            other => panic!("unknown conversion {other}"),
        };
        match object["status"].as_str().unwrap() {
            "accepted" => assert_eq!(result.unwrap(), i32v(object, "java_result")),
            "overflow" => assert_eq!(result, Err(coordinates::CoordinateError::Overflow)),
            other => panic!("unknown status {other}"),
        }
    }
    for row in row_array(&f["coordinates"], "malformed") {
        let object = row.as_object().unwrap();
        let n = i32v(object, "input");
        let java_result = object["java_result"].as_i64().expect("recorded Java coordinate result");
        assert!((i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&java_result));
        let result = match object["function"].as_str().unwrap() {
            "region_to_block" => coordinates::region_to_block(n),
            "region_to_chunk" => coordinates::region_to_chunk(n),
            "chunk_to_block" => coordinates::chunk_to_block(n),
            other => panic!("unknown conversion {other}"),
        };
        assert_eq!(result, Err(coordinates::CoordinateError::Overflow));
    }
}

#[test]
fn fixture_tiles_and_empty_visibility() {
    let f = fixture();
    for row in row_array(&f["tiles"], "rows") {
        let tile = &row["tile"];
        let x = row["region_x"].as_i64().unwrap() as i32;
        let z = row["region_z"].as_i64().unwrap() as i32;
        let zoom = row["zoom"].as_u64().unwrap() as u8;
        let max_zoom = row["max_zoom"].as_u64().unwrap() as u8;
        let actual = coordinates::tile_for_region(x, z, zoom, max_zoom).unwrap();
        let origin = coordinates::tile_origin(x, z, zoom).unwrap();
        assert_eq!(actual.level, tile["level"].as_u64().unwrap() as u8);
        assert_eq!(actual.x, tile["x"].as_i64().unwrap() as i32);
        assert_eq!(actual.z, tile["z"].as_i64().unwrap() as i32);
        assert_eq!(origin.0, tile["origin_x"].as_u64().unwrap() as u16);
        assert_eq!(origin.1, tile["origin_z"].as_u64().unwrap() as u16);
    }
    for row in row_array(&f["tiles"], "invalid") {
        let zoom = row["zoom"].as_u64().unwrap() as u8;
        let max_zoom = row["max_zoom"].as_u64().unwrap() as u8;
        assert_eq!(coordinates::tile_for_region(0, 0, zoom, max_zoom), Err(coordinates::CoordinateError::InvalidZoom));
    }
    let empty = visibility::VisibilityLimit::new(Vec::new()).unwrap();
    let empty_fixture = &f["visibility"]["empty"];
    assert_eq!(empty.contains_block(i32::MIN, i32::MAX), empty_fixture["contains_block"]);
    assert_eq!(empty.contains_chunk(-3, 7), empty_fixture["contains_chunk"]);
    assert_eq!(empty.contains_region(12, -9), empty_fixture["contains_region"]);
    assert_eq!(empty.count_chunks_in_region(0, 0).unwrap(), empty_fixture["count_chunks"].as_u64().unwrap() as u16);
    assert_eq!(empty.count_chunks_in_region(i32::MAX, 0), Err(visibility::VisibilityError::CountOverflow));
}

fn shape_from_row(row: &Value) -> visibility::VisibilityShape {
    let kind = row["kind"].as_str().unwrap();
    let params: Vec<i32> = row_array(row, "params").iter().map(|v| v.as_i64().unwrap() as i32).collect();
    match kind {
        "rectangle" => visibility::VisibilityShape::Rectangle(visibility::Rectangle::new(params[0], params[1], params[2], params[3]).unwrap()),
        "circle" => visibility::VisibilityShape::Circle(visibility::Circle::new(params[0], params[1], params[2]).unwrap()),
        "polygon" => visibility::VisibilityShape::Polygon(visibility::Polygon::new(params.chunks_exact(2).map(|p| (p[0], p[1])).collect()).unwrap()),
        "world_border" => visibility::VisibilityShape::WorldBorder(visibility::WorldBorderSnapshot::new(params[0], params[1], params[2]).unwrap()),
        other => panic!("unknown shape {other}"),
    }
}

#[test]
fn every_visibility_fixture_vector_matches() {
    let f = fixture();
    for row in row_array(&f["visibility"], "shapes") {
        let shape = shape_from_row(row);
        for vector in row_array(row, "blocks") {
            let (x, z) = query(vector);
            let expected = vector["result"].as_bool().unwrap();
            let actual = match &shape {
                visibility::VisibilityShape::Rectangle(v) => v.contains_block(x, z),
                visibility::VisibilityShape::Circle(v) => v.contains_block(x, z),
                visibility::VisibilityShape::Polygon(v) => v.contains_block(x, z),
                visibility::VisibilityShape::WorldBorder(v) => v.contains_block(x, z),
            };
            assert_eq!(actual, expected, "{} block ({x},{z})", row["id"]);
        }
        for vector in row_array(row, "chunks") {
            let (x, z) = query(vector);
            let expected = vector["result"].as_bool().unwrap();
            let actual = match &shape {
                visibility::VisibilityShape::Rectangle(v) => v.contains_chunk(x, z),
                visibility::VisibilityShape::Circle(v) => v.contains_chunk(x, z),
                visibility::VisibilityShape::Polygon(v) => v.contains_chunk(x, z),
                visibility::VisibilityShape::WorldBorder(v) => v.contains_chunk(x, z),
            };
            assert_eq!(actual, expected, "{} chunk ({x},{z})", row["id"]);
        }
        for vector in row_array(row, "regions") {
            let (x, z) = query(vector);
            let expected = vector["result"].as_bool().unwrap();
            let actual = match &shape {
                visibility::VisibilityShape::Rectangle(v) => v.contains_region(x, z),
                visibility::VisibilityShape::Circle(v) => v.contains_region(x, z),
                visibility::VisibilityShape::Polygon(v) => v.contains_region(x, z),
                visibility::VisibilityShape::WorldBorder(v) => v.contains_region(x, z),
            };
            assert_eq!(actual, expected, "{} region ({x},{z})", row["id"]);
        }
        let limit = visibility::VisibilityLimit::new(vec![shape]).unwrap();
        for vector in row_array(row, "count_chunks") {
            let (x, z) = query(vector);
            assert_eq!(limit.count_chunks_in_region(x, z).unwrap(), vector["result"].as_u64().unwrap() as u16, "{} count ({x},{z})", row["id"]);
        }
    }
}

#[test]
fn every_runtime_border_fixture_vector_matches() {
    for row in row_array(&fixture()["visibility"], "runtime") {
        let border = visibility::WorldBorderSnapshot::from_runtime(row["center_x"].as_f64().unwrap(), row["center_z"].as_f64().unwrap(), row["size"].as_f64().unwrap()).unwrap();
        assert_eq!(border.center_x(), row["resolved"]["center_x"].as_i64().unwrap() as i32);
        assert_eq!(border.center_z(), row["resolved"]["center_z"].as_i64().unwrap() as i32);
        assert_eq!(border.radius(), row["resolved"]["radius"].as_i64().unwrap() as i32);
        for vector in row_array(row, "blocks") { let (x,z)=query(vector); assert_eq!(border.contains_block(x,z), vector["result"].as_bool().unwrap()); }
        for vector in row_array(row, "chunks") { let (x,z)=query(vector); assert_eq!(border.contains_chunk(x,z), vector["result"].as_bool().unwrap()); }
        for vector in row_array(row, "regions") { let (x,z)=query(vector); assert_eq!(border.contains_region(x,z), vector["result"].as_bool().unwrap()); }
        for vector in row_array(row, "count_chunks") { let (x,z)=query(vector); assert_eq!(visibility::VisibilityLimit::new(vec![visibility::VisibilityShape::WorldBorder(border)]).unwrap().count_chunks_in_region(x,z).unwrap(), vector["result"].as_u64().unwrap() as u16); }
    }
}

#[test]
fn java_degenerate_polygon_rows_are_java_accepted_but_rust_rejects() {
    for row in row_array(&fixture()["visibility"], "degenerate") {
        let params: Vec<i32> = row_array(row, "params").iter().map(|v| v.as_i64().unwrap() as i32).collect();
        assert_eq!(
            visibility::Polygon::new(params.chunks_exact(2).map(|p| (p[0], p[1])).collect()),
            Err(visibility::VisibilityError::InvalidPolygon)
        );
        for vectors in ["blocks", "chunks", "regions", "count_chunks"] {
            for vector in row_array(row, vectors) {
                let false_result = if vectors == "count_chunks" {
                    vector["result"].as_u64().unwrap() == 0
                } else {
                    !vector["result"].as_bool().unwrap()
                };
                assert!(false_result, "{} should be false", vectors);
            }
        }
    }
}

#[test]
fn visibility_validation_and_wire_contract() {
    use squaremap_protocol::wire::{Point, VisibilityLimit as Wire, VisibilityLimitKind};
    for row in row_array(&fixture()["visibility"], "invalid") {
        let kind = row["kind"].as_str().unwrap();
        if kind == "polygon" {
            assert_eq!(row["java_result"], "accepted");
        } else {
            assert_eq!(row["java_exception"], "IllegalArgumentException");
        }
        let case = row["case"].as_str().unwrap_or("runtime_required");
        let result = match (kind, case) {
            ("rectangle", _) => visibility::Rectangle::new(1, 1, 1, 2).map(visibility::VisibilityShape::Rectangle),
            ("circle", _) => visibility::Circle::new(0, 0, 0).map(visibility::VisibilityShape::Circle),
            ("polygon", _) => visibility::Polygon::new(vec![(0, 0), (1, 1)]).map(visibility::VisibilityShape::Polygon),
            ("world_border", "runtime_required") => {
                let wire = Wire { kind: VisibilityLimitKind::WorldBorder as i32, ..Wire::default() };
                visibility::VisibilityLimit::from_wire(&[wire], None).map(|_| visibility::VisibilityShape::Rectangle(visibility::Rectangle::new(0, 0, 1, 1).unwrap()))
            },
            ("world_border", "wire_extra_fields") => {
                let wire = Wire { kind: VisibilityLimitKind::WorldBorder as i32, points: vec![Point { x: 1, z: 1 }], ..Wire::default() };
                visibility::VisibilityLimit::from_wire(&[wire], None).map(|_| visibility::VisibilityShape::Rectangle(visibility::Rectangle::new(0, 0, 1, 1).unwrap()))
            },
            ("world_border", "runtime_overflow") => visibility::WorldBorderSnapshot::new(i32::MAX, 0, 1).map(visibility::VisibilityShape::WorldBorder),
            ("world_border", "runtime_nan") => visibility::WorldBorderSnapshot::from_runtime(f64::NAN, 0.0, 1.0).map(visibility::VisibilityShape::WorldBorder),
            other => panic!("unknown invalid shape {other:?}"),
        };
        let expected = match row["rust_error"].as_str().unwrap() {
            "InvalidRectangle" => visibility::VisibilityError::InvalidRectangle,
            "InvalidCircle" => visibility::VisibilityError::InvalidCircle,
            "InvalidPolygon" => visibility::VisibilityError::InvalidPolygon,
            "WorldBorderRuntimeRequired" => visibility::VisibilityError::WorldBorderRuntimeRequired,
            "InvalidWorldBorder" => visibility::VisibilityError::InvalidWorldBorder,
            "QueryOverflow" => visibility::VisibilityError::QueryOverflow,
            other => panic!("unknown error {other}"),
        };
        assert_eq!(result.unwrap_err(), expected);
    }
    let malformed_border = Wire { kind: VisibilityLimitKind::WorldBorder as i32, points: vec![Point { x: 1, z: 1 }], ..Wire::default() };
    assert_eq!(visibility::VisibilityLimit::from_wire(&[malformed_border], None).unwrap_err(), visibility::VisibilityError::InvalidWorldBorder);
    let circle = Wire { kind: VisibilityLimitKind::Circle as i32, radius: 12.5, ..Wire::default() };
    assert_eq!(visibility::VisibilityLimit::from_wire(&[circle], None).unwrap_err(), visibility::VisibilityError::InvalidCircle);
}

#[test]
fn every_color_fixture_vector_matches() {
    let f = fixture();
    for row in row_array(&f, "colors") {
        let object = row.as_object().unwrap();
        let op = object["op"].as_str().unwrap();
        let actual = match op {
            "remove_alpha" => color::remove_alpha(i32v(object, "input") as u32),
            "abgr_to_argb" => color::abgr_to_argb(i32v(object, "input") as u32),
            "argb_to_rgba" => color::argb_to_rgba(i32v(object, "input") as u32),
            "rgba_to_argb" => color::rgba_to_argb(i32v(object, "input") as u32),
            "mix" => color::mix(hex(object["c1"].as_str().unwrap()), hex(object["c2"].as_str().unwrap()), f32::from_bits(i32v(object, "ratio_bits") as u32)).unwrap(),
            "shade_level" => color::shade_level(i32v(object, "color") as u32, i32v(object, "level") as u8).unwrap(),
            "shade_factor" => color::shade_factor(i32v(object, "color") as u32, f32::from_bits(i32v(object, "factor_bits") as u32)).unwrap(),
            "checkerboard_parity" => color::checkerboard_parity(i32v(object, "x"), i32v(object, "z")) as u32,
            "terrain" => color::terrain(i32v(object, "current"), i32v(object, "previous"), i32v(object, "color") as u32, i32v(object, "odd") != 0),
            "depth_checkerboard" => color::depth_checkerboard(i32v(object, "depth") as u8, i32v(object, "color") as u32, i32v(object, "odd") != 0),
            "glass" => color::glass_composite(i32v(object, "under") as u32, i32v(object, "glass") as u32, f32::from_bits(i32v(object, "alpha_bits") as u32)).unwrap(),
            "average_argb" => color::average_argb(&row_array(row, "values").iter().map(|v| v.as_i64().unwrap() as u32).collect::<Vec<_>>()).unwrap(),
            "water_biome_blend" => {
                let values = row_array(row, "values");
                let base = values[0].as_i64().unwrap() as u32;
                let samples: Vec<u32> = values[1..].iter().map(|v| v.as_i64().unwrap() as u32).collect();
                color::water_biome_blend(base, &samples).unwrap()
            },
            "fluid" => color::compose_fluid(
                i32v(object, "depth") as u8,
                i32v(object, "color") as u32,
                i32v(object, "under") as u32,
                if boolv(object, "water") { color::FluidKind::Water } else { color::FluidKind::Lava },
                i32v(object, "odd") != 0,
                color::FluidFlags {
                    water_checkerboard: boolv(object, "water_checker"),
                    water_clear: boolv(object, "water_clear"),
                    lava_checkerboard: boolv(object, "lava_checker"),
                },
            ).unwrap(),
            other => panic!("unhandled color fixture operation {other}"),
        };
        let expected = if op == "checkerboard_parity" {
            object["result"].as_u64().unwrap() as u32
        } else {
            hex(object["result"].as_str().unwrap())
        };
        assert_eq!(actual, expected, "{op}");
    }
    for row in row_array(&f, "color_invalid") {
        let object = row.as_object().unwrap();
        let op = object["op"].as_str().unwrap();
        if op == "average_empty" {
            let java_exception = object["java_exception"].as_str().expect("recorded Java exception");
            assert!(!java_exception.is_empty());
            assert!(java_exception.chars().all(|character| character.is_ascii_alphanumeric()));
        } else {
            let java_result = object["java_result"].as_str().expect("recorded Java color result");
            assert_eq!(java_result.len(), 10);
            let _ = hex(java_result);
        }
        let result = match op {
            "mix_nan" => color::mix(0, 0, f32::NAN).map(|_| ()),
            "mix_pos_inf" => color::mix(0, 0, f32::INFINITY).map(|_| ()),
            "mix_neg_inf" => color::mix(0, 0, f32::NEG_INFINITY).map(|_| ()),
            "shade_nan" => color::shade_factor(0, f32::NAN).map(|_| ()),
            "shade_out_of_range" => color::shade_factor(0, 1.1).map(|_| ()),
            "average_empty" => color::average_argb(&[]).map(|_| ()),
            "fluid_depth_zero" => color::compose_fluid(0, 0, 0, color::FluidKind::Water, false, Default::default()).map(|_| ()),
            other => panic!("unhandled invalid color operation {other}"),
        };
        let expected = match object["rust_error"].as_str().unwrap() {
            "NonFiniteFloat" => color::ColorError::NonFiniteFloat,
            "OutOfRangeFactor" => color::ColorError::OutOfRangeFactor,
            "EmptyBlend" => color::ColorError::EmptyBlend,
            "InvalidDepth" => color::ColorError::InvalidDepth,
            other => panic!("unknown color error {other}"),
        };
        assert_eq!(result, Err(expected));
    }
}

#[test]
fn every_fluid_classification_fixture_vector_matches() {
    for row in row_array(&fixture(), "fluid_classification") {
        let color_value = row["color"].as_i64().unwrap() as u32;
        let actual = color::classify_unknown_fluid(color_value, row["native_water"].as_bool().unwrap(), row["native_lava"].as_bool().unwrap());
        let expected = match row["result"].as_str().unwrap() {
            "water" => color::FluidKind::Water,
            "lava" => color::FluidKind::Lava,
            other => panic!("unknown fluid kind {other}"),
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn render_boundary_invariants() {
    let border = visibility::WorldBorderSnapshot::from_runtime(16.9, -16.9, 32.0).unwrap();
    assert_eq!((border.center_x(), border.center_z(), border.radius()), (16, -16, 16));
    assert!(border.contains_block(0, -1));
    assert!(!border.contains_block(-1, -1));
    let border_limit = visibility::VisibilityLimit::new(vec![visibility::VisibilityShape::WorldBorder(visibility::WorldBorderSnapshot::new(0, 0, 16).unwrap())]).unwrap();
    assert_eq!(border_limit.count_chunks_in_region(0, 0).unwrap(), 4);
    assert!(visibility::Circle::new(0, 0, 46_340).is_ok());
    assert_eq!(visibility::Circle::new(0, 0, 46_341), Err(visibility::VisibilityError::InvalidCircle));
    assert_eq!(
        visibility::Polygon::new(vec![(i32::MIN, 0), (i32::MAX, 0), (0, 1)]),
        Err(visibility::VisibilityError::InvalidPolygon)
    );
}


proptest! {
    #[test]
    fn region_to_block_domain(r in -4_194_304i32..=4_194_303i32) {
        let blocks = coordinates::region_to_block(r).unwrap();
        prop_assert_eq!(coordinates::block_to_region(blocks), r);
    }

    #[test]
    fn region_to_chunk_domain(r in -67_108_864i32..=67_108_863i32) {
        let chunks = coordinates::region_to_chunk(r).unwrap();
        prop_assert_eq!(coordinates::chunk_to_region(chunks), r);
    }

    #[test]
    fn chunk_to_block_domain(c in -134_217_728i32..=134_217_727i32) {
        let blocks = coordinates::chunk_to_block(c).unwrap();
        prop_assert_eq!(coordinates::block_to_chunk(blocks), c);
    }

    #[test]
    fn floor_coordinate_invariants(block in any::<i32>()) {
        let block_i64 = i64::from(block);
        let chunk = i64::from(coordinates::block_to_chunk(block));
        let region = i64::from(coordinates::block_to_region(block));
        prop_assert!(block_i64 >= chunk * 16 && block_i64 < (chunk + 1) * 16);
        prop_assert!(block_i64 >= region * 512 && block_i64 < (region + 1) * 512);
    }


    #[test]
    fn rectangle_endpoints_are_inclusive(x in -100i32..=100i32, z in -100i32..=100i32, width in 1i32..=100i32, height in 1i32..=100i32) {
        let rectangle = visibility::Rectangle::new(x, z, x + width, z + height).unwrap();
        prop_assert!(rectangle.contains_block(x, z));
        prop_assert!(rectangle.contains_block(x + width, z + height));
        prop_assert!(!rectangle.contains_block(x - 1, z));
        prop_assert!(!rectangle.contains_block(x + width + 1, z));
    }

    #[test]
    fn circle_radius_is_monotonic(cx in -100i32..=100i32, cz in -100i32..=100i32, inner in 1i32..=100i32, extra in 0i32..=100i32, dx in -200i32..=200i32, dz in -200i32..=200i32) {
        let small = visibility::Circle::new(cx, cz, inner).unwrap();
        let large = visibility::Circle::new(cx, cz, inner + extra).unwrap();
        prop_assert!(small.contains_block(cx + inner, cz));
        prop_assert!(!small.contains_block(cx + inner + 1, cz));
        prop_assert!(!small.contains_block(cx + dx, cz + dz) || large.contains_block(cx + dx, cz + dz));
    }

    #[test]
    fn visibility_union_is_monotonic_at_all_scales(region in -4i32..=4i32, x in -2048i32..=2048i32, z in -2048i32..=2048i32) {
        let rectangle = visibility::Rectangle::new(-17, -17, 17, 17).unwrap();
        let circle = visibility::Circle::new(0, 0, 32).unwrap();
        let one = visibility::VisibilityLimit::new(vec![visibility::VisibilityShape::Rectangle(rectangle.clone())]).unwrap();
        let union = visibility::VisibilityLimit::new(vec![
            visibility::VisibilityShape::Rectangle(rectangle.clone()),
            visibility::VisibilityShape::Circle(circle.clone()),
        ]).unwrap();
        prop_assert!(!one.contains_block(x, z) || union.contains_block(x, z));
        let chunk_x = coordinates::block_to_chunk(x);
        let chunk_z = coordinates::block_to_chunk(z);
        prop_assert!(!one.contains_chunk(chunk_x, chunk_z) || union.contains_chunk(chunk_x, chunk_z));
        let region_x = coordinates::block_to_region(x);
        let region_z = coordinates::block_to_region(z);
        prop_assert!(!one.contains_region(region_x, region_z) || union.contains_region(region_x, region_z));
        let start_x = region * 32;
        let start_z = region * 32;
        let expected = (0..32).flat_map(|dx| (0..32).map(move |dz| (start_x + dx, start_z + dz)))
            .filter(|&(cx, cz)| rectangle.contains_chunk(cx, cz) || circle.contains_chunk(cx, cz)).count();
        let one_count = one.count_chunks_in_region(region, region).unwrap();
        let union_count = union.count_chunks_in_region(region, region).unwrap();
        prop_assert!(one_count <= union_count);
        prop_assert_eq!(union_count as usize, expected);
    }

    #[test]
    fn polygon_region_scan_matches_chunk_implication(region in -4i32..=4i32) {
        let polygon = visibility::Polygon::new(vec![(0, 0), (96, 0), (0, 96)]).unwrap();
        let start = region * 32;
        let expected = (0..32).flat_map(|dx| (0..32).map(move |dz| (start + dx, start + dz)))
            .filter(|&(cx, cz)| polygon.contains_chunk(cx, cz)).count();
        prop_assert_eq!(polygon.contains_region(region, region), expected > 0);
        let limit = visibility::VisibilityLimit::new(vec![visibility::VisibilityShape::Polygon(polygon)]).unwrap();
        prop_assert_eq!(limit.count_chunks_in_region(region, region).unwrap() as usize, expected);
    }

    #[test]
    fn color_channels_preserve_alpha_and_finite_endpoints(c1 in any::<u32>(), c2 in any::<u32>(), ratio in 0.0f32..=1.0f32) {
        let mixed = color::mix(c1, c2, ratio).unwrap();
        if ratio > 0.0 && ratio < 1.0 {
            prop_assert_eq!(mixed >> 24, 0xFF);
        }
        for shift in [0, 8, 16] {
            prop_assert!(((mixed >> shift) & 0xFF) <= 0xFF);
        }
        prop_assert_eq!(color::mix(c1, c2, 0.0).unwrap(), c1);
        prop_assert_eq!(color::mix(c1, c2, 1.0).unwrap(), c2);
        prop_assert_eq!(color::shade_factor(c1, ratio).unwrap() >> 24, 0xFF);
    }

    #[test]
    fn average_is_order_independent_and_truncates(values in prop::collection::vec(any::<u32>(), 1..=32)) {
        let actual = color::average_argb(&values).unwrap();
        let mut reversed = values.clone();
        reversed.reverse();
        prop_assert_eq!(actual, color::average_argb(&reversed).unwrap());
        let n = values.len() as u64;
        let sums = values.iter().fold([0_u64; 4], |mut sums, &value| {
            sums[0] += u64::from(value >> 24);
            sums[1] += u64::from((value >> 16) & 0xFF);
            sums[2] += u64::from((value >> 8) & 0xFF);
            sums[3] += u64::from(value & 0xFF);
            sums
        });
        let expected = ((sums[0] / n) as u32) << 24 | ((sums[1] / n) as u32) << 16
            | ((sums[2] / n) as u32) << 8 | (sums[3] / n) as u32;
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn finite_and_nonfinite_color_inputs_are_typed(bits in any::<u32>()) {
        let value = f32::from_bits(bits);
        let mixed = color::mix(0x12345678, 0x80A0B0C0, value);
        let shaded = color::shade_factor(0x12345678, value);
        if !value.is_finite() {
            prop_assert_eq!(mixed, Err(color::ColorError::NonFiniteFloat));
            prop_assert_eq!(shaded, Err(color::ColorError::NonFiniteFloat));
        } else if !(0.0..=1.0).contains(&value) {
            prop_assert!(mixed.is_ok());
            prop_assert_eq!(shaded, Err(color::ColorError::OutOfRangeFactor));
        }
    }

    #[test]
    fn malformed_wire_inputs_never_panic(kind in any::<i32>(), point_count in 0usize..=6, radius in any::<f32>()) {
        use squaremap_protocol::wire::{Point, VisibilityLimit as Wire};
        let points = (0..point_count).map(|i| Point { x: i as i32, z: i as i32 }).collect();
        let wire = Wire { kind, points, radius: f64::from(radius), ..Wire::default() };
        let result = visibility::VisibilityLimit::from_wire(&[wire], None);
        if !(1..=4).contains(&kind) {
            prop_assert!(matches!(result, Err(visibility::VisibilityError::UnsupportedWireKind)));
        } else {
            prop_assert!(matches!(result, Ok(_) | Err(
                visibility::VisibilityError::InvalidWorldBorder
                | visibility::VisibilityError::InvalidCircle
                | visibility::VisibilityError::InvalidRectangle
                | visibility::VisibilityError::InvalidPolygon
                | visibility::VisibilityError::WorldBorderRuntimeRequired
                | visibility::VisibilityError::QueryOverflow
                | visibility::VisibilityError::CountOverflow
                | visibility::VisibilityError::UnsupportedWireKind
            )));
        }
    }

    #[test]
    fn runtime_borders_are_bounded_and_queryable(center_x in -1_000_000.0f64..=1_000_000.0f64, center_z in -1_000_000.0f64..=1_000_000.0f64, size in 0.0f64..=1_000.0f64, x in -1_000_000i32..=1_000_000i32, z in -1_000_000i32..=1_000_000i32) {
        let border = visibility::WorldBorderSnapshot::from_runtime(center_x, center_z, size).unwrap();
        prop_assert_eq!(border.radius(), (size / 2.0).ceil() as i32);
        let _ = border.contains_block(x, z);
        let _ = border.contains_chunk(coordinates::block_to_chunk(x), coordinates::block_to_chunk(z));
        let _ = border.contains_region(coordinates::block_to_region(x), coordinates::block_to_region(z));
    }

    #[test]
    fn invalid_zoom_and_shade_inputs_are_named(zoom in any::<u8>(), max_zoom in any::<u8>(), level in any::<u8>()) {
        let tile = coordinates::tile_for_region(0, 0, zoom, max_zoom);
        if zoom > max_zoom || max_zoom > 9 {
            prop_assert_eq!(tile, Err(coordinates::CoordinateError::InvalidZoom));
        }
        let shade = color::shade_level(0x12345678, level);
        if level > 2 {
            prop_assert_eq!(shade, Err(color::ColorError::InvalidShade));
        }
    }
}

#[test]
fn floor_coordinate_extremes_fit_half_open_i64_bounds() {
    for block in [i32::MIN, i32::MAX] {
        let block_i64 = i64::from(block);
        let chunk = i64::from(coordinates::block_to_chunk(block));
        let region = i64::from(coordinates::block_to_region(block));
        assert!(block_i64 >= chunk * 16);
        assert!(block_i64 < (chunk + 1) * 16);
        assert!(block_i64 >= region * 512);
        assert!(block_i64 < (region + 1) * 512);
    }
}

#[test]
fn accumulation_overflow_is_named_error() {
    let colors = vec![u32::MAX; (i32::MAX as usize / 255) + 1];
    assert_eq!(color::average_argb(&colors), Err(color::ColorError::AccumulationOverflow));
}

#[test]
fn malformed_named_errors_remain_typed() {
    assert_eq!(coordinates::region_to_block(i32::MAX), Err(coordinates::CoordinateError::Overflow));
    assert_eq!(coordinates::tile_for_region(0, 0, 10, 10), Err(coordinates::CoordinateError::InvalidZoom));
    assert_eq!(color::shade_level(0, 3), Err(color::ColorError::InvalidShade));
    assert_eq!(color::shade_factor(0x12345678, 1.1), Err(color::ColorError::OutOfRangeFactor));
    assert_eq!(color::average_argb(&[]), Err(color::ColorError::EmptyBlend));
    assert_eq!(visibility::WorldBorderSnapshot::from_runtime(f64::NAN, 0.0, 1.0), Err(visibility::VisibilityError::QueryOverflow));
    assert_eq!(visibility::WorldBorderSnapshot::from_runtime(0.0, 0.0, -1.0), Err(visibility::VisibilityError::QueryOverflow));
}
