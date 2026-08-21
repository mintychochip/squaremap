#[allow(dead_code)]
#[path = "../../tests/support/tile_cases.rs"]
mod tile_cases;

use async_trait::async_trait;
use squaremap_render::{PublishResult, TileStore, TileStoreError, decode_rgba_png};
use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tile_cases::{MANIFEST_PATH, apply_case_to, load_manifest, sha256_hex};

const WARMUP_PASSES: usize = 10;
const MEASURED_PASSES: usize = 30;

struct FsTileStore {
    root: PathBuf,
}

#[async_trait]
impl TileStore for FsTileStore {
    async fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, TileStoreError> {
        let path = self.root.join(path);
        if !path.is_file() {
            return Ok(None);
        }
        fs::read(&path)
            .map(Some)
            .map_err(|error| TileStoreError::new(error.to_string()))
    }

    async fn publish(&self, path: &Path, bytes: &[u8]) -> Result<PublishResult, TileStoreError> {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| TileStoreError::new(error.to_string()))?;
        }
        fs::write(&path, bytes).map_err(|error| TileStoreError::new(error.to_string()))?;
        Ok(PublishResult::default())
    }
}

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn png_paths(case_dir: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let Ok(zooms) = fs::read_dir(case_dir) else {
        return paths;
    };
    for zoom in zooms.flatten() {
        if !zoom.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(children) = fs::read_dir(zoom.path()) else {
            continue;
        };
        for child in children.flatten() {
            let name = child.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".png")
                && child
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
            {
                paths.push(format!("{}/{}", zoom.file_name().to_string_lossy(), name));
            }
        }
    }
    paths.sort();
    paths
}

fn fold_rgba(mut hash: u64, rgba: &[u8]) -> u64 {
    for byte in rgba {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(*byte));
    }
    hash
}

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async_main());
}

async fn async_main() {
    let (manifest, manifest_bytes) = load_manifest(Path::new(MANIFEST_PATH));
    assert_eq!(manifest.cases.len(), 10);
    let manifest_hash = sha256_hex(&manifest_bytes);
    let root = TempRoot(std::env::temp_dir().join(format!(
        "squaremap-tile-bench-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    )));
    fs::create_dir_all(&root.0).expect("temp root");
    let stores: Vec<Arc<dyn TileStore>> = manifest
        .cases
        .iter()
        .map(|case| {
            let dir = root.0.join(&case.id);
            fs::create_dir_all(&dir).expect("case dir");
            Arc::new(FsTileStore { root: dir }) as Arc<dyn TileStore>
        })
        .collect();
    for _ in 0..WARMUP_PASSES {
        for (case, store) in manifest.cases.iter().zip(&stores) {
            black_box(
                apply_case_to(store.clone(), &manifest, case)
                    .await
                    .expect("warmup"),
            );
        }
    }
    let start = Instant::now();
    let mut pass_nanos = Vec::with_capacity(MEASURED_PASSES);
    for _ in 0..MEASURED_PASSES {
        let pass_start = Instant::now();
        for (case, store) in manifest.cases.iter().zip(&stores) {
            apply_case_to(store.clone(), &manifest, case)
                .await
                .expect("apply");
        }
        pass_nanos.push(pass_start.elapsed().as_nanos().max(1));
    }
    let elapsed_nanos = start.elapsed().as_nanos().max(1);
    let mut hash = 0u64;
    for case in &manifest.cases {
        let case_dir = root.0.join(&case.id);
        for relative in png_paths(&case_dir) {
            let png = fs::read(case_dir.join(&relative)).expect("png");
            let rgba = decode_rgba_png(&png).expect("decode");
            hash = fold_rgba(hash, &rgba);
        }
    }
    let items = (MEASURED_PASSES * manifest.cases.len()) as f64;
    let pass_nanos_json = pass_nanos
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        "{{\"backend\":\"rust\",\"workload\":\"pyramid-png-v2\",\"case_count\":{},\"warmup_passes\":{},\"measured_passes\":{},\"elapsed_nanos\":{},\"items_per_second\":{:.6},\"checksum\":{},\"manifest_hash\":\"{}\",\"pass_nanos\":[{}]}}",
        manifest.cases.len(),
        WARMUP_PASSES,
        MEASURED_PASSES,
        elapsed_nanos,
        items * 1_000_000_000.0 / elapsed_nanos as f64,
        hash as i64,
        manifest_hash,
        pass_nanos_json
    );
    println!("{json}");
    write_ab_out(&json);
}

fn write_ab_out(json: &str) {
    let Ok(path) = std::env::var("SQUAREMAP_AB_OUT") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let path = Path::new(&path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("SQUAREMAP_AB_OUT parent");
    }
    fs::write(path, format!("{json}\n")).expect("SQUAREMAP_AB_OUT");
}
