use squaremap_render::{
    ChunkPixels,
    fixture::{FixtureCorpus, render_case},
};
use std::{hint::black_box, path::Path, time::Instant};

const ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v2/render"
);
const WARMUP_PASSES: usize = 10;
const MEASURED_PASSES: usize = 30;

fn checksum(mut hash: u64, output: &ChunkPixels) -> u64 {
    for pixel in output.pixels {
        hash = hash.wrapping_mul(31).wrapping_add(pixel as u64);
    }
    for edge in output.south_edge {
        hash = hash.wrapping_mul(31).wrapping_add(edge as u64);
    }
    hash
}

fn main() {
    let corpus = FixtureCorpus::load(Path::new(ROOT)).expect("valid render corpus");
    assert_eq!(corpus.cases.len(), 26);
    for _ in 0..WARMUP_PASSES {
        for case in &corpus.cases {
            black_box(render_case(case).expect("render"));
        }
    }
    let start = Instant::now();
    let mut hash = 0u64;
    for _ in 0..MEASURED_PASSES {
        let mut pass_hash = 0u64;
        for case in &corpus.cases {
            pass_hash = checksum(pass_hash, &render_case(case).expect("render"));
        }
        hash = hash.wrapping_mul(31).wrapping_add(pass_hash);
    }
    let elapsed_nanos = start.elapsed().as_nanos().max(1);
    let items = (MEASURED_PASSES * corpus.cases.len()) as f64;
    println!(
        "{{\"backend\":\"rust\",\"case_count\":{},\"warmup_passes\":{},\"measured_passes\":{},\"elapsed_nanos\":{},\"items_per_second\":{:.6},\"checksum\":{}}}",
        corpus.cases.len(),
        WARMUP_PASSES,
        MEASURED_PASSES,
        elapsed_nanos,
        items * 1_000_000_000.0 / elapsed_nanos as f64,
        hash as i64
    );
}
