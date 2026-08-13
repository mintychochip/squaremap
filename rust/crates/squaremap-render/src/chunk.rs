use crate::biome::{self, BiomeSource, BiomeSourceError};
use crate::color;
use crate::registry::RegistryGeneration;
use crate::snapshot::{Section, Snapshot};
use std::fmt;
use std::sync::Arc;

const CLEAR: u32 = 0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderSettings {
    pub iterate_up: bool,
    pub map_max_height: i32,
    pub biomes_enabled: bool,
    pub biome_blend: u32,
    pub glass_clear: bool,
    pub water_clear: bool,
    pub water_checkerboard: bool,
    pub lava_checkerboard: bool,
}
impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            iterate_up: false,
            map_max_height: -1,
            biomes_enabled: false,
            biome_blend: 0,
            glass_clear: true,
            water_clear: true,
            water_checkerboard: false,
            lava_checkerboard: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderContextError {
    InvalidMaxHeight,
    InvalidBlend,
    UnknownInvisibleId(u32),
    UnknownIterateBaseId(u32),
    InvalidDescriptor(u32),
    MissingBiomeSource,
    GenerationMismatch,
}
impl fmt::Display for RenderContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RenderContextError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NeighborDirection {
    North,
    South,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    Context(RenderContextError),
    Cancelled,
    IdentityMismatch,
    CoordinateMismatch,
    BoundsMismatch,
    Malformed(&'static str),
    UnknownDescriptor(u32),
    Biome(BiomeSourceError),
    Color,
}
impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RenderError {}
impl From<BiomeSourceError> for RenderError {
    fn from(value: BiomeSourceError) -> Self {
        Self::Biome(value)
    }
}
pub fn validate_neighbor_relation(
    ctx: &RenderContext,
    neighbor: &Snapshot,
    center: &Snapshot,
    direction: NeighborDirection,
) -> Result<(), RenderError> {
    validate_pair(
        ctx,
        neighbor,
        center,
        matches!(direction, NeighborDirection::North),
    )
}

pub struct RenderContext {
    pub(crate) generation: Arc<RegistryGeneration>,
    pub(crate) settings: RenderSettings,
    pub(crate) invisible_ids: Arc<[u32]>,
    pub(crate) iterate_up_base_ids: Arc<[u32]>,
    pub(crate) biome_source: Option<Arc<dyn BiomeSource>>,
    pub(crate) cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl RenderContext {
    pub fn try_new(
        generation: Arc<RegistryGeneration>,
        settings: RenderSettings,
        invisible_ids: impl IntoIterator<Item = u32>,
        iterate_up_base_ids: impl IntoIterator<Item = u32>,
        biome_source: Option<Arc<dyn BiomeSource>>,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<Self, RenderContextError> {
        if settings.map_max_height < -1 {
            return Err(RenderContextError::InvalidMaxHeight);
        }
        if settings.biome_blend > 15 {
            return Err(RenderContextError::InvalidBlend);
        }
        let mut invisible = invisible_ids.into_iter().collect::<Vec<_>>();
        invisible.sort_unstable();
        invisible.dedup();
        let mut iterate = iterate_up_base_ids.into_iter().collect::<Vec<_>>();
        iterate.sort_unstable();
        iterate.dedup();
        for id in &invisible {
            if generation.block(*id).is_none() {
                return Err(RenderContextError::UnknownInvisibleId(*id));
            }
        }
        for id in &iterate {
            if generation.block(*id).is_none() {
                return Err(RenderContextError::UnknownIterateBaseId(*id));
            }
        }
        for id in generation.block_ids() {
            let d = generation.block(id).expect("generation ID set");
            if !(1..=3).contains(&d.transparency)
                || !(1..=4).contains(&d.fluid)
                || d.tint_index > 3
                || (d.glass && !matches!(d.glass_alpha_percent, 25 | 50))
                || (!d.glass && d.glass_alpha_percent != 0)
            {
                return Err(RenderContextError::InvalidDescriptor(id));
            }
        }
        for id in generation.biome_ids() {
            if generation.biome(id).expect("generation ID set").tint_index > 3 {
                return Err(RenderContextError::InvalidDescriptor(id));
            }
        }
        if settings.biomes_enabled {
            let source = biome_source
                .as_ref()
                .ok_or(RenderContextError::MissingBiomeSource)?;
            if !Arc::ptr_eq(source.generation(), &generation) {
                return Err(RenderContextError::GenerationMismatch);
            }
        }
        Ok(Self {
            generation,
            settings,
            invisible_ids: invisible.into(),
            iterate_up_base_ids: iterate.into(),
            biome_source,
            cancelled: Arc::new(cancelled),
        })
    }
    pub fn generation(&self) -> &Arc<RegistryGeneration> {
        &self.generation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkPixels {
    pub pixels: [u32; 256],
    pub south_edge: [i32; 16],
}
impl ChunkPixels {
    pub fn pixel(&self, x: usize, z: usize) -> u32 {
        self.pixels[x * 16 + z]
    }
}

pub fn render_chunk(
    ctx: &RenderContext,
    north: Option<&Snapshot>,
    chunk: &Snapshot,
    south: Option<&Snapshot>,
) -> Result<ChunkPixels, RenderError> {
    render_chunk_seeded(ctx, north, chunk, south, None)
}

pub(crate) fn render_chunk_seeded(
    ctx: &RenderContext,
    north: Option<&Snapshot>,
    chunk: &Snapshot,
    south: Option<&Snapshot>,
    carried: Option<[i32; 16]>,
) -> Result<ChunkPixels, RenderError> {
    if (ctx.cancelled)() {
        return Err(RenderError::Cancelled);
    }
    validate_snapshot(ctx, chunk, true)?;
    if let Some(neighbor) = north {
        validate_neighbor_relation(ctx, neighbor, chunk, NeighborDirection::North)?;
    }
    if let Some(neighbor) = south {
        validate_neighbor_relation(ctx, neighbor, chunk, NeighborDirection::South)?;
    }
    let mut last_y = carried.unwrap_or([0; 16]);
    if carried.is_none() {
        if let Some(neighbor) = north {
            seed_north(ctx, neighbor, &mut last_y)?;
        }
    }
    let mut pixels = [CLEAR; 256];
    for x in 0..16 {
        if (ctx.cancelled)() {
            return Err(RenderError::Cancelled);
        }
        for z in 0..16 {
            let top = chunk.surface.heightmap[x + z * 16];
            if top <= chunk.min_y {
                continue;
            }
            let max_y = effective_max(ctx, chunk)?;
            let Some((id, y)) = select(ctx, chunk, x as i32, z as i32, top.min(max_y))? else {
                continue;
            };
            let shaded = shade(ctx, chunk, id, y, x as i32, z as i32, last_y[x])?;
            pixels[x * 16 + z] = shaded.color;
            if shaded.updates_last_y {
                last_y[x] = shaded.effective_y;
            }
        }
    }
    Ok(ChunkPixels {
        pixels,
        south_edge: last_y,
    })
}
fn validate_snapshot(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    _target: bool,
) -> Result<(), RenderError> {
    if !Arc::ptr_eq(&snapshot.registry_generation, &ctx.generation) {
        return Err(RenderError::IdentityMismatch);
    }
    let world = ctx
        .generation
        .world()
        .ok_or(RenderError::IdentityMismatch)?;
    if snapshot.world.namespace != world.namespace
        || snapshot.world.value != world.value
        || snapshot.world.epoch != world.epoch
        || snapshot.revision != ctx.generation.revision()
    {
        return Err(RenderError::IdentityMismatch);
    }
    if snapshot.sections.is_empty() {
        return Err(RenderError::Malformed("sections"));
    }
    if snapshot.surface.heightmap.len() != 256 {
        return Err(RenderError::Malformed("heightmap"));
    }
    if snapshot.max_y == i32::MAX {
        return Err(RenderError::Malformed("vertical bounds"));
    }
    let max_exclusive = i64::from(snapshot.max_y) + 1;
    if snapshot.surface.heightmap.iter().any(|height| {
        i64::from(*height) < i64::from(snapshot.min_y) || i64::from(*height) > max_exclusive
    }) {
        return Err(RenderError::Malformed("height range"));
    }
    let expected = i64::from(snapshot.max_y) - i64::from(snapshot.min_y) + 1;
    if expected <= 0 || expected % 16 != 0 || snapshot.sections.len() != expected as usize / 16 {
        return Err(RenderError::Malformed("vertical sections"));
    }
    for (index, section) in snapshot.sections.iter().enumerate() {
        let expected_y = i64::from(snapshot.min_y.div_euclid(16)) + index as i64;
        if i64::from(section.section_y) != expected_y
            || section.blocks.len() != 4096
            || section.biomes.len() != 64
            || section.palette.is_empty()
            || section.palette.len() > 4096
            || section.biome_palette.is_empty()
            || section.biome_palette.len() > 64
        {
            return Err(RenderError::Malformed("section"));
        }
        for (palette_index, id) in section.palette.iter().enumerate() {
            if ctx.generation.block(*id).is_none() {
                return Err(RenderError::UnknownDescriptor(*id));
            }
            if section.palette[..palette_index].contains(id) {
                return Err(RenderError::Malformed("palette"));
            }
        }
        for (palette_index, id) in section.biome_palette.iter().enumerate() {
            if ctx.generation.biome(*id).is_none() {
                return Err(RenderError::UnknownDescriptor(*id));
            }
            if section.biome_palette[..palette_index].contains(id) {
                return Err(RenderError::Malformed("palette"));
            }
        }
        for id in &section.blocks {
            if !section.palette.contains(id) {
                return Err(RenderError::Malformed("palette index"));
            }
        }
        for id in &section.biomes {
            if !section.biome_palette.contains(id) {
                return Err(RenderError::Malformed("palette index"));
            }
        }
    }
    Ok(())
}
fn validate_pair(
    ctx: &RenderContext,
    neighbor: &Snapshot,
    target: &Snapshot,
    north: bool,
) -> Result<(), RenderError> {
    validate_snapshot(ctx, neighbor, false)?;
    if neighbor.world.namespace != target.world.namespace
        || neighbor.world.value != target.world.value
        || neighbor.world.epoch != target.world.epoch
        || neighbor.revision != target.revision
        || neighbor.min_y != target.min_y
        || neighbor.max_y != target.max_y
        || neighbor.ceiling != target.ceiling
    {
        return Err(RenderError::BoundsMismatch);
    }
    let expected_z = if north {
        target.coordinate.z.checked_sub(1)
    } else {
        target.coordinate.z.checked_add(1)
    }
    .ok_or(RenderError::CoordinateMismatch)?;
    if neighbor.coordinate.x != target.coordinate.x || neighbor.coordinate.z != expected_z {
        return Err(RenderError::CoordinateMismatch);
    }
    Ok(())
}
fn effective_max(ctx: &RenderContext, chunk: &Snapshot) -> Result<i32, RenderError> {
    if ctx.settings.map_max_height == -1 {
        chunk
            .max_y
            .checked_add(1)
            .ok_or(RenderError::Malformed("vertical bounds"))
    } else {
        Ok(ctx.settings.map_max_height)
    }
}
fn section_for(snapshot: &Snapshot, y: i32) -> Option<&Section> {
    let index = i64::from(y.div_euclid(16)) - i64::from(snapshot.min_y.div_euclid(16));
    usize::try_from(index)
        .ok()
        .and_then(|index| snapshot.sections.get(index))
}
fn block(snapshot: &Snapshot, x: i32, y: i32, z: i32) -> Option<u32> {
    if y < snapshot.min_y || y > snapshot.max_y || !(0..16).contains(&x) || !(0..16).contains(&z) {
        return Some(0);
    }
    section_for(snapshot, y)
        .and_then(|section| {
            section
                .blocks
                .get(((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize)
        })
        .copied()
        .or(Some(0))
}
fn is_clear(ctx: &RenderContext, id: u32) -> bool {
    id == 0
        || ctx
            .generation
            .block(id)
            .is_some_and(|d| d.map_color == CLEAR)
}
fn is_air(ctx: &RenderContext, id: u32) -> bool {
    id == 0 || ctx.generation.block(id).is_some_and(|d| d.air)
}
fn is_invisible(ctx: &RenderContext, id: u32) -> bool {
    ctx.invisible_ids.binary_search(&id).is_ok()
        || ctx.generation.block(id).is_some_and(|descriptor| descriptor.transparency == 3)
}
fn is_iterate_up_base(ctx: &RenderContext, id: u32) -> bool {
    ctx.iterate_up_base_ids.binary_search(&id).is_ok()
        || ctx.generation.block(id).is_some_and(|descriptor| descriptor.iterate_up_base)
}
fn fluid_kind(ctx: &RenderContext, id: u32, color: u32) -> Option<color::FluidKind> {
    ctx.generation.block(id).and_then(|d| match d.fluid {
        1 => None,
        2 => Some(color::FluidKind::Water),
        3 => Some(color::FluidKind::Lava),
        4 => Some(color::classify_unknown_fluid(color, false, false)),
        _ => None,
    })
}
fn is_fluid(ctx: &RenderContext, id: u32) -> bool {
    ctx.generation
        .block(id)
        .is_some_and(|d| (2..=4).contains(&d.fluid))
}
fn is_glass(ctx: &RenderContext, id: u32) -> bool {
    ctx.generation.block(id).is_some_and(|d| d.glass)
}
fn select(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    x: i32,
    z: i32,
    start: i32,
) -> Result<Option<(u32, i32)>, RenderError> {
    let mut y = start;
    let mut id;
    if ctx.settings.iterate_up {
        let height = start;
        y = snapshot.min_y;
        if snapshot.ceiling {
            loop {
                y = y
                    .checked_add(1)
                    .ok_or(RenderError::Malformed("vertical bounds"))?;
                id = block(snapshot, x, y, z).unwrap_or(0);
                if is_air(ctx, id) || y >= height {
                    break;
                }
            }
            loop {
                y = y
                    .checked_add(1)
                    .ok_or(RenderError::Malformed("vertical bounds"))?;
                id = block(snapshot, x, y, z).unwrap_or(0);
                if is_iterate_up_base(ctx, id) || y >= height {
                    break;
                }
            }
        }
        loop {
            if y == i32::MIN {
                return Err(RenderError::Malformed("vertical bounds"));
            }
            y -= 1;
            id = block(snapshot, x, y, z).unwrap_or(0);
            if !(is_clear(ctx, id) || is_invisible(ctx, id)) || y <= snapshot.min_y {
                break;
            }
        }
    } else {
        if snapshot.ceiling {
            loop {
                if y == i32::MIN {
                    return Err(RenderError::Malformed("vertical bounds"));
                }
                y -= 1;
                id = block(snapshot, x, y, z).unwrap_or(0);
                if is_air(ctx, id) || y <= snapshot.min_y {
                    break;
                }
            }
        }
        loop {
            if y == i32::MIN {
                return Err(RenderError::Malformed("vertical bounds"));
            }
            y -= 1;
            id = block(snapshot, x, y, z).unwrap_or(0);
            if !(is_clear(ctx, id) || is_invisible(ctx, id)) || y <= snapshot.min_y {
                break;
            }
        }
    }
    Ok(Some((id, y)))
}
pub(crate) fn seed_bottom_edge(
    ctx: &RenderContext,
    above: &Snapshot,
) -> Result<[i32; 16], RenderError> {
    validate_snapshot(ctx, above, false)?;
    let mut edge = [0; 16];
    seed_north(ctx, above, &mut edge)?;
    Ok(edge)
}
fn seed_north(
    ctx: &RenderContext,
    north: &Snapshot,
    last_y: &mut [i32; 16],
) -> Result<(), RenderError> {
    for x in 0..16 {
        if (ctx.cancelled)() {
            return Err(RenderError::Cancelled);
        }
        let top = north.surface.heightmap[x + 15 * 16];
        let start = top.min(effective_max(ctx, north)?);
        if let Some((id, y)) = select(ctx, north, x as i32, 15, start)? {
            last_y[x] = if ctx.settings.glass_clear && is_glass(ctx, id) {
                glass_y(ctx, north, x as i32, 15, y)?
            } else {
                y
            };
        }
    }
    Ok(())
}
fn select_down(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    x: i32,
    z: i32,
    start: i32,
) -> Result<(u32, i32), RenderError> {
    let mut y = start;
    let mut id;
    if snapshot.ceiling {
        loop {
            if y == i32::MIN {
                return Err(RenderError::Malformed("vertical bounds"));
            }
            y -= 1;
            id = block(snapshot, x, y, z).unwrap_or(0);
            if is_air(ctx, id) || y <= snapshot.min_y {
                break;
            }
        }
    }
    loop {
        if y == i32::MIN {
            return Err(RenderError::Malformed("vertical bounds"));
        }
        y -= 1;
        id = block(snapshot, x, y, z).unwrap_or(0);
        if !(is_clear(ctx, id) || is_invisible(ctx, id)) || y <= snapshot.min_y {
            break;
        }
    }
    Ok((id, y))
}
fn glass_y(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    x: i32,
    z: i32,
    y: i32,
) -> Result<i32, RenderError> {
    let mut current = y;
    while is_glass(ctx, block(snapshot, x, current, z).unwrap_or(0)) {
        current = select_down(ctx, snapshot, x, z, current)?.1;
    }
    Ok(current)
}
pub(crate) struct ShadeResult {
    pub color: u32,
    pub effective_y: i32,
    pub updates_last_y: bool,
}

fn shade(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    mut id: u32,
    mut y: i32,
    x: i32,
    z: i32,
    previous: i32,
) -> Result<ShadeResult, RenderError> {
    if id == 0 {
        return Ok(ShadeResult {
            color: color::terrain(y, previous, CLEAR, color::checkerboard_parity(x, z)),
            effective_y: y,
            updates_last_y: true,
        });
    }
    let descriptor = ctx
        .generation
        .block(id)
        .ok_or(RenderError::UnknownDescriptor(id))?;
    let glass_color = if ctx.settings.glass_clear && descriptor.glass {
        let value = descriptor.map_color;
        y = glass_y(ctx, snapshot, x, z, y)?;
        id = block(snapshot, x, y, z).unwrap_or(0);
        Some((value, descriptor.glass_alpha_percent))
    } else {
        None
    };
    if id == 0 {
        let (glass, alpha) = glass_color.ok_or(RenderError::Malformed("glass descent"))?;
        let under = color::terrain(y, previous, CLEAR, color::checkerboard_parity(x, z));
        let color = color::glass_composite(under, glass, alpha as f32 / 100.0)
            .map_err(|_| RenderError::Color)?;
        return Ok(ShadeResult {
            color,
            effective_y: y,
            updates_last_y: true,
        });
    }
    let descriptor = ctx
        .generation
        .block(id)
        .ok_or(RenderError::UnknownDescriptor(id))?;
    let mut color = descriptor.map_color;
    if ctx.settings.biomes_enabled && descriptor.tint_index != 0 {
        let world_x = snapshot
            .coordinate
            .x
            .checked_mul(16)
            .and_then(|value| value.checked_add(x))
            .ok_or(RenderError::Malformed("coordinate arithmetic"))?;
        let world_z = snapshot
            .coordinate
            .z
            .checked_mul(16)
            .and_then(|value| value.checked_add(z))
            .ok_or(RenderError::Malformed("coordinate arithmetic"))?;
        color = biome::blend_color(
            ctx.biome_source
                .as_ref()
                .ok_or(RenderError::Context(RenderContextError::MissingBiomeSource))?
                .as_ref(),
            &ctx.generation,
            descriptor.tint_index,
            color,
            world_x,
            y,
            world_z,
            ctx.settings.biome_blend,
        )?;
    }
    let odd = color::checkerboard_parity(x, z);
    let mut updates_last_y = true;
    if is_fluid(ctx, id) && y > snapshot.min_y {
        updates_last_y = false;
        let mut depth = 0u8;
        let mut below = y
            .checked_sub(1)
            .ok_or(RenderError::Malformed("vertical bounds"))?;
        let under = loop {
            depth = depth.saturating_add(1);
            let candidate = block(snapshot, x, below, z).unwrap_or(0);
            if depth > 10 || !is_fluid(ctx, candidate) {
                break candidate;
            }
            let next = below
                .checked_sub(1)
                .ok_or(RenderError::Malformed("vertical bounds"))?;
            if next <= snapshot.min_y {
                break candidate;
            }
            below = next;
        };
        let kind = fluid_kind(ctx, id, color).ok_or(RenderError::Malformed("fluid class"))?;
        let flags = color::FluidFlags {
            water_checkerboard: ctx.settings.water_checkerboard,
            water_clear: ctx.settings.water_clear,
            lava_checkerboard: ctx.settings.lava_checkerboard,
        };
        color = color::compose_fluid(
            depth,
            color,
            ctx.generation
                .block(under)
                .map(|d| d.map_color)
                .unwrap_or(CLEAR),
            kind,
            odd,
            flags,
        )
        .map_err(|_| RenderError::Color)?;
    } else {
        color = color::terrain(y, previous, color, odd);
    }
    if let Some((glass, alpha)) = glass_color {
        color = color::glass_composite(color, glass, alpha as f32 / 100.0)
            .map_err(|_| RenderError::Color)?;
    }
    Ok(ShadeResult {
        color,
        effective_y: y,
        updates_last_y,
    })
}
pub(crate) fn select_for_region(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    x: i32,
    z: i32,
    start: i32,
) -> Result<Option<(u32, i32)>, RenderError> {
    select(ctx, snapshot, x, z, start)
}
pub(crate) fn shade_for_region(
    ctx: &RenderContext,
    snapshot: &Snapshot,
    id: u32,
    y: i32,
    x: i32,
    z: i32,
    previous: i32,
) -> Result<ShadeResult, RenderError> {
    shade(ctx, snapshot, id, y, x, z, previous)
}
pub(crate) fn validate_for_region(
    ctx: &RenderContext,
    snapshot: &Snapshot,
) -> Result<(), RenderError> {
    validate_snapshot(ctx, snapshot, false)
}
pub(crate) fn cancelled_for_region(ctx: &RenderContext) -> bool {
    (ctx.cancelled)()
}
