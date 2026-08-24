use squaremap_state::{
    ChunkCoordinate, DirtyLease, Repository, World, WorldId,
};
use std::sync::Arc;
use tempfile::tempdir;

fn world(value: &str, epoch: u64) -> World {
    World::new("minecraft", value, epoch, Vec::new())
}
fn coordinate(x: i32, z: i32) -> ChunkCoordinate {
    ChunkCoordinate { x, z }
}
async fn repository() -> (tempfile::TempDir, Arc<Repository>) {
    let dir = tempdir().unwrap();
    let repository = Arc::new(Repository::open(dir.path().join("state.sqlite")).await.unwrap());
    (dir, repository)
}

const BRIDGE_A: [u8; 16] = [1; 16];
const BRIDGE_B: [u8; 16] = [2; 16];

#[tokio::test]
async fn dirty_rows_are_assigned_to_an_owner_and_page_is_owner_scoped() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(2, 2), 1, &BRIDGE_B, &[2; 16], 1)
        .await
        .unwrap();
    let page = repository.dirty_page_for_owner(&BRIDGE_A, 10, 0).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].coordinate, coordinate(1, 1));
    assert_eq!(page[0].owner_bridge_id, BRIDGE_A.to_vec());
    let other = repository.dirty_page_for_owner(&BRIDGE_B, 10, 0).await.unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].coordinate, coordinate(2, 2));
}

#[tokio::test]
async fn dirty_page_honors_page_size_and_orders_rows() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    for x in 0..5 {
        repository
            .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(x, 0), x as u64 + 1, &BRIDGE_A, &[1; 16], x as u64 + 1)
            .await
            .unwrap();
    }
    let page = repository.dirty_page_for_owner(&BRIDGE_A, 2, 0).await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].coordinate, coordinate(4, 0));
    assert_eq!(page[1].coordinate, coordinate(3, 0));
    assert!(page[0].revision > page[1].revision);
}

#[tokio::test]
async fn assigned_rows_have_lease_expiry_and_expired_rows_are_reclaimed() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    let assigned = repository.assign_dirty_lease_at(&BRIDGE_A, 10, 100, 0).await.unwrap().unwrap();
    assert_eq!(assigned.coordinate, coordinate(1, 1));
    assert_eq!(assigned.owner_session_id, vec![1; 16]);
    // A second bridge must not receive the leased row before expiry.
    let other = repository.assign_dirty_lease_at(&BRIDGE_B, 10, 0, 0).await.unwrap();
    assert!(other.is_none());
    // After expiry the row is reclaimed and assignable.
    let reclaimed = repository.assign_dirty_lease_at(&BRIDGE_B, 10, 101, 101).await.unwrap();
    assert_eq!(reclaimed.unwrap().coordinate, coordinate(1, 1));
}

#[tokio::test]
async fn complete_is_owner_scoped_and_does_not_remove_other_owners_rows() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(2, 2), 1, &BRIDGE_B, &[2; 16], 1)
        .await
        .unwrap();
    assert!(!repository.complete_dirty_for_owner(&WorldId::new("minecraft", "overworld", 1), coordinate(2, 2), 1, &BRIDGE_A).await.unwrap());
    assert!(repository.complete_dirty_for_owner(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A).await.unwrap());
    let page = repository.dirty_page_for_owner(&BRIDGE_B, 10, 0).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].coordinate, coordinate(2, 2));
}

#[tokio::test]
async fn reassignment_on_reconnect_moves_rows_to_new_session_and_marks_replay() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(2, 2), 2, &BRIDGE_A, &[1; 16], 2)
        .await
        .unwrap();
    // The bridge reconnects with a fresh authenticated session and regains ownership.
    repository.reassign_bridge_lease(&BRIDGE_A, &[3; 16]).await.unwrap();
    let replay_page = repository
        .dirty_replay_page(&BRIDGE_A, &[3; 16], &WorldId::new("minecraft", "overworld", 1), None, 10)
        .await
        .unwrap();
    assert_eq!(replay_page.len(), 2);
    // Render page does not see replay-pending rows.
    let page = repository.dirty_page_for_owner(&BRIDGE_A, 10, 0).await.unwrap();
    assert!(page.is_empty());
    let checkpoint = repository.bridge_checkpoint(&BRIDGE_A).await.unwrap();
    assert!(checkpoint.is_some());
    assert_eq!(checkpoint.unwrap().session_id, vec![3; 16]);
}


#[tokio::test]
async fn complete_clears_replay_flag_and_retry_rows() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    repository.reassign_bridge_lease(&BRIDGE_A, &[3; 16]).await.unwrap();
    let replay_page = repository
        .dirty_replay_page(&BRIDGE_A, &[3; 16], &WorldId::new("minecraft", "overworld", 1), None, 10)
        .await
        .unwrap();
    assert_eq!(replay_page.len(), 1);
    repository.defer_dirty_for_owner(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, 0).await.unwrap();
    repository.complete_dirty_for_owner(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A).await.unwrap();
    assert!(repository
        .dirty_replay_page(&BRIDGE_A, &[3; 16], &WorldId::new("minecraft", "overworld", 1), None, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(repository.dirty_rows_for_replay(&BRIDGE_A).await.unwrap().is_empty());
    let page = repository.dirty_page_for_owner(&BRIDGE_A, 10, i64::MAX).await.unwrap();
    assert!(page.is_empty());
}

#[tokio::test]
async fn stale_epoch_rows_are_never_assigned_or_replayed() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    let assigned = repository.assign_dirty_lease_at(&BRIDGE_A, 10, 0, 0).await.unwrap();
    assert!(assigned.unwrap().revision == 1);
    // Epoch replacement must purge stale work before any assignment.
    repository.apply_world(world("overworld", 2)).await.unwrap();
    let none = repository.assign_dirty_lease_at(&BRIDGE_B, 10, 0, 0).await.unwrap();
    assert!(none.is_none());
    assert!(repository.dirty_rows_for_replay(&BRIDGE_B).await.unwrap().is_empty());
}

#[tokio::test]
async fn active_lease_rows_do_not_require_duplicate_replay() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    let assigned: DirtyLease = repository.assign_dirty_lease_at(&BRIDGE_A, 10, 0, 0).await.unwrap().unwrap();
    assert_eq!(assigned.owner_session_id, vec![1; 16]);
    let rows = repository
        .dirty_replay_page(&BRIDGE_A, &[1; 16], &WorldId::new("minecraft", "overworld", 1), None, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let leaked: Option<DirtyLease> = repository.assign_dirty_lease_at(&BRIDGE_B, 10, 0, 0).await.unwrap();
    assert!(leaked.is_none());
    let _ = assigned;
}

#[tokio::test]
async fn dirty_replay_page_pagination_resists_mid_replay_mutation() {
    let (_dir, repository) = repository().await;
    repository.apply_world(world("overworld", 1)).await.unwrap();
    for x in 0..5 {
        repository
            .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(x, 0), x as u64 + 1, &BRIDGE_A, &[1; 16], x as u64 + 1)
            .await
            .unwrap();
    }
    repository.reassign_bridge_lease(&BRIDGE_A, &[2; 16]).await.unwrap();
    let first = repository
        .dirty_replay_page(&BRIDGE_A, &[2; 16], &WorldId::new("minecraft", "overworld", 1), None, 2)
        .await
        .unwrap();
    assert_eq!(first.len(), 3); // 2 + sentinel for has_more
    let last = first[1].coordinate;
    // A live republish of (2,0) removes it from the replay set.
    repository
        .mark_dirty(&WorldId::new("minecraft", "overworld", 1), coordinate(2, 0), 10, &BRIDGE_A, &[2; 16], 100)
        .await
        .unwrap();
    let second = repository
        .dirty_replay_page(&BRIDGE_A, &[2; 16], &WorldId::new("minecraft", "overworld", 1), Some(&last), 2)
        .await
        .unwrap();
    assert_eq!(second.len(), 2); // (3,0) and (4,0), no sentinel
    assert_eq!(second[0].coordinate, coordinate(3, 0));
    assert_eq!(second[1].coordinate, coordinate(4, 0));
}
