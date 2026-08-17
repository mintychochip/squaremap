#[path = "../src/session.rs"]
mod session;

use session::{Decision, Session, SessionError};
use squaremap_protocol::wire::{
    ChunkCoordinate, ChunkDirty, Envelope, HelloAck, WorldIdentity, envelope,
};
use squaremap_state::{Repository, World};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

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
fn dirty_message(session_id: &[u8], sequence: u64, epoch: u64) -> Envelope {
    Envelope {
        protocol_major: 1,
        protocol_minor: 0,
        session_id: session_id.to_vec(),
        sequence,
        correlation_id: 0,
        payload: Some(envelope::Payload::ChunkDirty(ChunkDirty {
            world: Some(WorldIdentity {
                namespace: "minecraft".into(),
                value: "overworld".into(),
                epoch,
            }),
            coordinate: Some(ChunkCoordinate { x: 4, z: 8 }),
            revision: 7,
        })),
    }
}

#[test]
fn cursor_makes_exact_new_duplicate_and_latched_gap_decisions() {
    let mut cursor = session::SessionCursor::new();
    assert_eq!(cursor.accept(1), Decision::New);
    assert_eq!(cursor.accept(1), Decision::Duplicate);
    assert_eq!(
        cursor.accept(3),
        Decision::Gap {
            expected: 2,
            actual: 3
        }
    );
    assert_eq!(
        cursor.accept(2),
        Decision::Gap {
            expected: 2,
            actual: 2
        }
    );
}

#[tokio::test]
async fn duplicate_is_acknowledged_without_invoking_handler() {
    let id = [7_u8; 16];
    let mut server = Session::new([7; 16], id);
    let calls = Rc::new(Cell::new(0_u32));
    let first_calls = Rc::clone(&calls);
    let first = server
        .process(message(&id, 1), move |_| {
            first_calls.set(first_calls.get() + 1);
            async { Ok::<_, SessionError>(()) }
        })
        .await
        .unwrap();
    assert_eq!(first.ack().unwrap().acknowledged_sequence, 1);
    let duplicate_calls = Rc::clone(&calls);
    let duplicate = server
        .process(message(&id, 1), move |_| {
            duplicate_calls.set(duplicate_calls.get() + 1);
            async { Ok::<_, SessionError>(()) }
        })
        .await
        .unwrap();
    assert_eq!(duplicate.ack().unwrap().status, 2);
    assert_eq!(calls.get(), 1);
}

#[tokio::test]
async fn repository_dirty_ack_follows_commit_and_duplicate_skips_reapplication() {
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(
        Repository::open(directory.path().join("state.sqlite"))
            .await
            .unwrap(),
    );
    repository
        .apply_world(World::new("minecraft", "overworld", 1, Vec::new()))
        .await
        .unwrap();
    let id = [21_u8; 16];
    let mut session = Session::new([21; 16], id);
    let accepted = session
        .process_with_repository(dirty_message(&id, 1, 1), Arc::clone(&repository))
        .await
        .unwrap();
    assert_eq!(accepted.ack().unwrap().acknowledged_sequence, 1);
    assert_eq!(repository.recover().await.unwrap().dirty.len(), 1);
    let duplicate = session
        .process_with_repository(dirty_message(&id, 1, 1), repository)
        .await
        .unwrap();
    assert_eq!(duplicate.ack().unwrap().status, 2);
}

#[tokio::test]
async fn repository_failure_and_unrelated_new_payload_produce_no_ack_or_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(
        Repository::open(directory.path().join("state.sqlite"))
            .await
            .unwrap(),
    );
    repository
        .apply_world(World::new("minecraft", "overworld", 2, Vec::new()))
        .await
        .unwrap();
    let id = [22_u8; 16];
    let mut session = Session::new([22; 16], id);
    let stale = session
        .process_with_repository(dirty_message(&id, 1, 1), Arc::clone(&repository))
        .await;
    assert!(stale.is_err());
    assert!(repository.recover().await.unwrap().checkpoints.is_empty());
    let unrelated = session
        .process_with_repository(message(&id, 1), repository)
        .await;
    assert!(unrelated.is_err());
}

#[tokio::test]
async fn gap_returns_structured_full_bootstrap_error_and_latches_processing() {
    let id = [8_u8; 16];
    let mut server = Session::new([8; 16], id);
    let calls = Rc::new(Cell::new(0_u32));
    let gap = server
        .process(message(&id, 2), {
            let calls = Rc::clone(&calls);
            move |_| {
                calls.set(calls.get() + 1);
                async { Ok::<_, SessionError>(()) }
            }
        })
        .await
        .unwrap();
    let error = gap.protocol_error().unwrap();
    assert!(error.message.contains("expected 1"));
    assert!(error.message.contains("actual 2"));
    assert!(error.message.contains("full bootstrap replacement"));
    assert!(error.fatal);
    let after_gap = server
        .process(message(&id, 1), {
            let calls = Rc::clone(&calls);
            move |_| {
                calls.set(calls.get() + 1);
                async { Ok::<_, SessionError>(()) }
            }
        })
        .await
        .unwrap();
    assert!(after_gap.protocol_error().is_some());
    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn wrong_session_is_rejected_without_handler_call() {
    let id = [9_u8; 16];
    let mut server = Session::new([9; 16], id);
    let wrong = [10_u8; 16];
    let calls = Rc::new(Cell::new(0_u32));
    let result = server
        .process(message(&wrong, 1), {
            let calls = Rc::clone(&calls);
            move |_| {
                calls.set(calls.get() + 1);
                async { Ok::<_, SessionError>(()) }
            }
        })
        .await
        .unwrap();
    assert!(result.protocol_error().unwrap().message.contains("session"));
    assert_eq!(calls.get(), 0);
}
#[tokio::test]
async fn replacing_bridge_identity_preserves_sequence_cursor() {
    let id = [31_u8; 16];
    let mut server = Session::new([0; 16], id);
    let first = server
        .process(message(&id, 1), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert!(first.ack().is_some());
    server.replace_bridge_id([32; 16]).unwrap();
    let duplicate = server
        .process(message(&id, 1), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert_eq!(duplicate.ack().unwrap().status, 2);
    let gap = server
        .process(message(&id, 3), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert!(gap.protocol_error().is_some());
}

#[tokio::test]
async fn failed_handler_produces_no_ack_and_does_not_advance_cursor() {
    let id = [11_u8; 16];
    let mut server = Session::new([11; 16], id);
    let result = server
        .process(message(&id, 1), |_| async {
            Err::<(), _>(SessionError::HandlerFailed("durable write failed".into()))
        })
        .await;
    assert!(matches!(result, Err(SessionError::HandlerFailed(_))));
    let success = server
        .process(message(&id, 1), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert_eq!(success.ack().unwrap().acknowledged_sequence, 1);
}
#[tokio::test]
async fn cancelled_handler_does_not_consume_sequence() {
    let id = [12_u8; 16];
    let mut server = Session::new([12; 16], id);
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(10),
        server.process(message(&id, 1), |_| async {
            std::future::pending::<Result<(), SessionError>>().await
        }),
    )
    .await;
    assert!(cancelled.is_err());
    let retry = server
        .process(message(&id, 1), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert_eq!(retry.ack().unwrap().acknowledged_sequence, 1);
}

#[tokio::test]
async fn panicking_handler_does_not_consume_sequence() {
    let id = [13_u8; 16];
    let mut server = Session::new([13; 16], id);
    let panicked = server
        .process(message(&id, 1), |_| async {
            panic!("handler panic");
            #[allow(unreachable_code)]
            Ok::<(), SessionError>(())
        })
        .await;
    assert!(matches!(panicked, Err(SessionError::HandlerPanicked(_))));
    let retry = server
        .process(message(&id, 1), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert_eq!(retry.ack().unwrap().acknowledged_sequence, 1);
}
#[tokio::test]
async fn synchronously_panicking_handler_does_not_consume_sequence() {
    let id = [14_u8; 16];
    let mut server = Session::new([14; 16], id);
    let panicked = server
        .process(
            message(&id, 1),
            |_| -> std::future::Ready<Result<(), SessionError>> {
                panic!("synchronous handler panic");
            },
        )
        .await;
    assert!(matches!(panicked, Err(SessionError::HandlerPanicked(_))));
    let retry = server
        .process(message(&id, 1), |_| async { Ok::<_, SessionError>(()) })
        .await
        .unwrap();
    assert_eq!(retry.ack().unwrap().acknowledged_sequence, 1);
}

#[test]
fn authenticated_ids_are_rfc4122_v4() {
    for _ in 0..32 {
        let session = Session::authenticated([1; 16]);
        let id = session.session_id();
        assert_eq!(id[6] >> 4, 4);
        assert_eq!(id[8] & 0xc0, 0x80);
    }
}

#[test]
fn reconnect_requires_fresh_valid_uuid_and_resets_ordering() {
    let mut first = Session::authenticated([1; 16]);
    let mut second = Session::authenticated([1; 16]);
    assert_ne!(first.session_id(), second.session_id());
    assert_eq!(first.session_id().len(), 16);
    assert_eq!(second.session_id().len(), 16);
    assert_eq!(first.cursor_mut().accept(1), Decision::New);
    assert_eq!(second.cursor_mut().accept(1), Decision::New);
}

#[tokio::test]
async fn three_consecutive_new_messages_advance_and_ack_each_sequence() {
    let id = [40_u8; 16];
    let mut server = Session::new([40; 16], id);
    for sequence in 1..=3 {
        let outcome = server
            .process(message(&id, sequence), |_| async {
                Ok::<_, SessionError>(())
            })
            .await
            .unwrap();
        let ack = outcome.ack().expect("new payload should be accepted");
        assert_eq!(ack.acknowledged_sequence, sequence);
    }
}
