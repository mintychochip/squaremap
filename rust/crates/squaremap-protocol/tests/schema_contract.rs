use prost::Message;
use squaremap_protocol::wire::{
    envelope, marker, BlockStateDescriptor, ChunkSection, ChunkSnapshot, ChunkSnapshotBody,
    ConfigReplace, Envelope, LocaleSettings, Marker, MarkerCircle, MarkerEllipse, MarkerIcon,
    MarkerLayer, MarkerMultiPolygon, MarkerPolygon, MarkerPolyline, MarkerRectangle, Player,
    PlayersReplace, Point, VisibilityLimit, World,
};
fn assert_i32(_: i32) {}

#[test]
fn block_descriptor_air_field_round_trips_true_and_false() {
    for expected in [false, true] {
        let descriptor = BlockStateDescriptor {
            air: expected,
            ..Default::default()
        };
        let decoded = BlockStateDescriptor::decode(descriptor.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.air, expected);
    }
}
#[test]
fn exposes_complete_typed_state_contract() {
    let config = ConfigReplace {
        global: Some(Default::default()),
        advanced: Some(Default::default()),
        world: Some(Default::default()),
        locale: Some(LocaleSettings {
            spawn_marker_label: "Spawn".into(),
            world_border_marker_label: "World Border".into(),
            ..Default::default()
        }),
        render: Some(Default::default()),
        ui: Some(Default::default()),
        ..Default::default()
    };
    let world = World {
        icon: "world".into(),
        order: 1,
        ..Default::default()
    };
    let player = Player {
        world: world.identity.clone(),
        display_name: Some("Display".into()),
        armor: Some(20),
        health: Some(20),
        ..Default::default()
    };
    let players = PlayersReplace {
        players: vec![player],
        max_players: 20,
        ..Default::default()
    };
    let marker = Marker {
        geometry: Some(marker::Geometry::Icon(MarkerIcon::default())),
        style: Some(Default::default()),
        tooltip: Some(Default::default()),
        ..Default::default()
    };
    let geometries = [
        marker::Geometry::Icon(MarkerIcon::default()),
        marker::Geometry::Circle(MarkerCircle::default()),
        marker::Geometry::Ellipse(MarkerEllipse::default()),
        marker::Geometry::Rectangle(MarkerRectangle::default()),
        marker::Geometry::Polyline(MarkerPolyline::default()),
        marker::Geometry::Polygon(MarkerPolygon::default()),
        marker::Geometry::MultiPolygon(MarkerMultiPolygon::default()),
    ];
    let point = Point { x: -1, z: 2 };
    let layer = MarkerLayer {
        visible: true,
        show_controls: true,
        default_hidden: true,
        layer_priority: -1,
        z_index: 2,
        ..Default::default()
    };
    let body = ChunkSnapshotBody {
        sections: vec![ChunkSection::default()],
        ..Default::default()
    };
    let encoded_body = body.encode_to_vec();
    let snapshot = ChunkSnapshot {
        compressed_body: encoded_body.clone(),
        uncompressed_length: encoded_body.len() as u32,
        crc32c: 1,
        ..Default::default()
    };
    let config_envelope = Envelope {
        payload: Some(envelope::Payload::ConfigReplace(config)),
        ..Default::default()
    };
    let players_envelope = Envelope {
        payload: Some(envelope::Payload::PlayersReplace(players)),
        ..Default::default()
    };
    let snapshot_envelope = Envelope {
        payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
        ..Default::default()
    };
    assert!(matches!(
        config_envelope.payload,
        Some(envelope::Payload::ConfigReplace(_))
    ));
    assert!(matches!(
        players_envelope.payload,
        Some(envelope::Payload::PlayersReplace(_))
    ));
    assert!(matches!(
        snapshot_envelope.payload,
        Some(envelope::Payload::ChunkSnapshot(_))
    ));
    assert!(marker.geometry.is_some());
    assert_eq!(geometries.len(), 7);
    assert!(layer.show_controls);
    assert!(layer.visible);
    assert_eq!(point.x, -1);
    assert_eq!(point.z, 2);
    assert_i32(point.x);
    assert_i32(VisibilityLimit::default().center_x);
}
