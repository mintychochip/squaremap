//! Proves: production `views::apply_replacement` writers served by `HttpServer`
//! return bodies that semantically equal frozen Java JSON fixtures.
//!
//! Does not prove: bridge framing, tokens, Paper events, dirty/render, or that
//! Java exporters still emit those fixtures (`StateExporterTest` remains the
//! exporter oracle).

use serde_json::json;
use squaremap_compare::parity::{ParityProbeManifest, run_manifest};
use squaremap_protocol::wire::{
    Envelope, Icon, IconsReplace, Marker, MarkerCircle, MarkerEllipse, MarkerIcon, MarkerLayer,
    MarkerLayersReplace, MarkerMultiPolygon, MarkerPolygon, MarkerPolyline, MarkerRectangle,
    MarkerStyle, MarkerTooltip, Player, PlayerTrackerSettings, PlayersReplace, Point, PointList,
    Spawn, UiSettings, World, WorldIdentity, WorldStateReplace, ZoomSettings, envelope, marker,
};
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use squaremap_server::views::apply_replacement;
use std::path::Path;
use tempfile::tempdir;

const JAVA_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/bridge/v1/fixtures/java"
);

fn java_fixture(name: &str) -> Vec<u8> {
    let path = Path::new(JAVA_FIXTURES).join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn uuid_bytes() -> Vec<u8> {
    hex::decode("00112233445566778899aabbccddeeff").unwrap()
}

fn point(x: i32, z: i32) -> Point {
    Point { x, z }
}

fn world(
    namespace: &str,
    value: &str,
    display: &str,
    icon: &str,
    environment: &str,
    order: i32,
    tracker: bool,
) -> World {
    World {
        identity: Some(WorldIdentity {
            namespace: namespace.into(),
            value: value.into(),
            epoch: 0,
        }),
        display_name: display.into(),
        icon: icon.into(),
        environment: environment.into(),
        order,
        spawn: Some(Spawn { x: 0, z: 0 }),
        player_tracker: Some(PlayerTrackerSettings {
            enabled: tracker,
            update_interval: 1,
            label: "Players".into(),
            show_controls: true,
            default_hidden: false,
            priority: 0,
            z_index: 100,
            nameplate_enabled: tracker,
            nameplate_show_heads: tracker,
            nameplate_heads_url: "/heads/{uuid}".into(),
            nameplate_show_armor: tracker,
            nameplate_show_health: tracker,
            ..Default::default()
        }),
        zoom: Some(ZoomSettings {
            max: 5,
            r#def: 3,
            extra: 1,
        }),
        marker_update_interval: 5,
        tiles_update_interval: 10,
        ..Default::default()
    }
}

fn production_envelopes() -> Vec<Envelope> {
    let worlds = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 1,
            worlds: vec![
                world("minecraft", "overworld", "World", "default", "normal", 0, true),
                world("minecraft", "nether", "Nether", "nether", "nether", 1, false),
            ],
            ui: Some(UiSettings {
                title: "Squaremap".into(),
                coordinates_enabled: true,
                coordinates_html: "Coordinates".into(),
                link_enabled: true,
                sidebar_pinned: "minecraft_overworld".into(),
                sidebar_player_list_label: "Players".into(),
                sidebar_world_list_label: "Worlds".into(),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let players = Envelope {
        payload: Some(envelope::Payload::PlayersReplace(PlayersReplace {
            revision: 1,
            max_players: 20,
            players: vec![Player {
                name: "public".into(),
                uuid: uuid_bytes(),
                world: Some(WorldIdentity {
                    namespace: "minecraft".into(),
                    value: "overworld".into(),
                    epoch: 0,
                }),
                x: Some(1),
                y: Some(64),
                z: Some(2),
                yaw: Some(90),
                armor: Some(7),
                health: Some(20),
                ..Default::default()
            }],
        })),
        ..Default::default()
    };
    let styled = MarkerStyle {
        stroke: false,
        stroke_color: "#ff0000".into(),
        stroke_weight: 2,
        stroke_opacity: 0.5,
        fill: false,
        fill_color: Some("#00ff00".into()),
        fill_opacity: 0.4,
        fill_rule: "nonzero".into(),
    };
    let markers = Envelope {
        payload: Some(envelope::Payload::MarkerLayersReplace(
            MarkerLayersReplace {
                revision: 1,
                world: Some(WorldIdentity {
                    namespace: "minecraft".into(),
                    value: "overworld".into(),
                    epoch: 0,
                }),
                layers: vec![MarkerLayer {
                    id: "all".into(),
                    label: "All".into(),
                    show_controls: true,
                    default_hidden: false,
                    layer_priority: 0,
                    z_index: 10,
                    timestamp: 42,
                    markers: vec![
                        Marker {
                            geometry: Some(marker::Geometry::Icon(MarkerIcon {
                                point: Some(point(1, 2)),
                                size_x: 16,
                                size_z: 16,
                                anchor: Some(point(8, 16)),
                                tooltip_anchor: Some(point(0, 0)),
                                image: "spawn".into(),
                            })),
                            ..Default::default()
                        },
                        Marker {
                            geometry: Some(marker::Geometry::Circle(MarkerCircle {
                                center: Some(point(3, 4)),
                                radius: 5.5,
                            })),
                            ..Default::default()
                        },
                        Marker {
                            geometry: Some(marker::Geometry::Ellipse(MarkerEllipse {
                                center: Some(point(6, 7)),
                                radius_x: 8.5,
                                radius_z: 9.5,
                            })),
                            ..Default::default()
                        },
                        Marker {
                            geometry: Some(marker::Geometry::Rectangle(MarkerRectangle {
                                point1: Some(point(0, 0)),
                                point2: Some(point(10, 10)),
                            })),
                            ..Default::default()
                        },
                        Marker {
                            geometry: Some(marker::Geometry::Polyline(MarkerPolyline {
                                lines: vec![PointList {
                                    points: vec![point(0, 0), point(1, 1)],
                                }],
                            })),
                            ..Default::default()
                        },
                        Marker {
                            geometry: Some(marker::Geometry::Polyline(MarkerPolyline {
                                lines: vec![
                                    PointList {
                                        points: vec![point(2, 2)],
                                    },
                                    PointList {
                                        points: vec![point(3, 3)],
                                    },
                                ],
                            })),
                            ..Default::default()
                        },
                        Marker {
                            geometry: Some(marker::Geometry::Polygon(MarkerPolygon {
                                main_polygon: vec![point(0, 0), point(10, 0), point(10, 10)],
                                negative_space: vec![PointList {
                                    points: vec![point(2, 2), point(3, 2), point(3, 3)],
                                }],
                            })),
                            ..Default::default()
                        },
                        Marker {
                            style: Some(styled),
                            tooltip: Some(MarkerTooltip {
                                click: Some("click".into()),
                                hover: Some("hover".into()),
                            }),
                            geometry: Some(marker::Geometry::MultiPolygon(MarkerMultiPolygon {
                                polygons: vec![
                                    MarkerPolygon {
                                        main_polygon: vec![point(0, 0), point(1, 0), point(1, 1)],
                                        negative_space: vec![],
                                    },
                                    MarkerPolygon {
                                        main_polygon: vec![point(4, 4), point(5, 4), point(5, 5)],
                                        negative_space: vec![],
                                    },
                                ],
                            })),
                        },
                    ],
                    ..Default::default()
                }],
            },
        )),
        ..Default::default()
    };
    let icons = Envelope {
        payload: Some(envelope::Payload::IconsReplace(IconsReplace {
            revision: 1,
            icons: vec![Icon {
                id: "spawn".into(),
                image: vec![255, 0, 0, 255, 0, 0, 255, 255],
                mime_type: "image/rgba".into(),
                width: 2,
                height: 1,
            }],
        })),
        ..Default::default()
    };
    vec![worlds, players, markers, icons]
}

fn compare_json(name: &str, expected: &[u8], actual: &[u8]) {
    let java = tempdir().unwrap();
    let rust = tempdir().unwrap();
    std::fs::write(java.path().join(name), expected).unwrap();
    std::fs::write(rust.path().join(name), actual).unwrap();
    let manifest_dir = tempdir().unwrap();
    let manifest_path = manifest_dir.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "scenario": "java-view-http",
            "input_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "normalization": "none",
            "outputs": [{"path": name, "type": "json"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let fixture = ParityProbeManifest::load(&manifest_path).unwrap();
    let report = run_manifest(&fixture, java.path(), rust.path())
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    assert!(
        report.mismatches.is_empty() && report.complete,
        "{name} verdict={} missing={:?} extra={:?} mismatches={:?}\njava={}\nrust={}",
        report.verdict,
        report.missing_paths,
        report.extra_paths,
        report.mismatches,
        String::from_utf8_lossy(expected),
        String::from_utf8_lossy(actual)
    );
}

async fn served_bytes(path: &str) -> Vec<u8> {
    let output = tempdir().unwrap();
    let root = OutputRoot::new(output.path()).unwrap();
    for envelope in production_envelopes() {
        apply_replacement(&root, &envelope)
            .unwrap_or_else(|error| panic!("apply_replacement: {error}"));
    }
    let mut server = HttpServer::bind(HttpConfig::loopback(), root)
        .await
        .unwrap();
    let response = reqwest::get(format!(
        "http://{}{path}",
        server.local_addr().unwrap()
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
    let bytes = response.bytes().await.unwrap().to_vec();
    server.shutdown().await.unwrap();
    bytes
}

#[tokio::test]
async fn production_settings_http_matches_java_fixture() {
    compare_json(
        "settings.json",
        &java_fixture("settings.json"),
        &served_bytes("/tiles/settings.json").await,
    );
}

#[tokio::test]
async fn production_players_http_matches_java_fixture() {
    compare_json(
        "players.json",
        &java_fixture("players.json"),
        &served_bytes("/tiles/players.json").await,
    );
}

#[tokio::test]
async fn production_world_settings_http_matches_java_fixture() {
    compare_json(
        "world-settings.json",
        &java_fixture("world-settings.json"),
        &served_bytes("/tiles/minecraft_overworld/settings.json").await,
    );
}

#[tokio::test]
async fn production_markers_http_matches_java_fixture() {
    compare_json(
        "markers.json",
        &java_fixture("markers.json"),
        &served_bytes("/tiles/minecraft_overworld/markers.json").await,
    );
}

#[tokio::test]
async fn production_spawn_icon_http_matches_java_rgba() {
    let bytes = served_bytes("/images/icon/registered/spawn.png").await;
    let (width, height, rgba) = decode_icon_png(&bytes);
    assert_eq!((width, height), (2, 1));
    assert_eq!(rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
}

fn decode_icon_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let mut offset = 8;
    let mut width = 0;
    let mut height = 0;
    let mut idat = Vec::new();
    while offset < bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        let data = &bytes[offset + 8..offset + 8 + length];
        if kind == b"IHDR" {
            width = u32::from_be_bytes(data[..4].try_into().unwrap());
            height = u32::from_be_bytes(data[4..8].try_into().unwrap());
        } else if kind == b"IDAT" {
            idat.extend_from_slice(data);
        }
        offset += 12 + length;
        if kind == b"IEND" {
            break;
        }
    }
    let mut compressed = 2;
    let mut raw = Vec::new();
    while compressed + 5 <= idat.len().saturating_sub(4) {
        let final_block = idat[compressed] == 1;
        let length =
            u16::from_le_bytes(idat[compressed + 1..compressed + 3].try_into().unwrap()) as usize;
        raw.extend_from_slice(&idat[compressed + 5..compressed + 5 + length]);
        compressed += 5 + length;
        if final_block {
            break;
        }
    }
    let row = width as usize * 4;
    let mut pixels = Vec::new();
    for line in raw.chunks_exact(row + 1) {
        pixels.extend_from_slice(&line[1..]);
    }
    (width, height, pixels)
}
