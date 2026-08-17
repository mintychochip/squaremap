use squaremap_server::output::OutputRoot;
use squaremap_server::views::write_players;
use squaremap_state::view::{PlayerView, PlayersView};

#[test]
fn players_json_omits_hidden_and_optional_fields_and_keeps_latest_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    let view = PlayersView {
        players: vec![PlayerView {
            name: "public".into(),
            display_name: None,
            uuid: "00112233445566778899aabbccddeeff".into(),
            world: "world".into(),
            x: Some(1),
            y: Some(64),
            z: Some(2),
            yaw: Some(90),
            armor: Some(7),
            health: Some(20),
        }],
        max: 20,
    };
    let bytes = write_players(&root, &view).unwrap();
    assert_eq!(bytes, br#"{"players":[{"name":"public","uuid":"00112233445566778899aabbccddeeff","world":"world","x":1,"y":64,"z":2,"yaw":90,"armor":7,"health":20}],"max":20}"#);
    assert_eq!(root.latest_bytes("tiles/players.json").unwrap(), bytes);
}

#[test]
fn replacement_views_preserve_ui_world_names_marker_shapes_timestamps_and_icon_pixels() {
    use squaremap_protocol::wire::{
        Envelope, Icon, IconsReplace, Marker, MarkerLayer, MarkerLayersReplace, MarkerMultiPolygon,
        MarkerPolygon, MarkerPolyline, Point, PointList, UiSettings, World, WorldIdentity,
        WorldStateReplace, envelope, marker,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    let world = World {
        identity: Some(WorldIdentity {
            namespace: "custom".into(),
            value: "overworld".into(),
            ..Default::default()
        }),
        display_name: "Custom".into(),
        ..Default::default()
    };
    let world_envelope = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 1,
            worlds: vec![world],
            ui: Some(UiSettings {
                title: "Squaremap".into(),
                coordinates_enabled: true,
                coordinates_html: "Coordinates".into(),
                link_enabled: true,
                sidebar_pinned: "custom_overworld".into(),
                sidebar_player_list_label: "Players".into(),
                sidebar_world_list_label: "Worlds".into(),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &world_envelope).unwrap();
    let settings = String::from_utf8(root.latest_bytes("tiles/settings.json").unwrap()).unwrap();
    assert!(
        settings.contains("\"name\":\"custom_overworld\""),
        "{settings}"
    );
    assert!(settings.contains("\"title\":\"Squaremap\""));
    assert!(
        root.latest_bytes("tiles/custom_overworld/settings.json")
            .is_some()
    );

    let marker = Marker {
        geometry: Some(marker::Geometry::MultiPolygon(MarkerMultiPolygon {
            polygons: vec![MarkerPolygon {
                main_polygon: vec![
                    Point { x: 0, z: 0 },
                    Point { x: 2, z: 0 },
                    Point { x: 2, z: 2 },
                ],
                negative_space: vec![PointList {
                    points: vec![Point { x: 1, z: 1 }, Point { x: 1, z: 2 }],
                }],
            }],
        })),
        ..Default::default()
    };
    let single_line = Marker {
        geometry: Some(marker::Geometry::Polyline(MarkerPolyline {
            lines: vec![PointList {
                points: vec![Point { x: 3, z: 4 }, Point { x: 5, z: 6 }],
            }],
        })),
        ..Default::default()
    };
    let multi_line = Marker {
        geometry: Some(marker::Geometry::Polyline(MarkerPolyline {
            lines: vec![
                PointList {
                    points: vec![Point { x: 7, z: 8 }, Point { x: 9, z: 10 }],
                },
                PointList {
                    points: vec![Point { x: 11, z: 12 }, Point { x: 13, z: 14 }],
                },
            ],
        })),
        ..Default::default()
    };
    let polygon = Marker {
        geometry: Some(marker::Geometry::Polygon(MarkerPolygon {
            main_polygon: vec![Point { x: 15, z: 16 }, Point { x: 17, z: 18 }],
            negative_space: vec![PointList {
                points: vec![Point { x: 19, z: 20 }, Point { x: 21, z: 22 }],
            }],
        })),
        ..Default::default()
    };
    let marker_envelope = Envelope {
        payload: Some(envelope::Payload::MarkerLayersReplace(
            MarkerLayersReplace {
                revision: 1,
                world: Some(WorldIdentity {
                    namespace: "custom".into(),
                    value: "overworld".into(),
                    ..Default::default()
                }),
                layers: vec![MarkerLayer {
                    id: "layer".into(),
                    timestamp: 42,
                    markers: vec![marker, single_line, multi_line, polygon],
                    ..Default::default()
                }],
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &marker_envelope).unwrap();
    let markers = String::from_utf8(
        root.latest_bytes("tiles/custom_overworld/markers.json")
            .unwrap(),
    )
    .unwrap();
    assert!(markers.contains("\"timestamp\":42"));
    assert!(markers.contains("\"type\":\"polygon\""));
    assert!(markers.contains("\"points\":[[[{\"x\":0,\"z\":0}"));
    assert!(markers.contains("\"points\":[{\"x\":3,\"z\":4},{\"x\":5,\"z\":6}]"));
    assert!(markers.contains("\"points\":[[{\"x\":7,\"z\":8},{\"x\":9,\"z\":10}],[{\"x\":11,\"z\":12},{\"x\":13,\"z\":14}]]"));
    assert!(markers.contains("\"points\":[[{\"x\":15,\"z\":16},{\"x\":17,\"z\":18}],[{\"x\":19,\"z\":20},{\"x\":21,\"z\":22}]]"));

    let reloaded_world = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 2,
            worlds: vec![World {
                identity: Some(WorldIdentity {
                    namespace: "custom".into(),
                    value: "overworld".into(),
                    epoch: 1,
                }),
                display_name: "Reloaded".into(),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &reloaded_world).unwrap();
    let reloaded_settings = root
        .latest_bytes("tiles/custom_overworld/settings.json")
        .unwrap();
    assert!(!reloaded_settings.is_empty());
    assert!(
        root.latest_bytes("tiles/custom_overworld/markers.json")
            .is_none()
    );

    let icon_envelope = Envelope {
        payload: Some(envelope::Payload::IconsReplace(IconsReplace {
            revision: 1,
            icons: vec![Icon {
                id: "pixel".into(),
                image: vec![255, 0, 0, 255],
                mime_type: "image/rgba".into(),
                width: 1,
                height: 1,
            }],
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &icon_envelope).unwrap();
    let png = root
        .latest_bytes("images/icon/registered/pixel.png")
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(!temp.path().join("tiles/icons.json").exists());
    assert_eq!(root.canonical_state().unwrap().icons_revision, 1);
    assert!(squaremap_server::views::apply_replacement(&root, &icon_envelope).is_err());
    let empty_icons = Envelope {
        payload: Some(envelope::Payload::IconsReplace(IconsReplace {
            revision: 2,
            icons: vec![],
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &empty_icons).unwrap();
    assert!(
        !temp
            .path()
            .join("images/icon/registered/pixel.png")
            .exists()
    );
    let empty_worlds = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 3,
            worlds: vec![],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &empty_worlds).unwrap();
    assert!(
        !temp
            .path()
            .join("tiles/custom_overworld/settings.json")
            .exists()
    );
    assert!(
        !temp
            .path()
            .join("tiles/custom_overworld/markers.json")
            .exists()
    );
}
#[test]
fn checked_in_legacy_fixtures_are_consumed() {
    let empty: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/bridge/v1/views/empty.json"
    ))
    .unwrap();
    let settings: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/bridge/v1/views/settings.json"
    ))
    .unwrap();
    let world_settings: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/bridge/v1/views/world-settings.json"
    ))
    .unwrap();
    let players: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/bridge/v1/views/players.json"
    ))
    .unwrap();
    let icons: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/bridge/v1/views/icons.json"
    ))
    .unwrap();
    let markers: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/bridge/v1/views/markers.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    let production_icons = squaremap_protocol::wire::Envelope {
        payload: Some(squaremap_protocol::wire::envelope::Payload::IconsReplace(
            squaremap_protocol::wire::IconsReplace {
                revision: 1,
                icons: vec![squaremap_protocol::wire::Icon {
                    id: "spawn".into(),
                    image: vec![255, 0, 0, 255, 0, 0, 255, 255],
                    mime_type: "image/rgba".into(),
                    width: 2,
                    height: 1,
                }],
            },
        )),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &production_icons).unwrap();
    assert!(!temp.path().join("tiles/icons.json").exists());
    let actual_icons: serde_json::Value =
        serde_json::from_slice(&root.canonical_state().unwrap().icons).unwrap();
    assert_eq!(icons, actual_icons);
    let png = root
        .latest_bytes("images/icon/registered/spawn.png")
        .unwrap();
    assert_eq!(
        decode_rgba_png(&png),
        (2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255])
    );
    assert_eq!(empty["worlds"].as_array().unwrap().len(), 0);
    assert_eq!(settings["worlds"][0]["name"], "minecraft_overworld");
    assert_eq!(settings["ui"]["coordinates"]["enabled"], true);
    assert_eq!(world_settings["zoom"]["max"], 5);
    assert_eq!(players["players"][0]["armor"], 7);
    assert_eq!(players["players"][0]["health"], 20);
    assert_eq!(icons["icons"][0]["id"], "spawn");
    assert_eq!(markers[0]["timestamp"], 42);
    let marker_list = markers[0]["markers"].as_array().unwrap();
    assert_eq!(marker_list.len(), 8);
    assert_eq!(
        marker_list
            .iter()
            .map(|marker| marker["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "icon",
            "circle",
            "ellipse",
            "rectangle",
            "polyline",
            "polyline",
            "polygon",
            "polygon"
        ]
    );
    assert_eq!(marker_list[7]["fillRule"], "nonzero");
    assert_eq!(marker_list[7]["tooltip"], "hover");
}
#[test]
fn multipolygon_geometry_uses_distinct_type_tag() {
    let geometry = squaremap_state::view::MarkerGeometryView::MultiPolygon {
        points: vec![vec![vec![squaremap_state::view::ViewPoint { x: 1, z: 2 }]]],
    };
    let value: serde_json::Value =
        serde_json::from_slice(&squaremap_state::view::serialize_json(&geometry).unwrap()).unwrap();
    assert_eq!(value["type"], "multipolygon");
    let decoded: squaremap_state::view::MarkerGeometryView = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, geometry);
}
fn decode_rgba_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
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
    assert_eq!(&idat[..2], &[0x78, 0x01]);
    let mut compressed = 2;
    let mut raw = Vec::new();
    while compressed + 5 <= idat.len() - 4 {
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
    let mut pixels = Vec::with_capacity(row * height as usize);
    for line in raw.chunks_exact(row + 1) {
        assert_eq!(line[0], 0);
        pixels.extend_from_slice(&line[1..]);
    }
    (width, height, pixels)
}

#[test]
fn replacement_views_sort_worlds_players_markers_and_icons() {
    use squaremap_protocol::wire::{
        Envelope, Icon, IconsReplace, MarkerLayer, MarkerLayersReplace, Player, PlayersReplace,
        World, WorldIdentity, WorldStateReplace, envelope,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    let worlds = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 1,
            worlds: vec![
                World {
                    identity: Some(WorldIdentity {
                        namespace: "z".into(),
                        value: "world".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                World {
                    identity: Some(WorldIdentity {
                        namespace: "a".into(),
                        value: "world".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &worlds).unwrap();
    let settings = String::from_utf8(root.latest_bytes("tiles/settings.json").unwrap()).unwrap();
    assert!(
        settings.find("\"name\":\"a_world\"").unwrap()
            < settings.find("\"name\":\"z_world\"").unwrap()
    );

    let markers = Envelope {
        payload: Some(envelope::Payload::MarkerLayersReplace(
            MarkerLayersReplace {
                revision: 1,
                world: Some(WorldIdentity {
                    namespace: "a".into(),
                    value: "world".into(),
                    ..Default::default()
                }),
                layers: vec![
                    MarkerLayer {
                        id: "z".into(),
                        ..Default::default()
                    },
                    MarkerLayer {
                        id: "a".into(),
                        ..Default::default()
                    },
                ],
            },
        )),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &markers).unwrap();
    let marker_json =
        String::from_utf8(root.latest_bytes("tiles/a_world/markers.json").unwrap()).unwrap();
    assert!(marker_json.find("\"id\":\"a\"").unwrap() < marker_json.find("\"id\":\"z\"").unwrap());

    let icons = Envelope {
        payload: Some(envelope::Payload::IconsReplace(IconsReplace {
            revision: 1,
            icons: vec![
                Icon {
                    id: "z".into(),
                    image: vec![0, 0, 0, 255],
                    width: 1,
                    height: 1,
                    ..Default::default()
                },
                Icon {
                    id: "a".into(),
                    image: vec![0, 0, 0, 255],
                    width: 1,
                    height: 1,
                    ..Default::default()
                },
            ],
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &icons).unwrap();
    let icon_json = String::from_utf8(root.canonical_state().unwrap().icons.clone()).unwrap();
    assert!(icon_json.find("\"id\":\"a\"").unwrap() < icon_json.find("\"id\":\"z\"").unwrap());
    assert!(!temp.path().join("tiles/icons.json").exists());

    let players = Envelope {
        payload: Some(envelope::Payload::PlayersReplace(PlayersReplace {
            revision: 1,
            players: vec![
                Player {
                    uuid: vec![255],
                    world: Some(WorldIdentity {
                        namespace: "a".into(),
                        value: "world".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Player {
                    uuid: vec![0],
                    world: Some(WorldIdentity {
                        namespace: "a".into(),
                        value: "world".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &players).unwrap();
    let player_json = String::from_utf8(root.latest_bytes("tiles/players.json").unwrap()).unwrap();
    assert!(
        player_json.find("\"uuid\":\"00\"").unwrap() < player_json.find("\"uuid\":\"ff\"").unwrap()
    );
}
#[test]
fn unsupported_and_invalid_replacements_do_not_commit_state() {
    use squaremap_protocol::wire::{
        ConfigReplace, Envelope, Marker, MarkerLayer, MarkerLayersReplace, WorldIdentity, envelope,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    let unsupported = Envelope {
        payload: Some(envelope::Payload::ConfigReplace(ConfigReplace::default())),
        ..Default::default()
    };
    let error = squaremap_server::views::apply_replacement(&root, &unsupported).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(root.canonical_state().unwrap().worlds_revision, 0);

    let invalid = Envelope {
        payload: Some(envelope::Payload::MarkerLayersReplace(
            MarkerLayersReplace {
                revision: 1,
                world: Some(WorldIdentity {
                    namespace: "minecraft".into(),
                    value: "overworld".into(),
                    ..Default::default()
                }),
                layers: vec![MarkerLayer {
                    markers: vec![Marker::default()],
                    ..Default::default()
                }],
            },
        )),
        ..Default::default()
    };
    let error = squaremap_server::views::apply_replacement(&root, &invalid).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(root.canonical_state().unwrap().marker_revisions.len(), 0);
    assert!(
        root.latest_bytes("tiles/minecraft_overworld/markers.json")
            .is_none()
    );
}
#[test]
fn initial_out_of_order_replacements_converge_and_epoch_validation_is_atomic() {
    use squaremap_protocol::wire::{
        Envelope, MarkerLayer, MarkerLayersReplace, Player, PlayersReplace, World, WorldIdentity,
        WorldStateReplace, envelope,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    let identity = WorldIdentity {
        namespace: "custom".into(),
        value: "overworld".into(),
        epoch: 9,
    };
    let players = Envelope {
        payload: Some(envelope::Payload::PlayersReplace(PlayersReplace {
            revision: 1,
            players: vec![Player {
                uuid: vec![1],
                world: Some(identity.clone()),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &players).unwrap();
    let markers = Envelope {
        payload: Some(envelope::Payload::MarkerLayersReplace(
            MarkerLayersReplace {
                revision: 1,
                world: Some(identity.clone()),
                layers: vec![MarkerLayer {
                    id: "layer".into(),
                    ..Default::default()
                }],
            },
        )),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &markers).unwrap();
    let worlds = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 1,
            worlds: vec![World {
                identity: Some(identity.clone()),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &worlds).unwrap();

    let marker_revision_two = Envelope {
        payload: Some(envelope::Payload::MarkerLayersReplace(
            MarkerLayersReplace {
                revision: 2,
                world: Some(identity.clone()),
                layers: vec![MarkerLayer {
                    id: "layer".into(),
                    ..Default::default()
                }],
            },
        )),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &marker_revision_two).unwrap();
    assert!(squaremap_server::views::apply_replacement(&root, &marker_revision_two).is_err());
    let before = root
        .latest_bytes("tiles/custom_overworld/markers.json")
        .unwrap();
    let invalid_players = Envelope {
        payload: Some(envelope::Payload::PlayersReplace(PlayersReplace {
            revision: 2,
            players: vec![Player {
                uuid: vec![1],
                world: Some(WorldIdentity {
                    epoch: 10,
                    ..identity.clone()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(squaremap_server::views::apply_replacement(&root, &invalid_players).is_err());
    assert_eq!(
        root.latest_bytes("tiles/players.json").unwrap(),
        root.canonical_state().unwrap().players
    );
    assert_eq!(
        root.latest_bytes("tiles/custom_overworld/markers.json")
            .unwrap(),
        before
    );

    let newer_world = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 2,
            worlds: vec![World {
                identity: Some(WorldIdentity {
                    epoch: 10,
                    ..identity
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &newer_world).unwrap();
    assert!(
        !temp
            .path()
            .join("tiles/custom_overworld/markers.json")
            .exists()
    );
    assert!(
        !root
            .canonical_state()
            .unwrap()
            .marker_revisions
            .contains_key("custom_overworld")
    );
}

#[test]
fn replacement_reconciles_files_left_by_an_existing_process() {
    use squaremap_protocol::wire::{
        Envelope, Icon, IconsReplace, World, WorldIdentity, WorldStateReplace, envelope,
    };
    let temp = tempfile::tempdir().unwrap();
    {
        let root = OutputRoot::new(temp.path()).unwrap();
        root.atomic_write("images/icon/registered/stale.png", b"stale")
            .unwrap();
        root.atomic_write("tiles/old_world/settings.json", b"stale")
            .unwrap();
        root.atomic_write("tiles/old_world/markers.json", b"stale")
            .unwrap();
    }
    let root = OutputRoot::new(temp.path()).unwrap();
    let icons = Envelope {
        payload: Some(envelope::Payload::IconsReplace(IconsReplace {
            revision: 1,
            icons: vec![Icon {
                id: "active".into(),
                image: vec![1, 2, 3, 255],
                width: 1,
                height: 1,
                ..Default::default()
            }],
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &icons).unwrap();
    assert!(
        !temp
            .path()
            .join("images/icon/registered/stale.png")
            .exists()
    );
    let worlds = Envelope {
        payload: Some(envelope::Payload::WorldStateReplace(WorldStateReplace {
            revision: 1,
            worlds: vec![World {
                identity: Some(WorldIdentity {
                    namespace: "new".into(),
                    value: "world".into(),
                    epoch: 1,
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    squaremap_server::views::apply_replacement(&root, &worlds).unwrap();
    assert!(!temp.path().join("tiles/old_world/settings.json").exists());
    assert!(!temp.path().join("tiles/old_world/markers.json").exists());
    assert!(temp.path().join("tiles/new_world/settings.json").exists());
}

#[cfg(unix)]
#[test]
fn removal_never_follows_symlinked_components_outside_root() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.json"), b"secret").unwrap();
    let root = OutputRoot::new(temp.path()).unwrap();
    symlink(outside.path(), temp.path().join("escape")).unwrap();
    assert!(root.remove("escape/secret.json").is_err());
    assert!(outside.path().join("secret.json").exists());
}
