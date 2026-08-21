//! Bit-exact ports of the vanilla algorithms behind Java's
//! `BiomeColors#grassColorSampler`:
//! `BiomeSpecialEffects.GrassColorModifier#modifyColor` and the
//! `Biome.BIOME_INFO_NOISE` simplex noise it samples for the swamp modifier.
//!
//! Sources (Minecraft 26.2 decompiled sources): `BiomeSpecialEffects`,
//! `Biome`, `PerlinSimplexNoise`, `SimplexNoise`, `LegacyRandomSource`,
//! `WorldgenRandom`, `BitRandomSource`, `Mth`, `ARGB`. The swamp oracle is
//! pinned end-to-end by the Java-generated fixture corpus.

use squaremap_protocol::wire::GrassColorModifier;
use std::sync::LazyLock;

/// `Biome.BIOME_INFO_NOISE`: `PerlinSimplexNoise(WorldgenRandom(LegacyRandomSource(2345)), [0])`.
/// A single-octave set keeps exactly one `SimplexNoise`; its xo/yo/zo offsets
/// are consumed from the RNG stream but never sampled (`useNoiseStart=false`),
/// and the high-frequency reseed phase requires octaves above zero.
static BIOME_INFO_NOISE: LazyLock<SimplexNoise2D> =
    LazyLock::new(|| SimplexNoise2D::new(&mut LegacyRandom::new(2345)));

/// Applies vanilla `GrassColorModifier#modifyColor(x, z, grassColor)`.
pub(crate) fn resolve_grass(modifier: GrassColorModifier, base: u32, x: i32, z: i32) -> u32 {
    match modifier {
        GrassColorModifier::None => base,
        GrassColorModifier::DarkForest => {
            // ARGB.opaque((baseColor & 16711422) + 2634762 >> 1)
            (((base & 0xFEFEFE) + 2634762) >> 1) | 0xFF000000
        }
        GrassColorModifier::Swamp => {
            let value = biome_info_noise()
                .get_value(f64::from(x) * 0.0225, f64::from(z) * 0.0225);
            // groundValue < -0.1 ? -11766212 : -9801671
            if value < -0.1 {
                0xFF4C_763C
            } else {
                0xFF6A_7039
            }
        }
    }
}

fn biome_info_noise() -> &'static SimplexNoise2D {
    &BIOME_INFO_NOISE
}

/// `LegacyRandomSource`: the java.util.Random-compatible 48-bit LCG.
struct LegacyRandom {
    seed: i64,
}

impl LegacyRandom {
    const MULTIPLIER: i64 = 25214903917;
    const INCREMENT: i64 = 11;
    const MODULUS_MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ Self::MULTIPLIER) & Self::MODULUS_MASK,
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::INCREMENT)
            & Self::MODULUS_MASK;
        (self.seed >> (48 - bits)) as i32
    }

    /// `BitRandomSource#nextInt(bound)` (java.util.Random semantics).
    fn next_bounded(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0);
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            // Java relies on int overflow here to reject out-of-range samples.
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `BitRandomSource#nextDouble()`: ((next(26) << 27) + next(27)) * 2^-53.
    fn next_f64(&mut self) -> f64 {
        let combined = (i64::from(self.next(26)) << 27) + i64::from(self.next(27));
        combined as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

/// `SimplexNoise`, 2D evaluation only (the only form BIOME_INFO_NOISE samples).
struct SimplexNoise2D {
    p: [u8; 512],
}

impl SimplexNoise2D {
    const GRADIENT: [[i32; 3]; 16] = [
        [1, 1, 0],
        [-1, 1, 0],
        [1, -1, 0],
        [-1, -1, 0],
        [1, 0, 1],
        [-1, 0, 1],
        [1, 0, -1],
        [-1, 0, -1],
        [0, 1, 1],
        [0, -1, 1],
        [0, 1, -1],
        [0, -1, -1],
        [1, 1, 0],
        [0, -1, 1],
        [-1, 1, 0],
        [0, -1, -1],
    ];
    /// `Math.sqrt(3.0)` (IEEE correctly rounded; stable literal for Rust 1.88).
    const SQRT_3: f64 = 1.732_050_807_568_877_2;
    const F2: f64 = 0.5 * (Self::SQRT_3 - 1.0);
    const G2: f64 = (3.0 - Self::SQRT_3) / 6.0;

    fn new(random: &mut LegacyRandom) -> Self {
        // xo/yo/zo are drawn first to keep the RNG stream aligned with Java;
        // their values are unused because BIOME_INFO_NOISE is single-octave
        // and sampled with useNoiseStart=false.
        let _xo = random.next_f64() * 256.0;
        let _yo = random.next_f64() * 256.0;
        let _zo = random.next_f64() * 256.0;
        let mut p = [0u8; 512];
        for (index, entry) in p.iter_mut().take(256).enumerate() {
            *entry = index as u8;
        }
        for index in 0..256usize {
            let offset = random.next_bounded(256 - index as i32) as usize + index;
            p.swap(index, offset);
        }
        for index in 256..512usize {
            p[index] = p[index - 256];
        }
        Self { p }
    }

    fn p(&self, index: usize) -> usize {
        usize::from(self.p[index & 0xFF])
    }

    /// `Mth.floor`: `(int) Math.floor(v)` (saturating cast matches Java).
    fn floor(v: f64) -> i32 {
        v.floor() as i32
    }

    fn corner_noise(&self, gradient_index: usize, x: f64, y: f64, z: f64, base: f64) -> f64 {
        let t0 = base - x * x - y * y - z * z;
        if t0 < 0.0 {
            0.0
        } else {
            let t0 = t0 * t0;
            let g = Self::GRADIENT[gradient_index];
            let dot = f64::from(g[0]) * x + f64::from(g[1]) * y + f64::from(g[2]) * z;
            t0 * t0 * dot
        }
    }

    fn get_value(&self, xin: f64, yin: f64) -> f64 {
        let s = (xin + yin) * Self::F2;
        let i = Self::floor(xin + s);
        let j = Self::floor(yin + s);
        let t = (i + j) as f64 * Self::G2;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - i1 as f64 + Self::G2;
        let y1 = y0 - j1 as f64 + Self::G2;
        let x2 = x0 - 1.0 + 2.0 * Self::G2;
        let y2 = y0 - 1.0 + 2.0 * Self::G2;
        let ii = (i & 0xFF) as usize;
        let jj = (j & 0xFF) as usize;
        let gi0 = self.p(ii + self.p(jj)) % 12;
        let gi1 = self.p(ii + i1 + self.p(jj + j1)) % 12;
        let gi2 = self.p(ii + 1 + self.p(jj + 1)) % 12;
        70.0
            * (self.corner_noise(gi0, x0, y0, 0.0, 0.5)
                + self.corner_noise(gi1, x1, y1, 0.0, 0.5)
                + self.corner_noise(gi2, x2, y2, 0.0, 0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Oracle values extracted from testdata/bridge/v2/render/manifest.json.
    // The corpus was generated by ChunkRenderFixtureDocument against the real
    // vanilla `BiomeSpecialEffects.GrassColorModifier.SWAMP#modifyColor`, so
    // these coordinates pin the entire chain: LegacyRandomSource LCG stream,
    // SimplexNoise permutation, 2D simplex evaluation, and the -0.1 threshold.
    const SWAMP_LIGHT: u32 = 0xFF6A7039; // vanilla -9801671
    const SWAMP_DARK: u32 = 0xFF4C763C; // vanilla -11766212

    #[test]
    fn swamp_modifier_matches_java_oracle_samples() {
        let light = [
            ("biome-grass-radius-0", 4, 15),
            ("biome-grass-radius-0", 11, 15),
            ("biome-grass-radius-0", 12, 15),
            ("biome-blend-radius-3", 3, 16),
            ("biome-blend-radius-3", 3, 17),
            ("biome-blend-radius-3", 4, 15),
        ];
        let dark = [
            ("biome-grass-radius-0", 13, 15),
            ("biome-grass-radius-0", 14, 15),
            ("biome-grass-radius-0", 15, 15),
            ("biome-blend-radius-3", -1, 17),
            ("biome-blend-radius-3", 0, 17),
            ("biome-blend-radius-3", 1, 17),
        ];
        for (id, x, z) in light {
            assert_eq!(
                resolve_grass(GrassColorModifier::Swamp, 0, x, z),
                SWAMP_LIGHT,
                "{id} ({x},{z})"
            );
        }
        for (id, x, z) in dark {
            assert_eq!(
                resolve_grass(GrassColorModifier::Swamp, 0, x, z),
                SWAMP_DARK,
                "{id} ({x},{z})"
            );
        }
    }

    #[test]
    fn none_modifier_is_identity() {
        assert_eq!(
            resolve_grass(GrassColorModifier::None, 0xFF6E_B28E, -3, 900_001),
            0xFF6E_B28E
        );
    }

    #[test]
    fn dark_forest_matches_vanilla_formula() {
        // ARGB.opaque((baseColor & 0xFEFEFE) + 2634762 >> 1) for 0xFF6EB28E.
        assert_eq!(
            resolve_grass(GrassColorModifier::DarkForest, 0xFF6E_B28E, 3, 4),
            0xFF4B_734C
        );
    }
}
