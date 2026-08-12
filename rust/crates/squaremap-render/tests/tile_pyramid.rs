use squaremap_render::coordinates::RegionCoord;
use squaremap_render::{
    MemoryTileStore, PngOptions, PublishResult, RegionPixels, TileError, TilePyramid, TileStore,
    TileStoreError, decode_rgba_png, encode_rgba_png,
};
use async_trait::async_trait;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;
use std::pin::Pin;
use std::task::{Context, Poll};

const SIZE: usize = 512;
const RGBA_BYTES: usize = SIZE * SIZE * 4;

struct CancelAfterPending<F> {
    future: Pin<Box<F>>,
}

impl<F: Future> Future for CancelAfterPending<F> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        assert!(this.future.as_mut().poll(context).is_pending());
        Poll::Ready(())
    }
}

struct BlockingStore {
    inner: MemoryTileStore,
    block_next_read: AtomicBool,
    read_started: Notify,
    release_read: Notify,
}

impl BlockingStore {
    fn new() -> Self {
        Self {
            inner: MemoryTileStore::new(),
            block_next_read: AtomicBool::new(true),
            read_started: Notify::new(),
            release_read: Notify::new(),
        }
    }
}

#[async_trait]
impl TileStore for BlockingStore {
    async fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, TileStoreError> {
        if self.block_next_read.swap(false, Ordering::AcqRel) {
            self.read_started.notify_one();
            self.release_read.notified().await;
        }
        self.inner.read(path).await
    }

    async fn publish(&self, path: &Path, bytes: &[u8]) -> Result<PublishResult, TileStoreError> {
        self.inner.publish(path, bytes).await
    }
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

fn solid(color: [u8; 4]) -> RegionPixels {
    RegionPixels::full_rgba(color)
}

fn pixel(image: &[u8], x: usize, z: usize) -> [u8; 4] {
    let offset = (z * SIZE + x) * 4;
    image[offset..offset + 4].try_into().unwrap()
}

fn path(level: u8, x: i32, z: i32) -> PathBuf {
    PathBuf::from(format!("{level}/{x}_{z}.png"))
}

fn encoded_png(width: u32, height: u32, color_type: png::ColorType) -> Vec<u8> {
    let channels = match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::Rgb => 3,
        png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgba => 4,
    };
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(color_type);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; width as usize * height as usize * channels])
            .unwrap();
    }
    encoded
}

async fn decoded(store: &MemoryTileStore, path: &Path) -> Vec<u8> {
    let bytes = store.read(path).await.unwrap().unwrap();
    decode_rgba_png(&bytes).unwrap()
}

#[tokio::test]
async fn exact_paths_quadrants_and_negative_coordinates_match_java() {
    let store = Arc::new(MemoryTileStore::new());
    let pyramid = TilePyramid::new(store.clone(), 2, PngOptions::default()).unwrap();
    let cases = [
        (
            RegionCoord { x: 0, z: 0 },
            [path(2, 0, 0), path(1, 0, 0), path(0, 0, 0)],
            (0, 0),
        ),
        (
            RegionCoord { x: 1, z: 0 },
            [path(2, 1, 0), path(1, 0, 0), path(0, 0, 0)],
            (256, 0),
        ),
        (
            RegionCoord { x: 0, z: 1 },
            [path(2, 0, 1), path(1, 0, 0), path(0, 0, 0)],
            (0, 256),
        ),
        (
            RegionCoord { x: 1, z: 1 },
            [path(2, 1, 1), path(1, 0, 0), path(0, 0, 0)],
            (256, 256),
        ),
        (
            RegionCoord { x: -1, z: -1 },
            [path(2, -1, -1), path(1, -1, -1), path(0, -1, -1)],
            (256, 256),
        ),
    ];

    for (index, (region, expected_paths, parent_origin)) in cases.into_iter().enumerate() {
        let color = rgba(index as u8 + 1, 10, 20, 255);
        let result = pyramid.apply_region(region, &solid(color)).await.unwrap();
        assert_eq!(result.changed_paths, expected_paths);
        assert!(result.warnings.is_empty());
        let parent = decoded(&store, &result.changed_paths[1]).await;
        assert_eq!(pixel(&parent, parent_origin.0, parent_origin.1), color);
        assert_eq!(pyramid.live_lock_count(), 0);
    }
}

#[tokio::test]
async fn sparse_updates_distinguish_unset_from_transparent_and_preserve_neighbors() {
    let store = Arc::new(MemoryTileStore::new());
    let pyramid = TilePyramid::new(store.clone(), 0, PngOptions::default()).unwrap();
    let tile_path = path(0, 0, 0);
    let old = vec![7_u8; RGBA_BYTES];
    store.seed(
        tile_path.clone(),
        encode_rgba_png(&old, PngOptions::default()).unwrap(),
    );

    let mut update = RegionPixels::empty();
    for x in 32..48 {
        for z in 64..80 {
            update.set_rgba(x, z, rgba(1, 2, 3, 4)).unwrap();
        }
    }
    update.set_argb(32, 64, 0).unwrap();
    pyramid
        .apply_region(RegionCoord { x: 0, z: 0 }, &update)
        .await
        .unwrap();

    let image = decoded(&store, &tile_path).await;
    assert_eq!(pixel(&image, 32, 64), rgba(0, 0, 0, 0));
    assert_eq!(pixel(&image, 47, 79), rgba(1, 2, 3, 4));
    assert_eq!(pixel(&image, 31, 64), rgba(7, 7, 7, 7));
    assert_eq!(pixel(&image, 48, 80), rgba(7, 7, 7, 7));
    assert_eq!(pyramid.live_lock_count(), 0);
}

#[test]
fn compressed_and_uncompressed_pngs_decode_to_identical_rgba() {
    let mut image = vec![0_u8; RGBA_BYTES];
    for (index, byte) in image.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    let compressed = encode_rgba_png(&image, PngOptions { compression: true }).unwrap();
    let uncompressed = encode_rgba_png(&image, PngOptions { compression: false }).unwrap();
    assert_eq!(decode_rgba_png(&compressed).unwrap(), image);
    assert_eq!(decode_rgba_png(&uncompressed).unwrap(), image);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_regions_feeding_one_parent_preserve_both_updates() {
    let store = Arc::new(MemoryTileStore::new());
    let pyramid = Arc::new(TilePyramid::new(store.clone(), 1, PngOptions::default()).unwrap());
    let left = {
        let pyramid = pyramid.clone();
        tokio::spawn(async move {
            pyramid
                .apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(11, 0, 0, 255)))
                .await
        })
    };
    let right = {
        let pyramid = pyramid.clone();
        tokio::spawn(async move {
            pyramid
                .apply_region(RegionCoord { x: 1, z: 0 }, &solid(rgba(22, 0, 0, 255)))
                .await
        })
    };
    left.await.unwrap().unwrap();
    right.await.unwrap().unwrap();

    let parent = decoded(&store, &path(0, 0, 0)).await;
    assert_eq!(pixel(&parent, 0, 0), rgba(11, 0, 0, 255));
    assert_eq!(pixel(&parent, 255, 255), rgba(11, 0, 0, 255));
    assert_eq!(pixel(&parent, 256, 0), rgba(22, 0, 0, 255));
    assert_eq!(pixel(&parent, 511, 255), rgba(22, 0, 0, 255));
    assert_eq!(pyramid.live_lock_count(), 0);
}

#[tokio::test]
async fn cancellation_while_waiting_reclaims_the_destination_lock() {
    let store = Arc::new(BlockingStore::new());
    let pyramid = Arc::new(TilePyramid::new(store.clone(), 0, PngOptions::default()).unwrap());
    let first = {
        let pyramid = pyramid.clone();
        tokio::spawn(async move {
            pyramid
                .apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(11, 0, 0, 255)))
                .await
        })
    };
    store.read_started.notified().await;
    CancelAfterPending {
        future: Box::pin(pyramid.apply_region(
            RegionCoord { x: 0, z: 0 },
            &solid(rgba(22, 0, 0, 255)),
        )),
    }
    .await;
    assert_eq!(pyramid.live_lock_count(), 1);
    store.release_read.notify_one();
    first.await.unwrap().unwrap();
    assert_eq!(pyramid.live_lock_count(), 0);
}

#[tokio::test]
async fn publish_failures_are_crash_safe_and_directory_sync_is_a_warning() {
    let store = Arc::new(MemoryTileStore::new());
    let pyramid = TilePyramid::new(store.clone(), 0, PngOptions::default()).unwrap();
    let tile_path = path(0, 0, 0);
    let old_bytes = encode_rgba_png(&vec![9_u8; RGBA_BYTES], PngOptions::default()).unwrap();
    store.seed(tile_path.clone(), old_bytes.clone());
    store.fail_next_before_publish(tile_path.clone());

    let error = pyramid
        .apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(1, 2, 3, 255)))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TileError::Publish {
            path: tile_path.clone(),
            message: "injected pre-publish failure".into()
        }
    );
    assert_eq!(store.read(&tile_path).await.unwrap().unwrap(), old_bytes);
    assert_eq!(
        pixel(&decoded(&store, &tile_path).await, 0, 0),
        rgba(9, 9, 9, 9)
    );

    store.warn_next_directory_sync(tile_path.clone());
    let result = pyramid
        .apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(4, 5, 6, 255)))
        .await
        .unwrap();
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.warnings[0].path, tile_path);
    assert_eq!(
        result.warnings[0].message,
        "injected directory sync failure"
    );
    assert_eq!(
        pixel(&decoded(&store, &result.changed_paths[0]).await, 0, 0),
        rgba(4, 5, 6, 255)
    );
    assert_eq!(pyramid.live_lock_count(), 0);
}

#[tokio::test]
async fn existing_png_validation_is_strict_and_zoom_is_bounded() {
    assert_eq!(
        TilePyramid::new(Arc::new(MemoryTileStore::new()), 10, PngOptions::default()).unwrap_err(),
        TileError::InvalidZoom(10)
    );
    TilePyramid::new(Arc::new(MemoryTileStore::new()), 9, PngOptions::default()).unwrap();

    let corrupt = Arc::new(MemoryTileStore::new());
    corrupt.seed(path(0, 0, 0), b"not png".to_vec());
    let pyramid = TilePyramid::new(corrupt, 0, PngOptions::default()).unwrap();
    assert!(matches!(
        pyramid.apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(1, 1, 1, 1))).await,
        Err(TileError::Decode { path, .. }) if path == PathBuf::from("0/0_0.png")
    ));

    let wrong_size = Arc::new(MemoryTileStore::new());
    wrong_size.seed(path(0, 0, 0), encoded_png(16, 16, png::ColorType::Rgba));
    let pyramid = TilePyramid::new(wrong_size, 0, PngOptions::default()).unwrap();
    assert_eq!(
        pyramid
            .apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(1, 1, 1, 1)))
            .await
            .unwrap_err(),
        TileError::InvalidDimensions {
            path: path(0, 0, 0),
            width: 16,
            height: 16
        }
    );

    let rgb = Arc::new(MemoryTileStore::new());
    rgb.seed(path(0, 0, 0), encoded_png(512, 512, png::ColorType::Rgb));
    let pyramid = TilePyramid::new(rgb, 0, PngOptions::default()).unwrap();
    assert_eq!(
        pyramid
            .apply_region(RegionCoord { x: 0, z: 0 }, &solid(rgba(1, 1, 1, 1)))
            .await
            .unwrap_err(),
        TileError::InvalidColorType {
            path: path(0, 0, 0),
            actual: "Rgb".into()
        }
    );
}
