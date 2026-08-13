use crate::color;
use crate::registry::{GenerationToken, RegistryGeneration};
use crate::snapshot::Snapshot;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BiomeSourceError {
    Missing { x: i32, y: i32, z: i32 },
    InvalidBiome(u32),
    UnsupportedSelector,
    GenerationMismatch,
    SnapshotMetadataMismatch,
}
impl fmt::Display for BiomeSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BiomeSourceError {}

pub trait BiomeSource: Send + Sync {
    fn sample_block(&self, x: i32, y: i32, z: i32) -> Result<u32, BiomeSourceError>;
    fn generation(&self) -> &GenerationToken;
    fn resolved_grass(
        &self,
        x: i32,
        y: i32,
        z: i32,
        selected_biome: u32,
    ) -> Result<u32, BiomeSourceError>;
}
pub trait QuartBiomeSource: Send + Sync {
    fn sample_quart(
        &self,
        quart_x: i32,
        quart_y: i32,
        quart_z: i32,
    ) -> Result<u32, BiomeSourceError>;
    fn generation(&self) -> &GenerationToken;
}

#[derive(Clone)]
pub struct StaticBiomeSource {
    generation: GenerationToken,
    samples: HashMap<(i32, i32, i32), u32>,
    fallback: Option<u32>,
    grass: HashMap<(i32, i32, i32, u32), u32>,
}
impl StaticBiomeSource {
    pub fn new(generation: GenerationToken) -> Self {
        Self {
            generation,
            samples: HashMap::new(),
            fallback: None,
            grass: HashMap::new(),
        }
    }
    pub fn with_sample(mut self, x: i32, y: i32, z: i32, biome: u32) -> Self {
        self.samples.insert((x, y, z), biome);
        self
    }
    pub fn with_fallback(mut self, biome: u32) -> Self {
        self.fallback = Some(biome);
        self
    }
    pub fn with_grass_resolved(mut self, x: i32, y: i32, z: i32, biome: u32, color: u32) -> Self {
        self.grass.insert((x, y, z, biome), color);
        self
    }
}
impl BiomeSource for StaticBiomeSource {
    fn sample_block(&self, x: i32, y: i32, z: i32) -> Result<u32, BiomeSourceError> {
        self.samples
            .get(&(x, y, z))
            .copied()
            .or(self.fallback)
            .ok_or(BiomeSourceError::Missing { x, y, z })
    }
    fn generation(&self) -> &GenerationToken {
        &self.generation
    }
    fn resolved_grass(
        &self,
        x: i32,
        y: i32,
        z: i32,
        selected_biome: u32,
    ) -> Result<u32, BiomeSourceError> {
        self.grass
            .get(&(x, y, z, selected_biome))
            .copied()
            .ok_or(BiomeSourceError::UnsupportedSelector)
    }
}

#[derive(Clone)]
pub struct SnapshotBiomeSource {
    generation: GenerationToken,
    biome_zoom_seed: i64,
    snapshots: HashMap<(i32, i32), Arc<Snapshot>>,
    fallback: Option<u32>,
    grass: HashMap<(i32, i32, i32, u32), u32>,
}
impl SnapshotBiomeSource {
    pub fn new(generation: GenerationToken, biome_zoom_seed: i64) -> Self {
        Self {
            generation,
            biome_zoom_seed,
            snapshots: HashMap::new(),
            fallback: None,
            grass: HashMap::new(),
        }
    }
    pub fn with_snapshot(mut self, snapshot: Arc<Snapshot>) -> Result<Self, BiomeSourceError> {
        if !Arc::ptr_eq(&snapshot.registry_generation, &self.generation) {
            return Err(BiomeSourceError::GenerationMismatch);
        }
        let Some(world) = self.generation.world() else {
            return Err(BiomeSourceError::SnapshotMetadataMismatch);
        };
        if snapshot.revision != self.generation.revision()
            || snapshot.world.namespace != world.namespace
            || snapshot.world.value != world.value
            || snapshot.world.epoch != world.epoch
        {
            return Err(BiomeSourceError::SnapshotMetadataMismatch);
        }
        self.snapshots
            .insert((snapshot.coordinate.x, snapshot.coordinate.z), snapshot);
        Ok(self)
    }
    pub fn with_fallback(mut self, biome: u32) -> Self {
        self.fallback = Some(biome);
        self
    }
    pub fn with_grass_resolved(mut self, x: i32, y: i32, z: i32, biome: u32, color: u32) -> Self {
        self.grass.insert((x, y, z, biome), color);
        self
    }
    pub fn snapshots(&self) -> Vec<Arc<Snapshot>> {
        self.snapshots.values().cloned().collect()
    }
}
impl QuartBiomeSource for SnapshotBiomeSource {
    fn sample_quart(
        &self,
        quart_x: i32,
        quart_y: i32,
        quart_z: i32,
    ) -> Result<u32, BiomeSourceError> {
        let chunk_x = quart_x.div_euclid(4);
        let chunk_z = quart_z.div_euclid(4);
        let Some(snapshot) = self.snapshots.get(&(chunk_x, chunk_z)) else {
            return self.fallback.ok_or(BiomeSourceError::Missing {
                x: quart_x,
                y: quart_y,
                z: quart_z,
            });
        };
        let height = i64::from(snapshot.max_y) - i64::from(snapshot.min_y) + 1;
        if height <= 0 || height > i64::from(i32::MAX) {
            return Err(BiomeSourceError::UnsupportedSelector);
        }
        let quart_y = clamped_quart_y(quart_y, snapshot.min_y, height)?;
        let section_index =
            i64::from(quart_y.div_euclid(4)) - i64::from(snapshot.min_y.div_euclid(16));
        let Some(section) = usize::try_from(section_index)
            .ok()
            .and_then(|index| snapshot.sections.get(index))
        else {
            return self.fallback.ok_or(BiomeSourceError::Missing {
                x: quart_x,
                y: quart_y,
                z: quart_z,
            });
        };
        let index = (quart_y.rem_euclid(4) * 16 + quart_z.rem_euclid(4) * 4 + quart_x.rem_euclid(4))
            as usize;
        section
            .biomes
            .get(index)
            .copied()
            .or(self.fallback)
            .ok_or(BiomeSourceError::Missing {
                x: quart_x,
                y: quart_y,
                z: quart_z,
            })
    }
    fn generation(&self) -> &GenerationToken {
        &self.generation
    }
}
impl BiomeSource for SnapshotBiomeSource {
    fn sample_block(&self, x: i32, y: i32, z: i32) -> Result<u32, BiomeSourceError> {
        sample_block_with_quart(self, self.biome_zoom_seed, x, y, z)
    }
    fn generation(&self) -> &GenerationToken {
        &self.generation
    }
    fn resolved_grass(
        &self,
        x: i32,
        y: i32,
        z: i32,
        selected_biome: u32,
    ) -> Result<u32, BiomeSourceError> {
        self.grass
            .get(&(x, y, z, selected_biome))
            .copied()
            .ok_or(BiomeSourceError::UnsupportedSelector)
    }
}

const LCG_MULTIPLIER: i64 = 6_364_136_223_846_793_005;
const LCG_INCREMENT: i64 = 1_442_695_040_888_963_407;
fn lcg_next(rval: i64, c: i64) -> i64 {
    rval.wrapping_mul(
        rval.wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT),
    )
    .wrapping_add(c)
}
fn sample_block_with_quart<S: QuartBiomeSource + ?Sized>(
    source: &S,
    seed: i64,
    x: i32,
    y: i32,
    z: i32,
) -> Result<u32, BiomeSourceError> {
    let (quart_x, quart_y, quart_z) = select_quart(seed, x, y, z);
    source.sample_quart(quart_x, quart_y, quart_z)
}
fn get_fiddle(rval: i64) -> f64 {
    ((rval >> 24).rem_euclid(1024) as f64 / 1024.0 - 0.5) * 0.9
}
fn fiddled_distance(seed: i64, x: i32, y: i32, z: i32, dx: f64, dy: f64, dz: f64) -> f64 {
    let mut rval = lcg_next(seed, i64::from(x));
    rval = lcg_next(rval, i64::from(y));
    rval = lcg_next(rval, i64::from(z));
    rval = lcg_next(rval, i64::from(x));
    rval = lcg_next(rval, i64::from(y));
    rval = lcg_next(rval, i64::from(z));
    let fiddle_x = get_fiddle(rval);
    rval = lcg_next(rval, seed);
    let fiddle_y = get_fiddle(rval);
    rval = lcg_next(rval, seed);
    let fiddle_z = get_fiddle(rval);
    (dz + fiddle_z).powi(2) + (dy + fiddle_y).powi(2) + (dx + fiddle_x).powi(2)
}
fn select_quart(seed: i64, x: i32, y: i32, z: i32) -> (i32, i32, i32) {
    let abs_x = x.wrapping_sub(2);
    let abs_y = y.wrapping_sub(2);
    let abs_z = z.wrapping_sub(2);
    let parent_x = abs_x >> 2;
    let parent_y = abs_y >> 2;
    let parent_z = abs_z >> 2;
    let fract_x = f64::from(abs_x & 3) / 4.0;
    let fract_y = f64::from(abs_y & 3) / 4.0;
    let fract_z = f64::from(abs_z & 3) / 4.0;
    let mut min_i = 0;
    let mut min_distance = f64::INFINITY;
    for i in 0..8 {
        let x_even = i & 4 == 0;
        let y_even = i & 2 == 0;
        let z_even = i & 1 == 0;
        let corner_x = if x_even {
            parent_x
        } else {
            parent_x.wrapping_add(1)
        };
        let corner_y = if y_even {
            parent_y
        } else {
            parent_y.wrapping_add(1)
        };
        let corner_z = if z_even {
            parent_z
        } else {
            parent_z.wrapping_add(1)
        };
        let dx = if x_even { fract_x } else { fract_x - 1.0 };
        let dy = if y_even { fract_y } else { fract_y - 1.0 };
        let dz = if z_even { fract_z } else { fract_z - 1.0 };
        let next = fiddled_distance(seed, corner_x, corner_y, corner_z, dx, dy, dz);
        if min_distance > next {
            min_i = i;
            min_distance = next;
        }
    }
    (
        if min_i & 4 == 0 {
            parent_x
        } else {
            parent_x.wrapping_add(1)
        },
        if min_i & 2 == 0 {
            parent_y
        } else {
            parent_y.wrapping_add(1)
        },
        if min_i & 1 == 0 {
            parent_z
        } else {
            parent_z.wrapping_add(1)
        },
    )
}

pub(crate) fn blend_color<S: BiomeSource + ?Sized>(
    source: &S,
    generation: &RegistryGeneration,
    category: u32,
    base: u32,
    x: i32,
    y: i32,
    z: i32,
    radius: u32,
) -> Result<u32, BiomeSourceError> {
    if category == 0 {
        return Ok(base);
    }
    let mut values = [0u32; 900];
    let mut count = 0usize;
    if radius == 0 {
        let selected = source.sample_block(x, y, z)?;
        values[0] = biome_color(source, generation, x, y, z, selected, category)?;
        count = 1;
    } else {
        if radius > 15 {
            return Err(BiomeSourceError::UnsupportedSelector);
        }
        let r = i32::try_from(radius).map_err(|_| BiomeSourceError::UnsupportedSelector)?;
        let start_x = x
            .checked_sub(r)
            .ok_or(BiomeSourceError::UnsupportedSelector)?;
        let end_x = x
            .checked_add(r)
            .ok_or(BiomeSourceError::UnsupportedSelector)?;
        let start_z = z
            .checked_sub(r)
            .ok_or(BiomeSourceError::UnsupportedSelector)?;
        let end_z = z
            .checked_add(r)
            .ok_or(BiomeSourceError::UnsupportedSelector)?;
        for sx in start_x..end_x {
            for sz in start_z..end_z {
                let selected = source.sample_block(sx, y, sz)?;
                values[count] = biome_color(source, generation, sx, y, sz, selected, category)?;
                count += 1;
            }
        }
    }
    let sampled =
        color::average_argb(&values[..count]).map_err(|_| BiomeSourceError::UnsupportedSelector)?;
    if category == 3 {
        color::mix(base, sampled, 0.8).map_err(|_| BiomeSourceError::UnsupportedSelector)
    } else {
        Ok(sampled)
    }
}
fn clamped_quart_y(quart_y: i32, min_y: i32, height: i64) -> Result<i32, BiomeSourceError> {
    let min_quart = i64::from(min_y).div_euclid(4);
    let quart_height = height.div_euclid(4);
    if quart_height <= 0 {
        return Err(BiomeSourceError::UnsupportedSelector);
    }
    let max_quart = min_quart
        .checked_add(quart_height - 1)
        .ok_or(BiomeSourceError::UnsupportedSelector)?;
    i32::try_from(i64::from(quart_y).clamp(min_quart, max_quart))
        .map_err(|_| BiomeSourceError::UnsupportedSelector)
}
fn biome_color<S: BiomeSource + ?Sized>(
    source: &S,
    generation: &RegistryGeneration,
    x: i32,
    y: i32,
    z: i32,
    id: u32,
    category: u32,
) -> Result<u32, BiomeSourceError> {
    if category == 1 {
        return source.resolved_grass(x, y, z, id);
    }
    let d = generation
        .biome(id)
        .ok_or(BiomeSourceError::InvalidBiome(id))?;
    match category {
        2 => Ok(d.foliage_color),
        3 => Ok(d.water_color),
        _ => Err(BiomeSourceError::UnsupportedSelector),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        QuartBiomeSource, SnapshotBiomeSource, StaticBiomeSource, blend_color,
        sample_block_with_quart,
    };
    use crate::registry::Registry;
    use crate::snapshot::{Coordinate, Section, Snapshot, SurfaceHeightmap, World};
    use squaremap_protocol::wire::{BiomeDescriptor, RegistryReplace, WorldIdentity};
    use std::sync::Arc;

    #[test]
    fn quart_y_is_clamped_to_snapshot_vertical_bounds() {
        assert_eq!(super::clamped_quart_y(-100, -16, 32).unwrap(), -4);
        assert_eq!(super::clamped_quart_y(100, -16, 32).unwrap(), 3);
        assert_eq!(super::clamped_quart_y(-1, -16, 32).unwrap(), -1);
    }

    #[test]
    fn java_block_to_quart_vectors_match_seed_oracle() {
        let vectors = [
            ([-65, -33, 31], [-17, -9, 7]),
            ([-17, -1, 15], [-5, -1, 3]),
            ([-16, 0, 16], [-4, -1, 3]),
            ([-15, 1, 17], [-4, 0, 4]),
            ([-3, -3, -3], [-1, -2, -1]),
            ([-2, -2, -2], [-1, -1, -1]),
            ([-1, -1, -1], [-1, -1, -1]),
            ([0, 0, 0], [-1, -1, -1]),
            ([1, 1, 1], [0, 0, 0]),
            ([2, 2, 2], [0, 0, 0]),
            ([3, 3, 3], [0, 0, 1]),
            ([4, 4, 4], [1, 1, 1]),
            ([5, 5, 5], [1, 1, 1]),
            ([15, 63, -17], [3, 15, -5]),
            ([16, 64, -16], [3, 16, -4]),
            ([17, 65, -15], [4, 16, -4]),
            ([31, -64, 32], [7, -17, 8]),
            ([32, -63, 33], [8, -16, 8]),
            ([33, -62, 34], [8, -16, 8]),
            ([i32::MIN, 0, i32::MAX], [536_870_911, -1, 536_870_912]),
            ([i32::MAX, 0, i32::MIN], [536_870_911, -1, 536_870_912]),
        ];
        for (block, expected) in vectors {
            assert_eq!(
                super::select_quart(24_301, block[0], block[1], block[2]),
                (expected[0], expected[1], expected[2]),
                "block={block:?}",
            );
        }
    }
    #[test]
    fn all_selector_vectors_reach_heterogeneous_source_ids() {
        struct Recording {
            generation: crate::registry::GenerationToken,
            requests: std::sync::Mutex<Vec<(i32, i32, i32)>>,
        }
        impl QuartBiomeSource for Recording {
            fn sample_quart(&self, x: i32, y: i32, z: i32) -> Result<u32, crate::BiomeSourceError> {
                self.requests.lock().unwrap().push((x, y, z));
                let source_x = x.div_euclid(4);
                let source_z = z.div_euclid(4);
                Ok(if source_x == 1 && source_z == 0 {
                    11
                } else if source_x == -1 {
                    12
                } else if source_z == 1 {
                    14
                } else {
                    13
                })
            }
            fn generation(&self) -> &crate::registry::GenerationToken {
                &self.generation
            }
        }
        let registry = Registry::with_replace(RegistryReplace {
            biomes: vec![
                BiomeDescriptor {
                    id: 11,
                    ..Default::default()
                },
                BiomeDescriptor {
                    id: 12,
                    ..Default::default()
                },
                BiomeDescriptor {
                    id: 13,
                    ..Default::default()
                },
                BiomeDescriptor {
                    id: 14,
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
        let source = Recording {
            generation: registry.generation(),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let vectors: [([i32; 3], [i32; 3]); 21] = [
            ([-65, -33, 31], [-17, -9, 7]),
            ([-17, -1, 15], [-5, -1, 3]),
            ([-16, 0, 16], [-4, -1, 3]),
            ([-15, 1, 17], [-4, 0, 4]),
            ([-3, -3, -3], [-1, -2, -1]),
            ([-2, -2, -2], [-1, -1, -1]),
            ([-1, -1, -1], [-1, -1, -1]),
            ([0, 0, 0], [-1, -1, -1]),
            ([1, 1, 1], [0, 0, 0]),
            ([2, 2, 2], [0, 0, 0]),
            ([3, 3, 3], [0, 0, 1]),
            ([4, 4, 4], [1, 1, 1]),
            ([5, 5, 5], [1, 1, 1]),
            ([15, 63, -17], [3, 15, -5]),
            ([16, 64, -16], [3, 16, -4]),
            ([17, 65, -15], [4, 16, -4]),
            ([31, -64, 32], [7, -17, 8]),
            ([32, -63, 33], [8, -16, 8]),
            ([33, -62, 34], [8, -16, 8]),
            ([i32::MIN, 0, i32::MAX], [536_870_911, -1, 536_870_912]),
            ([i32::MAX, 0, i32::MIN], [536_870_911, -1, 536_870_912]),
        ];
        for (block, expected_quart) in vectors {
            let selected =
                sample_block_with_quart(&source, 24_301, block[0], block[1], block[2]).unwrap();
            let source_x = expected_quart[0].div_euclid(4);
            let source_z = expected_quart[2].div_euclid(4);
            let expected = if source_x == 1 && source_z == 0 {
                11
            } else if source_x == -1 {
                12
            } else if source_z == 1 {
                14
            } else {
                13
            };
            assert_eq!(selected, expected, "block={block:?}");
        }
        let requests = source.requests.lock().unwrap();
        let expected_requests: Vec<_> = vectors
            .iter()
            .map(|(_, quart)| (quart[0], quart[1], quart[2]))
            .collect();
        assert_eq!(requests.as_slice(), expected_requests.as_slice());
    }
    #[test]
    fn blend_range_at_coordinate_boundary_returns_structured_error() {
        let registry = Registry::with_replace(RegistryReplace {
            revision: 1,
            biomes: vec![BiomeDescriptor {
                id: 1,
                grass_color: 0x00abcdef,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        let generation = registry.generation();
        let source = StaticBiomeSource::new(generation.clone()).with_fallback(1);
        assert_eq!(
            blend_color(&source, &generation, 1, 0x00112233, i32::MAX, 0, 0, 1),
            Err(crate::BiomeSourceError::UnsupportedSelector),
        );
    }

    #[test]
    fn snapshot_biome_extreme_vertical_span_returns_structured_error() {
        let world = WorldIdentity {
            namespace: "test".into(),
            value: "world".into(),
            epoch: 1,
        };
        let registry = Registry::with_replace(RegistryReplace {
            world: Some(world.clone()),
            revision: 1,
            biomes: vec![BiomeDescriptor {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        let generation = registry.generation();
        let snapshot = Arc::new(Snapshot {
            world: World {
                namespace: world.namespace,
                value: world.value,
                epoch: world.epoch,
            },
            coordinate: Coordinate { x: 0, z: 0 },
            min_y: i32::MIN,
            max_y: i32::MAX,
            ceiling: false,
            revision: 1,
            sections: vec![Section {
                section_y: 0,
                palette: vec![1],
                blocks: vec![1; 4096],
                biome_palette: vec![1],
                biomes: vec![1; 64],
            }],
            surface: SurfaceHeightmap { heightmap: vec![] },
            registry_generation: generation.clone(),
        });
        let source = SnapshotBiomeSource::new(generation, 24_301)
            .with_snapshot(snapshot)
            .unwrap()
            .with_fallback(1);
        assert_eq!(
            source.sample_quart(0, 0, 0),
            Err(crate::BiomeSourceError::UnsupportedSelector)
        );
    }

    #[test]
    fn snapshot_source_rejects_distinct_generation_with_same_metadata() {
        let world = WorldIdentity {
            namespace: "test".into(),
            value: "world".into(),
            epoch: 1,
        };
        let replace = RegistryReplace {
            world: Some(world.clone()),
            revision: 1,
            biomes: vec![BiomeDescriptor {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let first = Registry::with_replace(replace.clone()).unwrap();
        let second = Registry::with_replace(replace).unwrap();
        let snapshot = Arc::new(Snapshot {
            world: World {
                namespace: world.namespace,
                value: world.value,
                epoch: world.epoch,
            },
            coordinate: Coordinate { x: 0, z: 0 },
            min_y: 0,
            max_y: 15,
            ceiling: false,
            revision: 1,
            sections: vec![Section {
                section_y: 0,
                palette: vec![1],
                blocks: vec![1; 4096],
                biome_palette: vec![1],
                biomes: vec![1; 64],
            }],
            surface: SurfaceHeightmap {
                heightmap: vec![0; 256],
            },
            registry_generation: first.generation(),
        });
        assert!(matches!(
            SnapshotBiomeSource::new(second.generation(), 24_301).with_snapshot(snapshot),
            Err(crate::BiomeSourceError::GenerationMismatch),
        ));
    }
    #[test]
    fn snapshot_source_rejects_foreign_world_or_revision_with_same_generation() {
        let world = WorldIdentity {
            namespace: "test".into(),
            value: "world".into(),
            epoch: 1,
        };
        let registry = Registry::with_replace(RegistryReplace {
            world: Some(world.clone()),
            revision: 1,
            biomes: vec![BiomeDescriptor {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        let snapshot = Arc::new(Snapshot {
            world: World {
                namespace: "other".into(),
                value: "world".into(),
                epoch: 1,
            },
            coordinate: Coordinate { x: 0, z: 0 },
            min_y: 0,
            max_y: 15,
            ceiling: false,
            revision: 2,
            sections: vec![Section {
                section_y: 0,
                palette: vec![1],
                blocks: vec![1; 4096],
                biome_palette: vec![1],
                biomes: vec![1; 64],
            }],
            surface: SurfaceHeightmap {
                heightmap: vec![0; 256],
            },
            registry_generation: registry.generation(),
        });
        assert!(matches!(
            SnapshotBiomeSource::new(registry.generation(), 24_301).with_snapshot(snapshot),
            Err(crate::BiomeSourceError::SnapshotMetadataMismatch)
        ));
    }
}
