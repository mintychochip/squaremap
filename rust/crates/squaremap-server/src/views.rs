use crate::output::OutputRoot;
use squaremap_protocol::wire::{
    Envelope, Marker, MarkerLayer, Point, World, WorldIdentity, WorldStateReplace, envelope, marker,
};
use squaremap_state::view::{
    IconView, IconsView, MarkerGeometryView, MarkerLayerView, MarkerStyleView, MarkerTooltipView,
    MarkerView, NameplatesView, PlayerTrackerView, PlayerView, PlayersView, PolylinePoints,
    SettingsView, SpawnView, UiCoordinatesView, UiLinkView, UiSidebarView, UiView, ViewPoint,
    WorldSettingsView, WorldSummaryView, ZoomView, serialize_json,
};
use std::collections::BTreeMap;
use std::io;

pub fn write_players(root: &OutputRoot, view: &PlayersView) -> io::Result<Vec<u8>> {
    write_json(root, "tiles/players.json", view)
}
pub fn write_settings(root: &OutputRoot, view: &SettingsView) -> io::Result<Vec<u8>> {
    write_json(root, "tiles/settings.json", view)
}
pub fn write_world_settings(
    root: &OutputRoot,
    world: &str,
    view: &WorldSettingsView,
) -> io::Result<Vec<u8>> {
    write_json(root, &format!("tiles/{world}/settings.json"), view)
}
pub fn apply_replacement(root: &OutputRoot, envelope: &Envelope) -> io::Result<Option<Vec<u8>>> {
    match envelope.payload.as_ref() {
        Some(envelope::Payload::PlayersReplace(value)) => {
            let mut state = root.canonical_state()?;
            reject_stale(value.revision, state.players_revision, "players")?;
            for player in &value.players {
                validate_player_epoch(&state, player)?;
            }
            let mut players = value.players.iter().map(player).collect::<Vec<_>>();
            players.sort_by(|left, right| left.uuid.cmp(&right.uuid));
            let bytes = write_players(
                root,
                &PlayersView {
                    players,
                    max: value.max_players,
                },
            )?;
            state.players_revision = value.revision;
            state.players = bytes.clone();
            root.replace_canonical(state)?;
            Ok(Some(bytes))
        }
        Some(envelope::Payload::MarkerLayersReplace(value)) => {
            let world = value
                .world
                .as_ref()
                .map(legacy_world_identity_name)
                .unwrap_or_else(|| "world".into());
            let mut state = root.canonical_state()?;
            reject_stale(
                value.revision,
                state.marker_revisions.get(&world).copied().unwrap_or(0),
                "markers",
            )?;
            if let Some(identity) = value.world.as_ref() {
                validate_world_epoch(&state, identity)?;
            } else if !state.world_epochs.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "marker replacement has no world identity",
                ));
            }
            let mut ordered_layers = value.layers.iter().collect::<Vec<_>>();
            ordered_layers.sort_by(|left, right| left.id.cmp(&right.id));
            let layers: Vec<MarkerLayerView> = ordered_layers
                .into_iter()
                .map(marker_layer)
                .collect::<io::Result<Vec<_>>>()?;
            let bytes = write_json(root, &format!("tiles/{world}/markers.json"), &layers)?;
            state.marker_revisions.insert(world.clone(), value.revision);
            state
                .marker_outputs
                .insert(format!("tiles/{world}/markers.json"), bytes.clone());
            root.replace_canonical(state)?;
            Ok(Some(bytes))
        }
        Some(envelope::Payload::IconsReplace(value)) => {
            let mut state = root.canonical_state()?;
            reject_stale(value.revision, state.icons_revision, "icons")?;
            let mut ordered_icons = value.icons.iter().collect::<Vec<_>>();
            ordered_icons.sort_by(|left, right| left.id.cmp(&right.id));
            let existing_icons = root.existing_files("images/icon/registered")?;
            let encoded = ordered_icons
                .iter()
                .map(|icon| {
                    let path = format!("images/icon/registered/{}.png", icon.id);
                    let bytes = encode_rgba_png(icon.width, icon.height, &icon.image)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    let view = IconView {
                        id: icon.id.clone(),
                        mime_type: icon.mime_type.clone(),
                        width: icon.width,
                        height: icon.height,
                    };
                    Ok((path, bytes, view))
                })
                .collect::<io::Result<Vec<_>>>()?;
            for (path, bytes, _) in &encoded {
                root.atomic_write(path, bytes)?;
            }
            let icons = encoded
                .iter()
                .map(|(_, _, view)| view.clone())
                .collect::<Vec<_>>();
            let bytes = serialize_json(&IconsView { icons })
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let current: BTreeMap<String, Vec<u8>> = encoded
                .iter()
                .map(|(path, bytes, _)| (path.clone(), bytes.clone()))
                .collect();
            for path in existing_icons.iter().filter(|path| {
                path.extension().is_some_and(|extension| extension == "png")
                    && !current.contains_key(path.to_str().unwrap_or_default())
            }) {
                root.remove(path)?;
            }
            for path in state
                .icon_assets
                .keys()
                .filter(|path| !current.contains_key(*path))
                .cloned()
                .collect::<Vec<_>>()
            {
                root.remove(path)?;
            }
            state.icon_assets = current;
            state.icons_revision = value.revision;
            state.icons = bytes.clone();
            root.replace_canonical(state)?;
            Ok(Some(bytes))
        }
        Some(envelope::Payload::WorldStateReplace(value)) => {
            let mut state = root.canonical_state()?;
            reject_stale(value.revision, state.worlds_revision, "worlds")?;
            let worlds = sorted_worlds(value);
            for world in &worlds {
                if let Some(identity) = world.identity.as_ref() {
                    let key = format!("{}/{}", identity.namespace, identity.value);
                    if identity.epoch < state.world_epochs.get(&key).copied().unwrap_or(0) {
                        return Err(stale_error("world epoch"));
                    }
                }
            }
            let reloaded_worlds = worlds
                .iter()
                .filter_map(|world| {
                    let identity = world.identity.as_ref()?;
                    let key = format!("{}/{}", identity.namespace, identity.value);
                    (state
                        .world_epochs
                        .get(&key)
                        .is_some_and(|previous| identity.epoch > *previous))
                    .then(|| world_name(world))
                })
                .collect::<Vec<_>>();
            let settings = settings(value, &worlds);
            let existing_tiles = root.existing_files("tiles")?;
            let bytes = write_settings(root, &settings)?;
            let mut outputs = BTreeMap::new();
            for world in &worlds {
                let name = world_name(world);
                let path = format!("tiles/{name}/settings.json");
                let bytes = write_world_settings(root, &name, &world_settings(world))?;
                outputs.insert(path, bytes);
            }
            let current_worlds: std::collections::HashSet<String> =
                worlds.iter().map(|world| world_name(world)).collect();
            for path in existing_tiles.iter().filter(|path| {
                path.to_str()
                    .and_then(|value| value.strip_prefix("tiles/"))
                    .and_then(|value| value.split_once('/'))
                    .is_some_and(|(name, file)| {
                        (file == "markers.json"
                            && reloaded_worlds.iter().any(|reloaded| reloaded == name))
                            || ((file == "settings.json" || file == "markers.json")
                                && !current_worlds.contains(name))
                    })
            }) {
                root.remove(path)?;
            }
            for path in state
                .marker_outputs
                .keys()
                .filter(|path| {
                    path.strip_prefix("tiles/")
                        .and_then(|value| value.strip_suffix("/markers.json"))
                        .is_some_and(|name| !current_worlds.contains(name))
                })
                .cloned()
                .collect::<Vec<_>>()
            {
                root.remove(&path)?;
                state.marker_outputs.remove(&path);
            }
            for name in reloaded_worlds {
                let path = format!("tiles/{name}/markers.json");
                root.remove(&path)?;
                state.marker_outputs.remove(&path);
                state.marker_revisions.remove(&name);
            }
            for path in state
                .world_outputs
                .keys()
                .filter(|path| !outputs.contains_key(*path))
                .cloned()
                .collect::<Vec<_>>()
            {
                root.remove(path)?;
            }
            state.world_outputs = outputs;
            state.world_epochs = worlds
                .iter()
                .filter_map(|world| {
                    world
                        .identity
                        .as_ref()
                        .map(|id| (format!("{}/{}", id.namespace, id.value), id.epoch))
                })
                .collect();
            state.worlds_revision = value.revision;
            state.worlds = bytes.clone();
            root.replace_canonical(state)?;
            Ok(Some(bytes))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported bridge payload",
        )),
    }
}

fn validate_world_epoch(
    state: &squaremap_state::CanonicalState,
    identity: &WorldIdentity,
) -> io::Result<()> {
    if state.world_epochs.is_empty() {
        return Ok(());
    }
    let key = format!("{}/{}", identity.namespace, identity.value);
    match state.world_epochs.get(&key) {
        Some(expected) if *expected == identity.epoch => Ok(()),
        Some(expected) if identity.epoch < *expected => Err(stale_error("world epoch")),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "world epoch does not match canonical world",
        )),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "world is not present in canonical state",
        )),
    }
}

fn validate_player_epoch(
    state: &squaremap_state::CanonicalState,
    player: &squaremap_protocol::wire::Player,
) -> io::Result<()> {
    if state.world_epochs.is_empty() {
        return Ok(());
    }
    let identity = player.world.as_ref().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "player has no world identity")
    })?;
    validate_world_epoch(state, identity)
}

fn reject_stale(revision: u64, previous: u64, domain: &str) -> io::Result<()> {
    if revision != 0 && revision <= previous {
        return Err(stale_error(domain));
    }
    Ok(())
}

fn stale_error(domain: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("stale {domain} replacement"),
    )
}

fn player(value: &squaremap_protocol::wire::Player) -> PlayerView {
    PlayerView {
        name: value.name.clone(),
        display_name: value.display_name.clone(),
        uuid: hex::encode(&value.uuid),
        world: value
            .world
            .as_ref()
            .map(legacy_world_identity_name)
            .unwrap_or_default(),
        x: value.x,
        y: value.y,
        z: value.z,
        yaw: value.yaw,
        armor: value.armor,
        health: value.health,
    }
}

fn sorted_worlds(value: &WorldStateReplace) -> Vec<&World> {
    let mut worlds = value.worlds.iter().collect::<Vec<_>>();
    worlds.sort_by(|left, right| {
        let left_identity = left.identity.as_ref();
        let right_identity = right.identity.as_ref();
        left_identity
            .map(|id| id.namespace.as_str())
            .unwrap_or("")
            .cmp(right_identity.map(|id| id.namespace.as_str()).unwrap_or(""))
            .then_with(|| {
                left_identity
                    .map(|id| id.value.as_str())
                    .unwrap_or("")
                    .cmp(right_identity.map(|id| id.value.as_str()).unwrap_or(""))
            })
    });
    worlds
}

fn settings(value: &WorldStateReplace, worlds: &[&World]) -> SettingsView {
    SettingsView {
        worlds: worlds
            .iter()
            .map(|world| WorldSummaryView {
                name: world_name(world),
                display_name: world.display_name.clone(),
                icon: world.icon.clone(),
                environment: world.environment.clone(),
                order: world.order,
            })
            .collect(),
        ui: value.ui.as_ref().map_or_else(
            || UiView {
                title: String::new(),
                coordinates: UiCoordinatesView {
                    enabled: false,
                    html: String::new(),
                },
                link: UiLinkView { enabled: false },
                sidebar: UiSidebarView {
                    pinned: String::new(),
                    player_list_label: String::new(),
                    world_list_label: String::new(),
                },
            },
            |ui| UiView {
                title: ui.title.clone(),
                coordinates: UiCoordinatesView {
                    enabled: ui.coordinates_enabled,
                    html: ui.coordinates_html.clone(),
                },
                link: UiLinkView {
                    enabled: ui.link_enabled,
                },
                sidebar: UiSidebarView {
                    pinned: ui.sidebar_pinned.clone(),
                    player_list_label: ui.sidebar_player_list_label.clone(),
                    world_list_label: ui.sidebar_world_list_label.clone(),
                },
            },
        ),
    }
}

fn world_settings(world: &World) -> WorldSettingsView {
    let tracker = world.player_tracker.as_ref();
    let zoom = world.zoom.as_ref();
    let nameplates = NameplatesView {
        enabled: tracker.map(|v| v.nameplate_enabled).unwrap_or(false),
        show_heads: tracker.map(|v| v.nameplate_show_heads).unwrap_or(false),
        heads_url: tracker
            .map(|v| v.nameplate_heads_url.clone())
            .unwrap_or_default(),
        show_armor: tracker.map(|v| v.nameplate_show_armor).unwrap_or(false),
        show_health: tracker.map(|v| v.nameplate_show_health).unwrap_or(false),
    };
    WorldSettingsView {
        spawn: world
            .spawn
            .as_ref()
            .map(|v| SpawnView { x: v.x, z: v.z })
            .unwrap_or(SpawnView { x: 0, z: 0 }),
        player_tracker: PlayerTrackerView {
            enabled: tracker.map(|v| v.enabled).unwrap_or(false),
            update_interval: tracker.map(|v| v.update_interval).unwrap_or(0),
            label: tracker.map(|v| v.label.clone()).unwrap_or_default(),
            show_controls: tracker.map(|v| v.show_controls).unwrap_or(false),
            default_hidden: tracker.map(|v| v.default_hidden).unwrap_or(false),
            priority: tracker.map(|v| v.priority).unwrap_or(0),
            z_index: tracker.map(|v| v.z_index).unwrap_or(0),
            nameplates,
        },
        zoom: ZoomView {
            max: zoom.map(|v| v.max).unwrap_or(0),
            r#def: zoom.map(|v| v.r#def).unwrap_or(0),
            extra: zoom.map(|v| v.extra).unwrap_or(0),
        },
        marker_update_interval: world.marker_update_interval,
        tiles_update_interval: world.tiles_update_interval,
    }
}

fn world_name(world: &World) -> String {
    world
        .identity
        .as_ref()
        .map(legacy_world_identity_name)
        .unwrap_or_default()
}

fn legacy_world_identity_name(world: &WorldIdentity) -> String {
    format!("{}_{}", world.namespace, world.value)
}
fn marker_layer(layer: &MarkerLayer) -> io::Result<MarkerLayerView> {
    Ok(MarkerLayerView {
        id: layer.id.clone(),
        name: layer.label.clone(),
        control: layer.show_controls,
        hide: layer.default_hidden,
        order: layer.layer_priority,
        z_index: layer.z_index,
        timestamp: layer.timestamp,
        markers: layer
            .markers
            .iter()
            .map(marker_view)
            .collect::<io::Result<Vec<_>>>()?,
    })
}

fn marker_view(value: &Marker) -> io::Result<MarkerView> {
    let style = value.style.as_ref();
    let geometry = value
        .geometry
        .as_ref()
        .map(geometry)
        .transpose()?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "marker geometry not set"))?;
    Ok(MarkerView {
        style: MarkerStyleView {
            stroke: style.map(|v| v.stroke).unwrap_or(true),
            color: style
                .map(|v| v.stroke_color.clone())
                .unwrap_or_else(|| "#0000ff".into()),
            weight: style.map(|v| v.stroke_weight).unwrap_or(3),
            opacity: style.map(|v| v.stroke_opacity).unwrap_or(1.0),
            fill: style.map(|v| v.fill).unwrap_or(true),
            fill_color: style.and_then(|v| v.fill_color.clone()),
            fill_opacity: style.map(|v| v.fill_opacity).unwrap_or(0.2),
            fill_rule: style
                .map(|v| v.fill_rule.clone())
                .unwrap_or_else(|| "evenodd".into()),
        },
        tooltip: value
            .tooltip
            .as_ref()
            .map(|tooltip| MarkerTooltipView {
                click: tooltip.click.clone(),
                hover: tooltip.hover.clone(),
            })
            .and_then(|tooltip| {
                if tooltip.click.is_some() || tooltip.hover.is_some() {
                    Some(tooltip)
                } else {
                    None
                }
            }),
        geometry,
    })
}
fn geometry(value: &marker::Geometry) -> io::Result<MarkerGeometryView> {
    Ok(match value {
        marker::Geometry::Icon(value) => MarkerGeometryView::Icon {
            point: point(value.point.as_ref()),
            size: ViewPoint {
                x: value.size_x,
                z: value.size_z,
            },
            anchor: point(value.anchor.as_ref()),
            tooltip_anchor: point(value.tooltip_anchor.as_ref()),
            icon: value.image.clone(),
        },
        marker::Geometry::Circle(value) => MarkerGeometryView::Circle {
            center: point(value.center.as_ref()),
            radius: value.radius,
        },
        marker::Geometry::Ellipse(value) => MarkerGeometryView::Ellipse {
            center: point(value.center.as_ref()),
            radius_x: value.radius_x,
            radius_z: value.radius_z,
        },
        marker::Geometry::Rectangle(value) => MarkerGeometryView::Rectangle {
            points: vec![point(value.point1.as_ref()), point(value.point2.as_ref())],
        },
        marker::Geometry::Polyline(value) => {
            let lines = value
                .lines
                .iter()
                .map(|line| line.points.iter().map(|p| point(Some(p))).collect())
                .collect::<Vec<Vec<ViewPoint>>>();
            let points = if lines.len() == 1 {
                PolylinePoints::Flat(lines.into_iter().next().unwrap_or_default())
            } else {
                PolylinePoints::Nested(lines)
            };
            MarkerGeometryView::Polyline { points }
        }
        marker::Geometry::Polygon(value) => MarkerGeometryView::Polygon {
            points: polygon_points(value),
        },
        marker::Geometry::MultiPolygon(value) => MarkerGeometryView::MultiPolygon {
            points: value.polygons.iter().map(polygon_points).collect(),
        },
    })
}

fn polygon_points(value: &squaremap_protocol::wire::MarkerPolygon) -> Vec<Vec<ViewPoint>> {
    let mut points = Vec::with_capacity(1 + value.negative_space.len());
    points.push(value.main_polygon.iter().map(|p| point(Some(p))).collect());
    points.extend(
        value
            .negative_space
            .iter()
            .map(|line| line.points.iter().map(|p| point(Some(p))).collect()),
    );
    points
}
fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, &'static str> {
    if width == 0 || height == 0 {
        return Err("icon dimensions must be non-zero");
    }
    let row = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or("icon dimensions overflow")?;
    let expected = row
        .checked_mul(usize::try_from(height).map_err(|_| "icon dimensions overflow")?)
        .ok_or("icon dimensions overflow")?;
    if rgba.len() != expected {
        return Err("icon RGBA byte length does not match dimensions");
    }
    let mut raw = Vec::with_capacity(expected + usize::try_from(height).unwrap_or(0));
    for line in rgba.chunks_exact(row) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut zlib = vec![0x78, 0x01];
    for (index, chunk) in raw.chunks(usize::from(u16::MAX)).enumerate() {
        let final_block = index + 1 == raw.chunks(usize::from(u16::MAX)).len();
        zlib.push(if final_block { 1 } else { 0 });
        let length = u16::try_from(chunk.len()).map_err(|_| "PNG block too large")?;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", &zlib);
    png_chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(kind.len() + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb88320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn point(value: Option<&Point>) -> ViewPoint {
    value
        .map(|value| ViewPoint {
            x: value.x,
            z: value.z,
        })
        .unwrap_or(ViewPoint { x: 0, z: 0 })
}

fn write_json<T: serde::Serialize>(root: &OutputRoot, path: &str, view: &T) -> io::Result<Vec<u8>> {
    let bytes =
        serialize_json(view).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    root.atomic_write(path, &bytes)?;
    Ok(bytes)
}
