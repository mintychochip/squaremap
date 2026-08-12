use crate::chunk::{
    ChunkPixels, RenderContext, RenderError, cancelled_for_region, render_chunk_seeded,
    seed_bottom_edge, select_for_region, shade_for_region, validate_for_region,
};
use crate::snapshot::Snapshot;

/// Render a Z-increasing column without scheduling. Missing current chunks
/// preserve the prior south-edge state and contribute no output pixels.
pub fn render_column<'a>(
    ctx: &RenderContext,
    above: Option<&Snapshot>,
    chunks: &[Option<&'a Snapshot>],
) -> Result<Vec<Option<ChunkPixels>>, RenderError> {
    if cancelled_for_region(ctx) {
        return Err(RenderError::Cancelled);
    }
    let start = if let Some(snapshot) = above {
        validate_for_region(ctx, snapshot)?;
        (
            snapshot.coordinate.x,
            snapshot
                .coordinate
                .z
                .checked_add(1)
                .ok_or(RenderError::CoordinateMismatch)?,
        )
    } else if let Some((index, Some(snapshot))) =
        chunks.iter().enumerate().find(|(_, chunk)| chunk.is_some())
    {
        validate_for_region(ctx, snapshot)?;
        let index = i32::try_from(index).map_err(|_| RenderError::CoordinateMismatch)?;
        (
            snapshot.coordinate.x,
            snapshot
                .coordinate
                .z
                .checked_sub(index)
                .ok_or(RenderError::CoordinateMismatch)?,
        )
    } else {
        return Ok(vec![None; chunks.len()]);
    };
    let bounds = above
        .map(|snapshot| (snapshot.min_y, snapshot.max_y, snapshot.ceiling))
        .or_else(|| {
            chunks
                .iter()
                .flatten()
                .next()
                .map(|snapshot| (snapshot.min_y, snapshot.max_y, snapshot.ceiling))
        })
        .ok_or(RenderError::CoordinateMismatch)?;
    for (index, chunk) in chunks.iter().enumerate() {
        let Some(current) = chunk else {
            continue;
        };
        validate_for_region(ctx, current)?;
        let index = i32::try_from(index).map_err(|_| RenderError::CoordinateMismatch)?;
        let expected_z = start
            .1
            .checked_add(index)
            .ok_or(RenderError::CoordinateMismatch)?;
        if current.coordinate.x != start.0 || current.coordinate.z != expected_z {
            return Err(RenderError::CoordinateMismatch);
        }
        if (current.min_y, current.max_y, current.ceiling) != bounds {
            return Err(RenderError::BoundsMismatch);
        }
    }
    let mut north = above;
    let mut carried = above
        .map(|snapshot| seed_bottom_edge(ctx, snapshot))
        .transpose()?;
    let mut output = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if cancelled_for_region(ctx) {
            return Err(RenderError::Cancelled);
        }
        let Some(current) = chunk else {
            north = None;
            output.push(None);
            continue;
        };
        let pixels = render_chunk_seeded(ctx, north, current, None, carried)?;
        carried = Some(pixels.south_edge);
        north = Some(current);
        output.push(Some(pixels));
    }
    Ok(output)
}

/// Render a south row without mutating the carried edge supplied by the caller.
pub fn render_south_row(
    ctx: &RenderContext,
    south: &Snapshot,
    carried: [i32; 16],
) -> Result<[u32; 16], RenderError> {
    render_south_row_with_edge(ctx, south, carried).map(|(row, _)| row)
}

fn render_south_row_with_edge(
    ctx: &RenderContext,
    south: &Snapshot,
    carried: [i32; 16],
) -> Result<([u32; 16], [i32; 16]), RenderError> {
    if cancelled_for_region(ctx) {
        return Err(RenderError::Cancelled);
    }
    validate_for_region(ctx, south)?;
    let mut carry = carried;
    let mut row = [0; 16];
    for x in 0..16 {
        if cancelled_for_region(ctx) {
            return Err(RenderError::Cancelled);
        }
        let top = south.surface.heightmap[x];
        if top <= south.min_y {
            continue;
        }
        let max_y = if ctx.settings.map_max_height == -1 {
            south
                .max_y
                .checked_add(1)
                .ok_or(RenderError::Malformed("vertical bounds"))?
        } else {
            ctx.settings.map_max_height
        };
        let Some((id, y)) = select_for_region(ctx, south, x as i32, 0, top.min(max_y))? else {
            continue;
        };
        let shaded = shade_for_region(ctx, south, id, y, x as i32, 0, carry[x])?;
        row[x] = shaded.color;
        if shaded.updates_last_y {
            carry[x] = shaded.effective_y;
        }
    }
    Ok((row, carry))
}

#[cfg(test)]
mod tests {
    use super::{render_column, render_south_row, render_south_row_with_edge};
    use crate::{
        Registry, RenderContext, RenderContextError, RenderError, RenderSettings, Section,
        Snapshot, SurfaceHeightmap, color, render_chunk,
    };
    use squaremap_protocol::wire::{
        BiomeDescriptor, BlockStateDescriptor, RegistryReplace, WorldIdentity,
    };
    use std::sync::Arc;

    fn underflow_boundary_snapshot() -> (Registry, Snapshot) {
        let min_y = i32::MIN;
        let max_y = min_y + 15;
        let world = WorldIdentity {
            namespace: "test".into(),
            value: "world".into(),
            epoch: 1,
        };
        let registry = Registry::with_replace(RegistryReplace {
            world: Some(world.clone()),
            revision: 1,
            block_states: vec![BlockStateDescriptor {
                id: 1,
                transparency: 1,
                fluid: 1,
                map_color: 0x00112233,
                ..Default::default()
            }],
            biomes: vec![BiomeDescriptor {
                id: 1,
                ..Default::default()
            }],
        })
        .unwrap();
        let snapshot = Snapshot {
            world: crate::snapshot::World {
                namespace: world.namespace,
                value: world.value,
                epoch: world.epoch,
            },
            coordinate: crate::snapshot::Coordinate { x: 0, z: 0 },
            min_y,
            max_y,
            ceiling: false,
            revision: 1,
            sections: vec![Section {
                section_y: min_y.div_euclid(16),
                palette: vec![1],
                blocks: vec![1; 4096],
                biome_palette: vec![1],
                biomes: vec![1; 64],
            }],
            surface: SurfaceHeightmap {
                heightmap: vec![min_y; 256],
            },
            registry_generation: registry.generation(),
        };
        (registry, snapshot)
    }

    fn fixture(glass_y: Option<i32>, under: Option<(i32, u32)>, top: i32) -> (Registry, Snapshot) {
        let world = WorldIdentity {
            namespace: "test".into(),
            value: "world".into(),
            epoch: 1,
        };
        let registry = Registry::with_replace(RegistryReplace {
            world: Some(world.clone()),
            revision: 1,
            block_states: vec![
                BlockStateDescriptor {
                    id: 1,
                    air: true,
                    transparency: 1,
                    fluid: 1,
                    map_color: 0,
                    ..Default::default()
                },
                BlockStateDescriptor {
                    id: 2,
                    transparency: 2,
                    fluid: 2,
                    map_color: 0x000000ff,
                    ..Default::default()
                },
                BlockStateDescriptor {
                    id: 3,
                    transparency: 1,
                    fluid: 1,
                    map_color: 0x00ff0000,
                    glass: true,
                    glass_alpha_percent: 25,
                    ..Default::default()
                },
                BlockStateDescriptor {
                    id: 4,
                    transparency: 1,
                    fluid: 1,
                    map_color: 0x0000ff00,
                    tint_index: 1,
                    ..Default::default()
                },
            ],
            biomes: vec![BiomeDescriptor {
                id: 1,
                ..Default::default()
            }],
        })
        .unwrap();
        let mut blocks = vec![1; 4096];
        let index = |y: i32| ((y & 15) << 8) as usize;
        if let Some(y) = glass_y {
            blocks[index(y)] = 3;
        }
        if let Some((y, id)) = under {
            blocks[index(y)] = id;
        }
        let mut heightmap = vec![0; 256];
        heightmap[0] = top;
        let snapshot = Snapshot {
            world: crate::snapshot::World {
                namespace: world.namespace,
                value: world.value,
                epoch: world.epoch,
            },
            coordinate: crate::snapshot::Coordinate { x: 0, z: 0 },
            min_y: 0,
            max_y: 15,
            ceiling: false,
            revision: 1,
            sections: vec![Section {
                section_y: 0,
                palette: vec![1, 2, 3, 4],
                blocks,
                biome_palette: vec![1],
                biomes: vec![1; 64],
            }],
            surface: SurfaceHeightmap { heightmap },
            registry_generation: registry.generation(),
        };
        (registry, snapshot)
    }

    fn context(registry: &Registry) -> RenderContext {
        RenderContext::try_new(
            registry.generation(),
            RenderSettings::default(),
            [],
            [],
            None,
            || false,
        )
        .unwrap()
    }

    #[test]
    fn south_row_empty_and_selected_air_have_distinct_carry_effects() {
        let (empty_registry, empty) = fixture(None, None, 0);
        let (selected_registry, selected) = fixture(None, None, 1);
        let (_, empty_edge) =
            render_south_row_with_edge(&context(&empty_registry), &empty, [77; 16]).unwrap();
        let (_, selected_edge) =
            render_south_row_with_edge(&context(&selected_registry), &selected, [77; 16]).unwrap();
        assert_eq!(empty_edge[0], 77);
        assert_eq!(selected_edge[0], 0);
        assert_eq!(
            render_south_row(&context(&empty_registry), &empty, [77; 16]).unwrap()[0],
            0
        );
        assert_eq!(
            render_south_row(&context(&selected_registry), &selected, [77; 16]).unwrap()[0],
            color::terrain(0, 77, 0, color::checkerboard_parity(0, 0))
        );
    }

    #[test]
    fn glass_paths_match_pixel_and_effective_y_at_center_and_south_row() {
        for (name, under, expected_edge) in [
            ("air", None, -1),
            ("opaque", Some((1, 4)), 1),
            ("fluid", Some((1, 2)), 77),
        ] {
            let (registry, snapshot) = fixture(
                Some(if under.is_none() { 0 } else { 2 }),
                under,
                if under.is_none() { 1 } else { 3 },
            );
            let ctx = context(&registry);
            let center = render_chunk(&ctx, None, &snapshot, None).unwrap();
            let (south, edge) = render_south_row_with_edge(&ctx, &snapshot, [77; 16]).unwrap();
            let y = if under.is_none() { -1 } else { 1 };
            let base = if under.is_none() {
                color::terrain(y, 0, 0, color::checkerboard_parity(0, 0))
            } else if name == "opaque" {
                color::terrain(y, 0, 0x0000ff00, color::checkerboard_parity(0, 0))
            } else {
                color::compose_fluid(
                    1,
                    0x000000ff,
                    0,
                    color::FluidKind::Water,
                    false,
                    color::FluidFlags {
                        water_clear: true,
                        ..Default::default()
                    },
                )
                .unwrap()
            };
            let expected_center = color::glass_composite(base, 0x00ff0000, 0.25).unwrap();
            let south_base = if under.is_none() {
                color::terrain(y, 77, 0, color::checkerboard_parity(0, 0))
            } else if name == "opaque" {
                color::terrain(y, 77, 0x0000ff00, color::checkerboard_parity(0, 0))
            } else {
                color::compose_fluid(
                    1,
                    0x000000ff,
                    0,
                    color::FluidKind::Water,
                    false,
                    color::FluidFlags {
                        water_clear: true,
                        ..Default::default()
                    },
                )
                .unwrap()
            };
            let expected_south = color::glass_composite(south_base, 0x00ff0000, 0.25).unwrap();
            assert_eq!(center.pixels[0], expected_center, "{name} center");
            assert_eq!(
                center.south_edge[0],
                if name == "fluid" { 0 } else { expected_edge },
                "{name} center edge"
            );
            assert_eq!(south[0], expected_south, "{name} south");
            assert_eq!(edge[0], expected_edge, "{name} south edge");
        }
    }

    #[test]
    fn traversal_selected_sentinel_air_shades_and_updates_effective_y() {
        let (registry, snapshot) = fixture(None, None, 1);
        let shaded =
            super::shade_for_region(&context(&registry), &snapshot, 0, 0, 0, 0, 77).unwrap();
        assert_eq!(
            shaded.color,
            color::terrain(0, 77, 0, color::checkerboard_parity(0, 0))
        );
        assert_eq!(shaded.effective_y, 0);
        assert!(shaded.updates_last_y);
    }

    #[test]
    fn south_row_empty_column_at_min_height_skips_selection_without_underflow() {
        let (registry, snapshot) = underflow_boundary_snapshot();
        let context = RenderContext::try_new(
            registry.generation(),
            RenderSettings::default(),
            [],
            [],
            None,
            || false,
        )
        .unwrap();
        let row = render_south_row(&context, &snapshot, [77; 16]).unwrap();
        assert_eq!(row[0], 0);
    }
    #[test]
    fn forged_snapshot_identity_is_rejected_even_with_same_generation() {
        let (registry, snapshot) = fixture(None, None, 1);
        let context1 = context(&registry);
        let mut forged_world = snapshot;
        forged_world.world.namespace = "forged".into();
        assert_eq!(
            render_chunk(&context1, None, &forged_world, None),
            Err(RenderError::IdentityMismatch)
        );

        let (registry, snapshot) = fixture(None, None, 1);
        let context2 = context(&registry);
        let mut forged_revision = snapshot;
        forged_revision.revision += 1;
        assert_eq!(
            render_chunk(&context2, None, &forged_revision, None),
            Err(RenderError::IdentityMismatch)
        );
    }

    #[test]
    fn max_height_boundary_returns_typed_error_without_overflow() {
        let (registry, mut snapshot) = fixture(None, None, 1);
        snapshot.min_y = i32::MAX - 15;
        snapshot.max_y = i32::MAX;
        assert_eq!(
            render_south_row(&context(&registry), &snapshot, [0; 16]),
            Err(RenderError::Malformed("vertical bounds")),
        );
    }

    #[test]
    fn south_neighbor_coordinate_increment_is_checked() {
        let (registry, mut center) = fixture(None, None, 1);
        let (_, mut south) = fixture(None, None, 1);
        south.registry_generation = center.registry_generation.clone();
        center.coordinate.z = i32::MAX;
        south.coordinate.z = i32::MAX;
        assert_eq!(
            render_chunk(&context(&registry), None, &center, Some(&south)),
            Err(RenderError::CoordinateMismatch),
        );
    }

    #[test]
    fn column_preflight_rejects_bounds_after_missing_gap() {
        let (registry, first) = fixture(None, None, 1);
        let (_, mut third) = fixture(None, None, 1);
        third.registry_generation = first.registry_generation.clone();
        third.coordinate.z = 2;
        third.max_y = 31;
        third.ceiling = true;
        third.sections.push(Section {
            section_y: 1,
            palette: vec![1],
            blocks: vec![1; 4096],
            biome_palette: vec![1],
            biomes: vec![1; 64],
        });
        let result = render_column(
            &context(&registry),
            None,
            &[Some(&first), None, Some(&third)],
        );
        assert_eq!(result, Err(RenderError::BoundsMismatch));
    }

    #[test]
    fn biome_world_coordinate_multiplication_returns_typed_error() {
        let (registry, mut snapshot) = fixture(None, Some((1, 4)), 3);
        snapshot.coordinate.x = i32::MAX;
        let source =
            Arc::new(crate::StaticBiomeSource::new(registry.generation()).with_fallback(1));
        let settings = RenderSettings {
            biomes_enabled: true,
            ..RenderSettings::default()
        };
        let context = RenderContext::try_new(
            registry.generation(),
            settings,
            [],
            [],
            Some(source),
            || false,
        )
        .unwrap();
        assert!(matches!(
            super::shade_for_region(&context, &snapshot, 4, 1, 0, 0, 0),
            Err(RenderError::Malformed("coordinate arithmetic")),
        ));
    }

    #[test]
    fn biome_render_context_rejects_distinct_generation_source() {
        let (registry, _) = fixture(None, None, 1);
        let other = Registry::with_replace(RegistryReplace {
            revision: 1,
            ..Default::default()
        })
        .unwrap();
        let source = Arc::new(crate::StaticBiomeSource::new(other.generation()).with_fallback(1));
        let settings = RenderSettings {
            biomes_enabled: true,
            ..RenderSettings::default()
        };
        assert!(matches!(
            RenderContext::try_new(
                registry.generation(),
                settings,
                [],
                [],
                Some(source),
                || false
            ),
            Err(RenderContextError::GenerationMismatch),
        ));
    }

    #[test]
    fn fluid_at_min_height_updates_last_y_after_terrain_shading() {
        let (registry, snapshot) = fixture(None, Some((0, 2)), 1);
        let shaded =
            super::shade_for_region(&context(&registry), &snapshot, 2, 0, 0, 0, 77).unwrap();
        assert!(shaded.updates_last_y);
        assert_eq!(shaded.effective_y, 0);
        assert_eq!(
            shaded.color,
            color::terrain(0, 77, 0x000000ff, color::checkerboard_parity(0, 0))
        );
    }

    #[test]
    fn glass_descending_to_fluid_at_min_height_updates_last_y() {
        let (registry, snapshot) = fixture(Some(1), Some((0, 2)), 2);
        let shaded =
            super::shade_for_region(&context(&registry), &snapshot, 3, 1, 0, 0, 77).unwrap();
        assert!(shaded.updates_last_y);
        assert_eq!(shaded.effective_y, 0);
    }

    #[test]
    fn fluid_depth_stops_before_reading_below_minimum() {
        let (registry, snapshot) = fixture(None, Some((0, 2)), 1);
        let shaded =
            super::shade_for_region(&context(&registry), &snapshot, 2, 1, 0, 0, 0).unwrap();
        let expected = color::compose_fluid(
            1,
            0x000000ff,
            0x000000ff,
            color::FluidKind::Water,
            false,
            color::FluidFlags {
                water_clear: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(shaded.color, expected);
    }

    fn extreme_snapshot(mut snapshot: Snapshot) -> Snapshot {
        let min_y = i32::MIN;
        snapshot.min_y = min_y;
        snapshot.max_y = min_y + 15;
        snapshot.sections[0].section_y = min_y.div_euclid(16);
        snapshot.surface.heightmap.fill(min_y);
        snapshot
    }

    #[test]
    fn iterate_up_at_min_height_returns_typed_error_without_panicking() {
        let (registry, mut snapshot) = fixture(None, None, 1);
        snapshot = extreme_snapshot(snapshot);
        snapshot.surface.heightmap[0] = i32::MIN + 1;
        let context = RenderContext::try_new(
            registry.generation(),
            RenderSettings {
                iterate_up: true,
                ..RenderSettings::default()
            },
            [],
            [],
            None,
            || false,
        )
        .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_chunk(&context, None, &snapshot, None)
        }));
        assert!(matches!(
            result,
            Ok(Err(RenderError::Malformed("vertical bounds")))
        ));
    }

    #[test]
    fn glass_descent_at_min_height_returns_typed_error_without_panicking() {
        let (registry, snapshot) = fixture(Some(0), None, 1);
        let snapshot = extreme_snapshot(snapshot);
        let context = context(&registry);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::shade_for_region(&context, &snapshot, 3, i32::MIN, 0, 0, 0)
        }));
        assert!(matches!(
            result,
            Ok(Err(RenderError::Malformed("vertical bounds")))
        ));
    }

    #[test]
    fn fluid_depth_scan_at_min_height_returns_typed_error_without_panicking() {
        let (registry, snapshot) = fixture(None, None, 1);
        let mut snapshot = extreme_snapshot(snapshot);
        let index = ((i32::MIN & 15) << 8) as usize;
        snapshot.sections[0].blocks[index] = 2;
        let context = context(&registry);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::shade_for_region(&context, &snapshot, 2, i32::MIN + 1, 0, 0, 0)
        }));
        assert!(matches!(
            result,
            Ok(Err(RenderError::Malformed("vertical bounds")))
        ));
    }
    #[test]
    fn ceiling_positive_air_matches_non_ceiling_and_does_not_treat_clear_non_air_as_air() {
        let (registry, mut ceiling) = fixture(None, Some((4, 4)), 6);
        ceiling.ceiling = true;
        let (_, mut non_ceiling) = fixture(None, Some((4, 4)), 6);
        non_ceiling.registry_generation = registry.generation();
        let context = context(&registry);
        let ceiling_pixels = render_chunk(&context, None, &ceiling, None).unwrap();
        let non_ceiling_pixels = render_chunk(&context, None, &non_ceiling, None).unwrap();
        assert_eq!(ceiling_pixels.pixel(0, 0), non_ceiling_pixels.pixel(0, 0));
    }
}
