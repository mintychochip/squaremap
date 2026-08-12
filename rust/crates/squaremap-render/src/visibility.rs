//! Minecraft-independent visibility geometry with Java-compatible coarse semantics.

use crate::coordinates::{block_to_chunk, block_to_region, chunk_to_block, region_to_block, region_to_chunk};
use squaremap_protocol::wire::{VisibilityLimit as WireLimit, VisibilityLimitKind};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VisibilityError {
    InvalidRectangle,
    InvalidCircle,
    InvalidPolygon,
    InvalidWorldBorder,
    CountOverflow,
    QueryOverflow,
    UnsupportedWireKind,
    WorldBorderRuntimeRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rectangle {
    min_x: i32, min_z: i32, max_x: i32, max_z: i32,
    min_chunk_x: i32, min_chunk_z: i32, max_chunk_x: i32, max_chunk_z: i32,
    min_region_x: i32, min_region_z: i32, max_region_x: i32, max_region_z: i32,
}
impl Rectangle {
    pub fn new(min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> Result<Self, VisibilityError> {
        if min_x >= max_x || min_z >= max_z { return Err(VisibilityError::InvalidRectangle); }
        Ok(Self { min_x, min_z, max_x, max_z,
            min_chunk_x: block_to_chunk(min_x), min_chunk_z: block_to_chunk(min_z),
            max_chunk_x: block_to_chunk(max_x), max_chunk_z: block_to_chunk(max_z),
            min_region_x: block_to_region(min_x), min_region_z: block_to_region(min_z),
            max_region_x: block_to_region(max_x), max_region_z: block_to_region(max_z) })
    }
    pub fn contains_block(&self, x: i32, z: i32) -> bool { x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z }
    pub fn contains_chunk(&self, x: i32, z: i32) -> bool { x >= self.min_chunk_x && x <= self.max_chunk_x && z >= self.min_chunk_z && z <= self.max_chunk_z }
    pub fn contains_region(&self, x: i32, z: i32) -> bool { x >= self.min_region_x && x <= self.max_region_x && z >= self.min_region_z && z <= self.max_region_z }
    fn count(&self, region_x: i32, region_z: i32) -> Result<u16, VisibilityError> {
        let (min_x, max_x) = region_chunk_span(region_x)?;
        let (min_z, max_z) = region_chunk_span(region_z)?;
        let width = (max_x.min(i64::from(self.max_chunk_x)) - min_x.max(i64::from(self.min_chunk_x)) + 1).max(0);
        let height = (max_z.min(i64::from(self.max_chunk_z)) - min_z.max(i64::from(self.min_chunk_z)) + 1).max(0);
        u16::try_from(width * height).map_err(|_| VisibilityError::CountOverflow)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Circle { center_x: i32, center_z: i32, radius: i32, radius_sq: i64 }
impl Circle {
    pub fn new(center_x: i32, center_z: i32, radius: i32) -> Result<Self, VisibilityError> {
        if !(1..=46_340).contains(&radius) { return Err(VisibilityError::InvalidCircle); }
        Ok(Self { center_x, center_z, radius, radius_sq: i64::from(radius) * i64::from(radius) })
    }
    fn distance_ok(&self, x: i64, z: i64) -> bool {
        let dx = x - i64::from(self.center_x); let dz = z - i64::from(self.center_z);
        dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz)) <= self.radius_sq
    }
    pub fn contains_block(&self, x: i32, z: i32) -> bool { self.distance_ok(i64::from(x), i64::from(z)) }
    fn nearest(&self, x: i32, z: i32, extent: i64) -> bool {
        let Ok(bx) = chunk_to_block(x) else { return false };
        let Ok(bz) = chunk_to_block(z) else { return false };
        self.nearest_blocks(i64::from(bx), i64::from(bz), extent)
    }
    fn nearest_region(&self, x: i32, z: i32) -> bool {
        let Ok(bx) = region_to_block(x) else { return false };
        let Ok(bz) = region_to_block(z) else { return false };
        self.nearest_blocks(i64::from(bx), i64::from(bz), 511)
    }
    fn nearest_blocks(&self, mut x: i64, mut z: i64, extent: i64) -> bool {
        if x < i64::from(self.center_x) { x += (i64::from(self.center_x) - x).min(extent); }
        if z < i64::from(self.center_z) { z += (i64::from(self.center_z) - z).min(extent); }
        self.distance_ok(x, z)
    }
    pub fn contains_chunk(&self, x: i32, z: i32) -> bool { self.nearest(x, z, 15) }
    pub fn contains_region(&self, x: i32, z: i32) -> bool { self.nearest_region(x, z) }
    fn count(&self, region_x: i32, region_z: i32) -> Result<u16, VisibilityError> {
        let (sx, _) = region_chunk_span(region_x)?; let (sz, _) = region_chunk_span(region_z)?;
        let sx = i32::try_from(sx).map_err(|_| VisibilityError::CountOverflow)?;
        let sz = i32::try_from(sz).map_err(|_| VisibilityError::CountOverflow)?;
        if self.contains_chunk(sx, sz) && self.contains_chunk(sx + 31, sz)
            && self.contains_chunk(sx, sz + 31) && self.contains_chunk(sx + 31, sz + 31) { return Ok(1024); }
        let mut count = 0u16;
        for x in 0..32 { for z in 0..32 { if self.contains_chunk(sx + x, sz + z) { count += 1; } } }
        Ok(count)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Polygon { points: Box<[(i32, i32)]>, min_x: i32, min_z: i32, width: i32, height: i32 }
impl Polygon {
    pub fn new(points: Vec<(i32, i32)>) -> Result<Self, VisibilityError> {
        if points.len() < 3 { return Err(VisibilityError::InvalidPolygon); }
        let (min_x, max_x) = points.iter().map(|p| p.0).fold((i32::MAX, i32::MIN), |(a,b),x| (a.min(x),b.max(x)));
        let (min_z, max_z) = points.iter().map(|p| p.1).fold((i32::MAX, i32::MIN), |(a,b),x| (a.min(x),b.max(x)));
        let width = i64::from(max_x) - i64::from(min_x); let height = i64::from(max_z) - i64::from(min_z);
        if width > i64::from(i32::MAX) || height > i64::from(i32::MAX) { return Err(VisibilityError::InvalidPolygon); }
        Ok(Self { points: points.into_boxed_slice(), min_x, min_z, width: width as i32, height: height as i32 })
    }
    fn contains_i64(&self, x: i64, y: i64) -> bool {
        if self.points.len() <= 2 || x < i64::from(self.min_x) || y < i64::from(self.min_z)
            || x >= i64::from(self.min_x) + i64::from(self.width)
            || y >= i64::from(self.min_z) + i64::from(self.height) { return false; }
        let x = x as f64; let y = y as f64; let n = self.points.len(); let mut hits = 0;
        let (mut lastx, mut lasty) = self.points[n - 1];
        for &(curx, cury) in self.points.iter() {
            if cury != lasty {
                let leftx;
                if curx < lastx { if x >= f64::from(lastx) { lastx = curx; lasty = cury; continue; } leftx = curx; }
                else { if x >= f64::from(curx) { lastx = curx; lasty = cury; continue; } leftx = lastx; }
                let (test1, test2);
                if cury < lasty {
                    if y < f64::from(cury) || y >= f64::from(lasty) { lastx = curx; lasty = cury; continue; }
                    if x < f64::from(leftx) { hits += 1; lastx = curx; lasty = cury; continue; }
                    test1 = x - f64::from(curx); test2 = y - f64::from(cury);
                } else {
                    if y < f64::from(lasty) || y >= f64::from(cury) { lastx = curx; lasty = cury; continue; }
                    if x < f64::from(leftx) { hits += 1; lastx = curx; lasty = cury; continue; }
                    test1 = x - f64::from(lastx); test2 = y - f64::from(lasty);
                }
                let denominator = f64::from(lasty.wrapping_sub(cury));
                let delta_x = f64::from(lastx.wrapping_sub(curx));
                if test1 < test2 / denominator * delta_x { hits += 1; }
            }
            lastx = curx; lasty = cury;
        }
        hits & 1 != 0
    }
    pub fn contains_block(&self, x: i32, z: i32) -> bool { self.contains_i64(i64::from(x), i64::from(z)) }
    pub fn contains_chunk(&self, x: i32, z: i32) -> bool {
        let Ok(min_x) = chunk_to_block(x) else { return false }; let Ok(min_z) = chunk_to_block(z) else { return false };
        for dx in 0..16_i64 { for dz in 0..16_i64 { if self.contains_i64(i64::from(min_x) + dx, i64::from(min_z) + dz) { return true; } } }
        false
    }
    pub fn contains_region(&self, x: i32, z: i32) -> bool {
        let Ok(min_x) = region_to_chunk(x) else { return false }; let Ok(min_z) = region_to_chunk(z) else { return false };
        for dx in 0..32_i64 { for dz in 0..32_i64 { let Some(cx) = min_x.checked_add(dx as i32) else { continue }; let Some(cz) = min_z.checked_add(dz as i32) else { continue }; if self.contains_chunk(cx, cz) { return true; } } }
        false
    }
    fn count(&self, region_x: i32, region_z: i32) -> Result<u16, VisibilityError> {
        let (sx, _) = region_chunk_span(region_x)?; let (sz, _) = region_chunk_span(region_z)?;
        let sx = i32::try_from(sx).map_err(|_| VisibilityError::CountOverflow)?; let sz = i32::try_from(sz).map_err(|_| VisibilityError::CountOverflow)?;
        let mut count = 0u16; for x in 0..32 { for z in 0..32 { if self.contains_chunk(sx + x, sz + z) { count += 1; } } } Ok(count)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WorldBorderSnapshot { center_x: i32, center_z: i32, radius: i32 }
impl WorldBorderSnapshot {
    pub fn center_x(&self) -> i32 { self.center_x }
    pub fn center_z(&self) -> i32 { self.center_z }
    pub fn radius(&self) -> i32 { self.radius }

    pub fn new(center_x: i32, center_z: i32, radius: i32) -> Result<Self, VisibilityError> {
        if radius < 0 || i64::from(center_x) - i64::from(radius) < i64::from(i32::MIN) || i64::from(center_x) + i64::from(radius) > i64::from(i32::MAX)
            || i64::from(center_z) - i64::from(radius) < i64::from(i32::MIN) || i64::from(center_z) + i64::from(radius) > i64::from(i32::MAX) { return Err(VisibilityError::QueryOverflow); }
        Ok(Self { center_x, center_z, radius })
    }
    pub fn from_runtime(center_x: f64, center_z: f64, size: f64) -> Result<Self, VisibilityError> {
        if !center_x.is_finite() || !center_z.is_finite() || !size.is_finite() || size < 0.0 { return Err(VisibilityError::QueryOverflow); }
        let cx = center_x.trunc(); let cz = center_z.trunc(); let radius = (size / 2.0).ceil();
        if cx < f64::from(i32::MIN) || cx > f64::from(i32::MAX) || cz < f64::from(i32::MIN) || cz > f64::from(i32::MAX) || radius > f64::from(i32::MAX) { return Err(VisibilityError::QueryOverflow); }
        Self::new(cx as i32, cz as i32, radius as i32)
    }
    pub fn contains_block(&self, x: i32, z: i32) -> bool { x >= self.center_x - self.radius && x < self.center_x + self.radius && z >= self.center_z - self.radius && z < self.center_z + self.radius }
    pub fn contains_chunk(&self, x: i32, z: i32) -> bool { x >= block_to_chunk(self.center_x - self.radius) && x <= block_to_chunk(self.center_x + self.radius) && z >= block_to_chunk(self.center_z - self.radius) && z <= block_to_chunk(self.center_z + self.radius) }
    pub fn contains_region(&self, x: i32, z: i32) -> bool { x >= block_to_region(self.center_x - self.radius) && x <= block_to_region(self.center_x + self.radius) && z >= block_to_region(self.center_z - self.radius) && z <= block_to_region(self.center_z + self.radius) }
    fn count(&self, region_x: i32, region_z: i32) -> Result<u16, VisibilityError> {
        let (min_x, max_x) = region_chunk_span(region_x)?; let (min_z, max_z) = region_chunk_span(region_z)?;
        let min_cx = i64::from(block_to_chunk(self.center_x - self.radius)); let max_cx = i64::from(block_to_chunk(self.center_x + self.radius));
        let min_cz = i64::from(block_to_chunk(self.center_z - self.radius)); let max_cz = i64::from(block_to_chunk(self.center_z + self.radius));
        let width = (max_x.min(max_cx) - min_x.max(min_cx) + 1).max(0); let height = (max_z.min(max_cz) - min_z.max(min_cz) + 1).max(0);
        u16::try_from(width * height).map_err(|_| VisibilityError::CountOverflow)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VisibilityShape { Rectangle(Rectangle), Circle(Circle), Polygon(Polygon), WorldBorder(WorldBorderSnapshot) }

impl VisibilityShape {
    fn block(&self, x: i32, z: i32) -> bool { match self { Self::Rectangle(v)=>v.contains_block(x,z), Self::Circle(v)=>v.contains_block(x,z), Self::Polygon(v)=>v.contains_block(x,z), Self::WorldBorder(v)=>v.contains_block(x,z) } }
    fn chunk(&self, x: i32, z: i32) -> bool { match self { Self::Rectangle(v)=>v.contains_chunk(x,z), Self::Circle(v)=>v.contains_chunk(x,z), Self::Polygon(v)=>v.contains_chunk(x,z), Self::WorldBorder(v)=>v.contains_chunk(x,z) } }
    fn region(&self, x: i32, z: i32) -> bool { match self { Self::Rectangle(v)=>v.contains_region(x,z), Self::Circle(v)=>v.contains_region(x,z), Self::Polygon(v)=>v.contains_region(x,z), Self::WorldBorder(v)=>v.contains_region(x,z) } }
    fn count(&self, x: i32, z: i32) -> Result<u16, VisibilityError> { match self { Self::Rectangle(v)=>v.count(x,z), Self::Circle(v)=>v.count(x,z), Self::Polygon(v)=>v.count(x,z), Self::WorldBorder(v)=>v.count(x,z) } }
}

#[derive(Debug)]
pub struct VisibilityLimit { shapes: Box<[VisibilityShape]> }
impl VisibilityLimit {
    pub fn new(shapes: Vec<VisibilityShape>) -> Result<Self, VisibilityError> {
        Ok(Self { shapes: shapes.into_boxed_slice() })
    }

    pub fn from_wire(values: &[WireLimit], runtime_border: Option<WorldBorderSnapshot>) -> Result<Self, VisibilityError> {
        let mut shapes = Vec::with_capacity(values.len());
        for value in values {
            match VisibilityLimitKind::try_from(value.kind).map_err(|_| VisibilityError::UnsupportedWireKind)? {
                VisibilityLimitKind::WorldBorder => {
                    if !value.points.is_empty() || value.center_x != 0 || value.center_z != 0 || value.radius != 0.0 {
                        return Err(VisibilityError::InvalidWorldBorder);
                    }
                    shapes.push(VisibilityShape::WorldBorder(runtime_border.ok_or(VisibilityError::WorldBorderRuntimeRequired)?));
                }
                VisibilityLimitKind::Circle => {
                    if !value.points.is_empty() || !value.radius.is_finite() || value.radius.fract() != 0.0
                        || value.radius < 1.0 || value.radius > 46_340.0 {
                        return Err(VisibilityError::InvalidCircle);
                    }
                    shapes.push(VisibilityShape::Circle(Circle::new(value.center_x, value.center_z, value.radius as i32)?));
                }
                VisibilityLimitKind::Rectangle => {
                    if value.points.len() != 2 { return Err(VisibilityError::InvalidRectangle); }
                    let a = &value.points[0];
                    let b = &value.points[1];
                    shapes.push(VisibilityShape::Rectangle(Rectangle::new(a.x, a.z, b.x, b.z)?));
                }
                VisibilityLimitKind::Polygon => {
                    if value.points.len() < 3 { return Err(VisibilityError::InvalidPolygon); }
                    shapes.push(VisibilityShape::Polygon(Polygon::new(value.points.iter().map(|p| (p.x, p.z)).collect())?));
                }
                VisibilityLimitKind::Unspecified => return Err(VisibilityError::UnsupportedWireKind),
            }
        }
        Ok(Self { shapes: shapes.into_boxed_slice() })
    }

    pub fn shapes(&self) -> &[VisibilityShape] { &self.shapes }
    pub fn contains_block(&self, x: i32, z: i32) -> bool {
        self.shapes.is_empty() || self.shapes.iter().any(|s| s.block(x, z))
    }
    pub fn contains_chunk(&self, x: i32, z: i32) -> bool {
        self.shapes.is_empty() || self.shapes.iter().any(|s| s.chunk(x, z))
    }
    pub fn contains_region(&self, x: i32, z: i32) -> bool {
        self.shapes.is_empty() || self.shapes.iter().any(|s| s.region(x, z))
    }
    pub fn count_chunks_in_region(&self, x: i32, z: i32) -> Result<u16, VisibilityError> {
        let _ = region_chunk_span(x)?;
        let _ = region_chunk_span(z)?;
        if self.shapes.is_empty() { return Ok(1024); }
        if self.shapes.len() == 1 { return self.shapes[0].count(x, z); }
        let (sx, _) = region_chunk_span(x)?;
        let (sz, _) = region_chunk_span(z)?;
        let sx = i32::try_from(sx).map_err(|_| VisibilityError::CountOverflow)?;
        let sz = i32::try_from(sz).map_err(|_| VisibilityError::CountOverflow)?;
        let mut count = 0u16;
        for dx in 0..32 {
            for dz in 0..32 {
                if self.contains_chunk(sx + dx, sz + dz) { count += 1; }
            }
        }
        Ok(count)
    }
}

fn region_chunk_span(region: i32) -> Result<(i64, i64), VisibilityError> {
    let start = i64::from(region) * 32; let end = start + 31;
    if start < i64::from(i32::MIN) || end > i64::from(i32::MAX) { return Err(VisibilityError::CountOverflow); }
    Ok((start, end))
}
