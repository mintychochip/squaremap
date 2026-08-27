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

/// A view-distance ring is dirtied after the player's already-loaded interior.
/// Newest-revision order would paint only that ring and leave the player on
/// empty ocean. The live page has to prefer the neighborhood around recent
/// activity so walking fills a solid patch, not 16-block specks.
#[tokio::test]
async fn dirty_page_for_owner_prefers_player_neighborhood_over_view_ring() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let world_id = overworld.id();
    let player = coordinate(0, 0);
    repository
        .mark_dirty(&world_id, player, 100, &BRIDGE_A, &[1; 16], 100)
        .await
        .unwrap();
    let mut revision = 90;
    for (dx, dz) in [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ] {
        repository
            .mark_dirty(
                &world_id,
                coordinate(dx, dz),
                revision,
                &BRIDGE_A,
                &[1; 16],
                revision,
            )
            .await
            .unwrap();
        revision += 1;
    }
    let mut ring_revision = 200;
    for x in -12_i32..=12 {
        for z in -12_i32..=12 {
            if x.abs() != 12 && z.abs() != 12 {
                continue;
            }
            repository
                .mark_dirty(
                    &world_id,
                    coordinate(x, z),
                    ring_revision,
                    &BRIDGE_A,
                    &[1; 16],
                    ring_revision,
                )
                .await
                .unwrap();
            ring_revision += 1;
        }
    }
    let page = repository
        .dirty_page_for_owner(&BRIDGE_A, 32, 0)
        .await
        .unwrap();
    assert_eq!(page.len(), 32);
    let selected: Vec<_> = page.iter().map(|row| row.coordinate).collect();
    assert!(
        selected.contains(&player),
        "player cell must be in the live page, got {selected:?}"
    );
    let interior = selected
        .iter()
        .filter(|coord| coord.x.abs() <= 1 && coord.z.abs() <= 1)
        .count();
    assert!(
        interior >= 9,
        "live page must fill the player's 3x3, interior={interior}, selected={selected:?}"
    );
}

/// Newest-revision activity can be a populate/spawn cluster tens of chunks
/// away. When a player is online the live page must follow that player, not
/// the globally newest centroid.
#[tokio::test]
async fn dirty_page_for_owner_with_player_focus_ignores_distant_newest_cluster() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    let world_id = overworld.id();
    let player = coordinate(1, -4);
    repository
        .mark_dirty(&world_id, player, 10, &BRIDGE_A, &[1; 16], 10)
        .await
        .unwrap();
    for (dx, dz) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
        repository
            .mark_dirty(
                &world_id,
                coordinate(player.x + dx, player.z + dz),
                11,
                &BRIDGE_A,
                &[1; 16],
                11,
            )
            .await
            .unwrap();
    }
    for i in 0..40 {
        repository
            .mark_dirty(
                &world_id,
                coordinate(8 + (i % 8), 34 + (i / 8)),
                1000 + i as u64,
                &BRIDGE_A,
                &[1; 16],
                1000 + i as u64,
            )
            .await
            .unwrap();
    }
    let unfocused = repository
        .dirty_page_for_owner(&BRIDGE_A, 32, 0)
        .await
        .unwrap();
    assert!(
        !unfocused.iter().any(|row| row.coordinate == player),
        "without a player focus the distant newest cluster must win"
    );
    let focused = repository
        .dirty_page_for_owner_near(&BRIDGE_A, 32, 0, Some((player.x, player.z)))
        .await
        .unwrap();
    let selected: Vec<_> = focused.iter().map(|row| row.coordinate).collect();
    assert!(
        selected.contains(&player),
        "player cell must be selected when focused, got {selected:?}"
    );
    let nearest: Vec<_> = selected.iter().take(5).copied().collect();
    assert!(
        nearest.iter().all(|coord| {
            (coord.x - player.x).abs() <= 1 && (coord.z - player.z).abs() <= 1
        }),
        "the first live slots must be the player's neighborhood, got {nearest:?}"
    );
}

/// Live fly-in ticks must not keep selecting newest chunks that just failed a
/// snapshot. After deferral those rows wait on retry backoff so the player's
/// already-loaded cell can enter the next page.
#[tokio::test]
async fn dirty_page_for_owner_skips_rows_waiting_on_retry_backoff() {
    let (_dir, repository) = repository().await;
    let overworld = world("overworld", 1);
    repository.apply_world(overworld.clone()).await.unwrap();
    repository
        .mark_dirty(&overworld.id(), coordinate(1, 1), 1, &BRIDGE_A, &[1; 16], 1)
        .await
        .unwrap();
    repository
        .mark_dirty(&overworld.id(), coordinate(9, 9), 2, &BRIDGE_A, &[1; 16], 2)
        .await
        .unwrap();
    repository
        .defer_dirty_for_owner(&overworld.id(), coordinate(9, 9), 2, &BRIDGE_A, 100)
        .await
        .unwrap();
    let page = repository.dirty_page_for_owner(&BRIDGE_A, 10, 100).await.unwrap();
    assert_eq!(page.len(), 1, "deferred newest chunk must not occupy the live page");
    assert_eq!(page[0].coordinate, coordinate(1, 1));
    let later = repository.dirty_page_for_owner(&BRIDGE_A, 10, 102).await.unwrap();
    assert!(
        later.iter().any(|row| row.coordinate == coordinate(9, 9)),
        "newest chunk must return after backoff, got {:?}",
        later.iter().map(|row| row.coordinate).collect::<Vec<_>>()
    );
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
    // Live render still sees unleased replay-pending rows after reconnect.
    let page = repository.dirty_page_for_owner(&BRIDGE_A, 10, 0).await.unwrap();
    assert_eq!(page.len(), 2);
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
