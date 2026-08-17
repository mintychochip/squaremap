#[path = "../src/session.rs"]
mod session;

use session::Session;
use squaremap_protocol::wire::{
    AckStatus, BackendResultCode, ControlKind, ControlRequest, Envelope, HelloAck, WorldIdentity,
    envelope,
};
use squaremap_server::control::ControlState;

fn world(epoch: u64) -> WorldIdentity {
    WorldIdentity {
        namespace: "minecraft".into(),
        value: "overworld".into(),
        epoch,
    }
}

fn control(kind: ControlKind, identity: Option<WorldIdentity>) -> ControlRequest {
    ControlRequest {
        kind: kind as i32,
        world: identity,
        center_x: 0,
        center_z: 0,
        radius: 4,
        coordinates: Vec::new(),
    }
}

fn message(session_id: &[u8], sequence: u64) -> Envelope {
    Envelope {
        protocol_major: 1,
        protocol_minor: 0,
        session_id: session_id.to_vec(),
        sequence,
        correlation_id: 0,
        payload: Some(envelope::Payload::HelloAck(HelloAck {
            protocol_major: 1,
            protocol_minor: 0,
            backend_version: String::new(),
            accepted: true,
            rejection_reason: String::new(),
        })),
    }
}

#[test]
fn control_results_preserve_java_world_and_epoch_contract() {
    let mut state = ControlState::default();
    state.replace_worlds(vec![world(7)]);
    let health = state.handle(&control(ControlKind::Health, None));
    assert_eq!(health.code, BackendResultCode::Healthy as i32);
    let stale = state.handle(&control(ControlKind::FullRender, Some(world(0))));
    assert_eq!(stale.code, BackendResultCode::UnknownWorld as i32);
    assert_eq!(
        stale.substitutions.first().map(|s| s.key.as_str()),
        Some("world")
    );
}

#[tokio::test]
async fn session_fences_duplicates_gaps_and_wrong_sessions() {
    let id = [2_u8; 16];
    let mut session = Session::new([1; 16], id);
    let first = session
        .process(message(&id, 1), |_| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(first.ack().unwrap().status, AckStatus::Accepted as i32);
    let duplicate = session
        .process(message(&id, 1), |_| async {
            panic!("duplicate handler invoked")
        })
        .await
        .unwrap();
    assert_eq!(duplicate.ack().unwrap().status, AckStatus::Duplicate as i32);
    let gap = session
        .process(message(&id, 3), |_| async { panic!("gap handler invoked") })
        .await
        .unwrap();
    assert!(gap.protocol_error().is_some_and(|error| error.fatal));
    let wrong = session
        .process(message(&[9_u8; 16], 2), |_| async {
            panic!("wrong-session handler invoked")
        })
        .await
        .unwrap();
    assert!(wrong.protocol_error().is_some_and(|error| error.fatal));
}
