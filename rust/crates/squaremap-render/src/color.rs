//! Exact ARGB color operations used by map rendering.

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ColorError { InvalidShade, NonFiniteFloat, OutOfRangeFactor, EmptyBlend, AccumulationOverflow, InvalidDepth }

pub fn remove_alpha(color: u32) -> u32 { 0xFF00_0000 | (color & 0x00FF_FFFF) }

pub fn shade_level(color: u32, level: u8) -> Result<u32, ColorError> {
    let factor = match level { 0 => 180.0_f32 / 255.0_f32, 1 => 220.0_f32 / 255.0_f32, 2 => 1.0, _ => return Err(ColorError::InvalidShade) };
    shade_factor(color, factor)
}

pub fn shade_factor(color: u32, factor: f32) -> Result<u32, ColorError> {
    if !factor.is_finite() { return Err(ColorError::NonFiniteFloat); }
    if !(0.0..=1.0).contains(&factor) { return Err(ColorError::OutOfRangeFactor); }
    let r = (((color >> 16) & 0xff) as f32 * factor) as u32;
    let g = (((color >> 8) & 0xff) as f32 * factor) as u32;
    let b = ((color & 0xff) as f32 * factor) as u32;
    Ok(0xFF00_0000 | r << 16 | g << 8 | b)
}

pub fn abgr_to_argb(color: u32) -> u32 {
    let a = (color >> 24) & 0xff; let r = color & 0xff; let g = (color >> 8) & 0xff; let b = (color >> 16) & 0xff;
    a << 24 | r << 16 | g << 8 | b
}
pub fn argb_to_rgba(color: u32) -> u32 {
    let a = (color >> 24) & 0xff; let r = (color >> 16) & 0xff; let g = (color >> 8) & 0xff; let b = color & 0xff;
    r << 24 | g << 16 | b << 8 | a
}
pub fn rgba_to_argb(color: u32) -> u32 {
    let r = (color >> 24) & 0xff; let g = (color >> 16) & 0xff; let b = (color >> 8) & 0xff; let a = color & 0xff;
    a << 24 | r << 16 | g << 8 | b
}

pub fn mix(c1: u32, c2: u32, ratio: f32) -> Result<u32, ColorError> {
    if !ratio.is_finite() { return Err(ColorError::NonFiniteFloat); }
    if ratio >= 1.0 { return Ok(c2); }
    if ratio <= 0.0 { return Ok(c1); }
    let inv = 1.0_f32 - ratio;
    let r = ((((c1 >> 16) & 0xff) as f32 * inv) + (((c2 >> 16) & 0xff) as f32 * ratio)) as u32;
    let g = ((((c1 >> 8) & 0xff) as f32 * inv) + (((c2 >> 8) & 0xff) as f32 * ratio)) as u32;
    let b = (((c1 & 0xff) as f32 * inv) + ((c2 & 0xff) as f32 * ratio)) as u32;
    Ok(0xFF00_0000 | r << 16 | g << 8 | b)
}

pub fn checkerboard_parity(img_x: i32, img_z: i32) -> bool { (img_x.wrapping_add(img_z) & 1) != 0 }

pub fn terrain(current_y: i32, previous_y: i32, color: u32, odd: bool) -> u32 {
    let diff = (f64::from(current_y) - f64::from(previous_y)) + (if odd { 1.0_f64 } else { 0.0 } - 0.5) * 0.4;
    let level = if diff > 0.6 { 2 } else if diff < -0.6 { 0 } else { 1 };
    shade_level(color, level).expect("terrain selects a valid shade")
}

pub fn depth_checkerboard(depth: u8, color: u32, odd: bool) -> u32 {
    let diff = f64::from(depth) * 0.1_f64 + if odd { 0.2_f64 } else { 0.0 };
    let level = if diff < 0.5 { 2 } else if diff > 0.9 { 0 } else { 1 };
    shade_level(color, level).expect("depth selects a valid shade")
}

pub fn glass_composite(under: u32, glass: u32, alpha: f32) -> Result<u32, ColorError> { mix(under, glass, alpha) }

pub fn average_argb(colors: &[u32]) -> Result<u32, ColorError> {
    if colors.is_empty() { return Err(ColorError::EmptyBlend); }
    let mut sums = [0_u64; 4];
    for &color in colors {
        let channels = [(color >> 24) & 0xff, (color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff];
        for (sum, channel) in sums.iter_mut().zip(channels) {
            *sum = sum.checked_add(u64::from(channel)).ok_or(ColorError::AccumulationOverflow)?;
            if *sum > i32::MAX as u64 { return Err(ColorError::AccumulationOverflow); }
        }
    }
    let n = colors.len() as u64;
    Ok(((sums[0] / n) as u32) << 24 | ((sums[1] / n) as u32) << 16 | ((sums[2] / n) as u32) << 8 | (sums[3] / n) as u32)
}

pub fn water_biome_blend(base: u32, samples: &[u32]) -> Result<u32, ColorError> { mix(base, average_argb(samples)?, 0.8_f32) }

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FluidKind { Water, Lava }
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct FluidFlags { pub water_checkerboard: bool, pub water_clear: bool, pub lava_checkerboard: bool }

pub fn compose_fluid(depth: u8, mut color: u32, under_block: u32, kind: FluidKind, odd: bool, flags: FluidFlags) -> Result<u32, ColorError> {
    if !(1..=11).contains(&depth) { return Err(ColorError::InvalidDepth); }
    let mut shaded = false;
    match kind {
        FluidKind::Water => {
            if flags.water_checkerboard { color = depth_checkerboard(depth, color, odd); shaded = true; }
            if flags.water_clear {
                if !flags.water_checkerboard { color = shade_factor(color, 0.85_f32 - f32::from(depth) * 0.01_f32)?; }
                color = mix(color, under_block, 0.20_f32 / (f32::from(depth) / 2.0_f32))?;
                shaded = true;
            }
        }
        FluidKind::Lava => if flags.lava_checkerboard { color = depth_checkerboard(depth, color, odd); shaded = true; },
    }
    Ok(if shaded { color } else { remove_alpha(color) })
}

pub fn classify_unknown_fluid(rendered_color: u32, native_is_water: bool, native_is_lava: bool) -> FluidKind {
    if native_is_water { FluidKind::Water } else if native_is_lava { FluidKind::Lava } else if (rendered_color >> 24) & 0xff == 0xff { FluidKind::Lava } else { FluidKind::Water }
}
