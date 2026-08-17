use squaremap_protocol::wire::{
    Envelope, MarkerLayersReplace, PlayersReplace, World, WorldIdentity, WorldStateReplace,
    envelope,
};
use squaremap_server::output::OutputRoot;
use squaremap_server::views::apply_replacement;

use tempfile::tempdir;

fn identity(epoch: u64) -> WorldIdentity {
    WorldIdentity {
        namespace: "minecraft".into(),
        value: "overworld".into(),
        epoch,
    }
}

fn envelope(payload: envelope::Payload) -> Envelope {
    Envelope {
        protocol_major: 1,
        protocol_minor: 0,
        session_id: vec![1; 16],
        sequence: 1,
        correlation_id: 0,
        payload: Some(payload),
    }
}

#[test]
fn players_replacement_is_sorted_and_stale_revision_is_rejected() {
    let directory = tempdir().unwrap();
    let root = OutputRoot::new(directory.path()).unwrap();
    let first = PlayersReplace {
        revision: 2,
        players: Vec::new(),
        max_players: 20,
    };
    assert!(
        apply_replacement(&root, &envelope(envelope::Payload::PlayersReplace(first)))
            .unwrap()
            .is_some()
    );
    let stale = PlayersReplace {
        revision: 1,
        players: Vec::new(),
        max_players: 20,
    };
    assert!(apply_replacement(&root, &envelope(envelope::Payload::PlayersReplace(stale))).is_err());
}

#[test]
fn marker_replacement_requires_matching_world_epoch_after_world_state_is_known() {
    let directory = tempdir().unwrap();
    let root = OutputRoot::new(directory.path()).unwrap();
    let world = World {
        identity: Some(identity(7)),
        ..World::default()
    };
    let worlds = WorldStateReplace {
        revision: 1,
        worlds: vec![world],
        ui: None,
    };
    assert!(
        apply_replacement(
            &root,
            &envelope(envelope::Payload::WorldStateReplace(worlds))
        )
        .unwrap()
        .is_some()
    );

    let stale = MarkerLayersReplace {
        revision: 1,
        world: Some(identity(6)),
        layers: Vec::new(),
    };
    assert!(
        apply_replacement(
            &root,
            &envelope(envelope::Payload::MarkerLayersReplace(stale))
        )
        .is_err()
    );

    let current = MarkerLayersReplace {
        revision: 1,
        world: Some(identity(7)),
        layers: Vec::new(),
    };
    assert!(
        apply_replacement(
            &root,
            &envelope(envelope::Payload::MarkerLayersReplace(current))
        )
        .unwrap()
        .is_some()
    );
}
