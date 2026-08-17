use squaremap_protocol::wire::{
    ChunkCoordinate, DirtyReplayItem, DirtyReplayRequest, DirtyResyncComplete, DirtyResyncStatus,
    ReplayStart, ResumeWatermark, WorldIdentity, dirty_replay_request::Cursor,
};
use squaremap_server::dirty_resync::{
    DirtyResyncError, validate_completion, validate_item, validate_request, validate_resume,
};

fn world(epoch: u64) -> WorldIdentity {
    WorldIdentity {
        namespace: "minecraft".into(),
        value: "overworld".into(),
        epoch,
    }
}

fn request() -> DirtyReplayRequest {
    DirtyReplayRequest {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        world: Some(world(1)),
        max_items: 2,
        cursor: Some(Cursor::Start(ReplayStart {})),
    }
}

#[test]
fn resume_watermark_rejects_wrong_bridge_session_or_revision() {
    let watermark = ResumeWatermark {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 11,
        last_durable_sequence: 43,
    };
    assert_eq!(
        validate_resume(&watermark, &[4; 16], &[1; 16], 11),
        Err(DirtyResyncError::InvalidSession)
    );
    assert_eq!(
        validate_resume(&watermark, &[2; 16], &[2; 16], 11),
        Err(DirtyResyncError::InvalidSession)
    );
    assert_eq!(
        validate_resume(
            &ResumeWatermark {
                bridge_id: vec![0; 16],
                ..watermark.clone()
            },
            &[0; 16],
            &[1; 16],
            11
        ),
        Err(DirtyResyncError::InvalidSession)
    );
    assert_eq!(
        validate_resume(&watermark, &[3; 16], &[1; 16], 12),
        Err(DirtyResyncError::RevisionMismatch)
    );
    assert!(validate_resume(&watermark, &[3; 16], &[1; 16], 11).is_ok());
}

#[test]
fn replay_request_rejects_empty_or_oversized_page_and_invalid_world() {
    let mut candidate = request();
    assert!(validate_request(&candidate, &[3; 16], &[1; 16], 4, 2).is_ok());
    candidate.max_items = 0;
    assert_eq!(
        validate_request(&candidate, &[3; 16], &[1; 16], 4, 2),
        Err(DirtyResyncError::InvalidPageSize { max: 2 })
    );
    candidate.max_items = 3;
    assert_eq!(
        validate_request(&candidate, &[3; 16], &[1; 16], 4, 2),
        Err(DirtyResyncError::InvalidPageSize { max: 2 })
    );
    candidate.max_items = 1025;
    assert_eq!(
        validate_request(&candidate, &[3; 16], &[1; 16], 4, 2048),
        Err(DirtyResyncError::InvalidPageSize { max: 1024 })
    );
    candidate.max_items = 1;
    candidate.world.as_mut().unwrap().epoch = 0;
    assert_eq!(
        validate_request(&candidate, &[3; 16], &[1; 16], 4, 2),
        Err(DirtyResyncError::WorldMismatch)
    );
}

#[test]
fn replay_items_reject_duplicate_gap_wrong_identity_and_wrong_epoch() {
    let candidate = request();
    let item = |index, bridge_id, session_id, replay_id, epoch| DirtyReplayItem {
        bridge_id: vec![bridge_id; 16],
        session_id: vec![session_id; 16],
        config_revision: 4,
        replay_id,
        item_index: index,
        world: Some(world(epoch)),
        coordinate: Some(ChunkCoordinate {
            x: index as i32,
            z: 0,
        }),
        revision: 10 + u64::from(index),
    };
    assert!(validate_item(&item(0, 3, 1, 2, 1), &candidate, 0).is_ok());
    assert_eq!(
        validate_item(&item(0, 3, 1, 2, 1), &candidate, 1),
        Err(DirtyResyncError::DuplicateIndex(0))
    );
    assert_eq!(
        validate_item(&item(2, 3, 1, 2, 1), &candidate, 1),
        Err(DirtyResyncError::InvalidIndex {
            expected: 1,
            actual: 2
        })
    );
    assert_eq!(
        validate_item(&item(1, 4, 1, 2, 1), &candidate, 1),
        Err(DirtyResyncError::InvalidSession)
    );
    assert_eq!(
        validate_item(&item(1, 3, 1, 3, 1), &candidate, 1),
        Err(DirtyResyncError::ReplayMismatch)
    );
    assert_eq!(
        validate_item(&item(1, 3, 1, 2, 2), &candidate, 1),
        Err(DirtyResyncError::WorldMismatch)
    );
}

#[test]
fn completion_rejects_wrong_identity_failure_reason_and_count() {
    let candidate = request();
    let failed = DirtyResyncComplete {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_count: 1,
        status: DirtyResyncStatus::Failed as i32,
        failure_reason: String::new(),
        has_more: false,
        last_coordinate: None,
    };
    assert_eq!(
        validate_completion(&failed, &candidate, 1),
        Err(DirtyResyncError::InvalidCompletion)
    );
    let complete = DirtyResyncComplete {
        bridge_id: vec![4; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_count: 2,
        status: DirtyResyncStatus::Complete as i32,
        failure_reason: String::new(),
        has_more: false,
        last_coordinate: None,
    };
    assert_eq!(
        validate_completion(&complete, &candidate, 2),
        Err(DirtyResyncError::InvalidSession)
    );
    let complete = DirtyResyncComplete {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_count: 2,
        status: DirtyResyncStatus::Complete as i32,
        failure_reason: String::new(),
        has_more: false,
        last_coordinate: None,
    };
    assert!(validate_completion(&complete, &candidate, 2).is_ok());
}

#[test]
fn replay_request_rejects_missing_cursor() {
    let mut candidate = request();
    candidate.cursor = None;
    assert_eq!(
        validate_request(&candidate, &[3; 16], &[1; 16], 4, 2),
        Err(DirtyResyncError::MissingCursor)
    );
}

#[test]
fn completion_rejects_has_more_without_last_coordinate() {
    let candidate = request();
    let mut complete = DirtyResyncComplete {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_count: 2,
        status: DirtyResyncStatus::Complete as i32,
        failure_reason: String::new(),
        has_more: false,
        last_coordinate: None,
    };
    assert!(validate_completion(&complete, &candidate, 2).is_ok());
    complete.has_more = true;
    assert_eq!(
        validate_completion(&complete, &candidate, 2),
        Err(DirtyResyncError::InvalidCursor)
    );
    complete.last_coordinate = Some(ChunkCoordinate { x: 1, z: 1 });
    assert!(validate_completion(&complete, &candidate, 2).is_ok());
    complete.item_count = 1;
    assert_eq!(
        validate_completion(&complete, &candidate, 1),
        Err(DirtyResyncError::InvalidCompletion)
    );
}

#[test]
fn replay_completion_and_items_cannot_exceed_requested_page() {
    let candidate = request();
    let item = DirtyReplayItem {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_index: 2,
        world: Some(world(1)),
        coordinate: Some(ChunkCoordinate { x: 0, z: 0 }),
        revision: 10,
    };
    assert_eq!(
        validate_item(&item, &candidate, 2),
        Err(DirtyResyncError::InvalidIndex {
            expected: 2,
            actual: 2
        })
    );
    let completion = DirtyResyncComplete {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_count: 3,
        status: DirtyResyncStatus::Complete as i32,
        failure_reason: String::new(),
        has_more: false,
        last_coordinate: None,
    };
    assert_eq!(
        validate_completion(&completion, &candidate, 3),
        Err(DirtyResyncError::InvalidIndex {
            expected: 2,
            actual: 3
        })
    );
}
#[test]
fn bridge_identity_rejects_missing_or_zero_values() {
    assert_eq!(
        squaremap_server::dirty_resync::validate_bridge_identity(&[]),
        Err(DirtyResyncError::InvalidSession)
    );
    assert_eq!(
        squaremap_server::dirty_resync::validate_bridge_identity(&[0; 16]),
        Err(DirtyResyncError::InvalidSession)
    );
    assert!(squaremap_server::dirty_resync::validate_bridge_identity(&[9; 16]).is_ok());
}

#[test]
fn fresh_session_identity_mismatch_is_rejected() {
    let watermark = ResumeWatermark {
        bridge_id: vec![9; 16],
        session_id: vec![2; 16],
        config_revision: 4,
        last_durable_sequence: 0,
    };
    assert!(validate_resume(&watermark, &[9; 16], &[2; 16], 4).is_ok());
    let request = request();
    assert_eq!(
        validate_request(&request, &[9; 16], &[2; 16], 4, 2),
        Err(DirtyResyncError::InvalidSession)
    );
}
