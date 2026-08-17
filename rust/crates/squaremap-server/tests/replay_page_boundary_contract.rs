use squaremap_protocol::wire::{
    ChunkCoordinate, DirtyReplayItem, DirtyReplayRequest, DirtyResyncComplete, DirtyResyncStatus,
    ReplayStart, ResumeWatermark, WorldIdentity, dirty_replay_request::Cursor,
};
use squaremap_server::dirty_resync::{
    DirtyResyncError, validate_completion, validate_item, validate_request,
};

fn world() -> WorldIdentity {
    WorldIdentity {
        namespace: "minecraft".into(),
        value: "overworld".into(),
        epoch: 1,
    }
}

fn request() -> DirtyReplayRequest {
    DirtyReplayRequest {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        world: Some(world()),
        max_items: 1024,
        cursor: Some(Cursor::Start(ReplayStart {})),
    }
}

#[test]
fn maximum_replay_page_accepts_exactly_1024_items_and_completes_with_more() {
    let candidate = request();
    assert!(validate_request(&candidate, &[3; 16], &[1; 16], 4, 2048).is_ok());
    for index in 0..1024_u32 {
        let item = DirtyReplayItem {
            bridge_id: vec![3; 16],
            session_id: vec![1; 16],
            config_revision: 4,
            replay_id: 2,
            item_index: index,
            world: Some(world()),
            coordinate: Some(ChunkCoordinate {
                x: index as i32,
                z: 0,
            }),
            revision: index as u64 + 1,
        };
        assert!(
            validate_item(&item, &candidate, index).is_ok(),
            "item {index} rejected"
        );
    }
    let completion = DirtyResyncComplete {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_count: 1024,
        status: DirtyResyncStatus::Complete as i32,
        failure_reason: String::new(),
        has_more: true,
        last_coordinate: Some(ChunkCoordinate { x: 1023, z: 0 }),
    };
    assert!(validate_completion(&completion, &candidate, 1024).is_ok());
}

#[test]
fn replay_item_1024_is_rejected_instead_of_emitted_as_a_1025th_item() {
    let candidate = request();
    let item = DirtyReplayItem {
        bridge_id: vec![3; 16],
        session_id: vec![1; 16],
        config_revision: 4,
        replay_id: 2,
        item_index: 1024,
        world: Some(world()),
        coordinate: Some(ChunkCoordinate { x: 1024, z: 0 }),
        revision: 1025,
    };
    assert_eq!(
        validate_item(&item, &candidate, 1024),
        Err(DirtyResyncError::InvalidIndex {
            expected: 1024,
            actual: 1024
        })
    );
}
