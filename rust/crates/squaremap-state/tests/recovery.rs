use squaremap_state::{
    ChunkCoordinate, DirtyChunk, JobKind, JobState, RenderJob, Repository, World, WorldId,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn world(epoch: u64) -> World {
    World::new("minecraft", "overworld", epoch, br"{}".to_vec())
}

fn chunk(x: i32, z: i32) -> ChunkCoordinate {
    ChunkCoordinate { x, z }
}

fn dirty_rows(recovery: &squaremap_state::Recovery) -> Vec<(u64, i32, i32)> {
    recovery
        .dirty
        .iter()
        .map(|row| (row.revision, row.coordinate.x, row.coordinate.z))
        .collect()
}

#[tokio::test]
async fn schema_pragmas_reopen_revision_guards_checkpoint_and_deterministic_recovery() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("state.sqlite");
    let repository = Repository::open(&db).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    repository
        .mark_dirty(&world.id(), chunk(4, 8), 7, &[1_u8; 16], 7)
        .await
        .unwrap();
    repository
        .mark_dirty(&world.id(), chunk(4, 8), 7, &[1_u8; 16], 7)
        .await
        .unwrap();
    repository
        .mark_dirty(&world.id(), chunk(4, 8), 6, &[1_u8; 16], 8)
        .await
        .unwrap();
    let checkpoint = repository.recover().await.unwrap().checkpoints;
    assert_eq!(checkpoint[0].durable_sequence, 8);
    assert_eq!(dirty_rows(&repository.recover().await.unwrap()), vec![(7, 4, 8)]);

    repository
        .mark_dirty(&world.id(), chunk(4, 8), 8, &[1_u8; 16], 9)
        .await
        .unwrap();
    let updated = repository.recover().await.unwrap();
    assert_eq!(dirty_rows(&updated), vec![(8, 4, 8)]);
    repository
        .complete_dirty(&world.id(), chunk(4, 8), 7)
        .await
        .unwrap();
    assert_eq!(dirty_rows(&repository.recover().await.unwrap()), vec![(8, 4, 8)]);
    repository
        .complete_dirty(&world.id(), chunk(4, 8), 8)
        .await
        .unwrap();
    assert!(repository.recover().await.unwrap().dirty.is_empty());

    let mut job = RenderJob::new(world.id(), JobKind::Full, b"payload".to_vec());
    job.state = JobState::Running;
    repository.create_render_job(job.clone()).await.unwrap();
    let reopened = Repository::open(&db).await.unwrap();
    let recovered = reopened.recover().await.unwrap();
    assert_eq!(recovered.worlds, vec![world.clone()]);
    assert_eq!(recovered.jobs.len(), 1);
    job.state = JobState::Running;
    job.completed_chunks = 3;
    reopened.update_render_job(job).await.unwrap();
    let updated_job = reopened.recover().await.unwrap();
    assert_eq!(updated_job.jobs[0].state, JobState::Resumable);
    assert_eq!(updated_job.jobs[0].completed_chunks, 3);
    assert_eq!(recovered.jobs[0].state, JobState::Resumable);
    assert_eq!(recovered.jobs[0].payload, b"payload");
    assert_eq!(updated_job, reopened.recover().await.unwrap());

    let (journal, synchronous, foreign_keys, busy_timeout, application_id) = reopened.pragma_values().await.unwrap();
    assert_eq!(journal, "wal");
    assert_eq!(synchronous, 1);
    assert_eq!(foreign_keys, 1);
    assert_eq!(busy_timeout, 5000);
    assert_eq!(application_id, 0x53514d50);
    let sqlite = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(sqlite.query_row("SELECT version FROM schema_version", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
}

#[tokio::test]
async fn epoch_guards_purge_old_work_and_stale_operations_cannot_resurrect() {
    let dir = tempdir().unwrap();
    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let first = world(1);
    repository.apply_world(first.clone()).await.unwrap();
    repository.mark_dirty(&first.id(), chunk(1, 1), 3, &[2_u8; 16], 1).await.unwrap();
    let old_job = RenderJob::new(first.id(), JobKind::Full, vec![1]);
    repository.create_render_job(old_job).await.unwrap();
    let second = world(2);
    repository.apply_world(second.clone()).await.unwrap();
    assert!(repository.remove_world(&first.id()).await.unwrap() == false);
    repository.mark_dirty(&first.id(), chunk(2, 2), 4, &[2_u8; 16], 2).await.unwrap_err();
    repository.complete_dirty(&first.id(), chunk(1, 1), 3).await.unwrap_err();
    let recovered = repository.recover().await.unwrap();
    assert_eq!(recovered.worlds, vec![second]);
    assert!(recovered.dirty.is_empty());
    assert!(recovered.jobs.is_empty());
}


#[tokio::test]
async fn remove_keeps_epoch_tombstone_against_delayed_apply_and_reactivation() {
    let dir = tempdir().unwrap();
    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let epoch_two = World::new("minecraft", "overworld", 2, br"{}".to_vec());
    repository.apply_world(epoch_two.clone()).await.unwrap();
    assert!(repository.remove_world(&epoch_two.id()).await.unwrap());
    assert!(repository.apply_world(world(1)).await.is_err());
    assert!(repository.apply_world(epoch_two.clone()).await.is_err());
    assert!(repository.mark_dirty(&epoch_two.id(), chunk(1, 1), 1, &[3_u8; 16], 1).await.is_err());
    let epoch_three = World::new("minecraft", "overworld", 3, br"{}".to_vec());
    repository.apply_world(epoch_three.clone()).await.unwrap();
    assert!(repository.mark_dirty(&epoch_two.id(), chunk(1, 1), 1, &[3_u8; 16], 1).await.is_err());
    let max_epoch = World::new("minecraft", "max", i64::MAX as u64, br"{}".to_vec());
    repository.apply_world(max_epoch.clone()).await.unwrap();
    assert!(repository.remove_world(&max_epoch.id()).await.unwrap());
    assert!(repository.apply_world(max_epoch.clone()).await.is_err());
}

#[tokio::test]
async fn malformed_schema_is_rejected_before_wal_or_file_mutation() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("malformed.sqlite");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch(
        "PRAGMA application_id=0x53514D50;
         CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
         INSERT INTO schema_version VALUES(1);
         CREATE TABLE worlds(namespace TEXT NOT NULL,value TEXT NOT NULL,epoch INTEGER NOT NULL,PRIMARY KEY(namespace,value));
         CREATE TABLE dirty_chunks(namespace TEXT);
         CREATE TABLE render_jobs(id BLOB);
         CREATE TABLE session_checkpoints(session_id BLOB);
         CREATE TABLE legacy_imports(relative_path TEXT);",
    ).unwrap();
    drop(connection);
    let before = fs::read(&db).unwrap();
    assert!(Repository::open(&db).await.is_err());
    assert_eq!(fs::read(&db).unwrap(), before);
    assert!(!db.with_extension("sqlite-wal").exists());
    assert!(!db.with_extension("sqlite-shm").exists());
}

#[tokio::test]
async fn duplicate_resume_coordinates_and_symlinks_are_rejected_with_path() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("legacy");
    fs::create_dir_all(&files).unwrap();
    let db = dir.path().join("state.sqlite");
    let repository = Repository::open(&db).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    let resume = files.join("resume_render.json");
    fs::write(&resume, br#"[[{"x":1,"z":2},true],[{"x":1,"z":2},false]]"#).unwrap();
    let error = repository.import_legacy_state(&world.id(), &files).await.unwrap_err();
    assert!(error.to_string().contains("resume_render.json"));
    let outside = dir.path().join("outside.json");
    fs::write(&outside, br#"[{"x":1,"z":2}]"#).unwrap();
    fs::remove_file(&files.join("dirty_chunks.json")).ok();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, files.join("dirty_chunks.json")).unwrap();
    #[cfg(unix)]
    {
        let error = repository.import_legacy_state(&world.id(), &files).await.unwrap_err();
        assert!(error.to_string().contains("dirty_chunks.json"));
    }
}

#[tokio::test]
async fn legacy_markers_include_epoch_and_unambiguous_world_identity() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("world");
    fs::create_dir_all(&files).unwrap();
    fs::write(files.join("dirty_chunks.json"), br#"[{"x":7,"z":9}]"#).unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":1,"z":2},true]]"#).unwrap();
    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let epoch_one = World::new("a/b", "c", 1, Vec::new());
    repository.apply_world(epoch_one.clone()).await.unwrap();
    repository.import_legacy_state(&epoch_one.id(), &files).await.unwrap();
    repository.remove_world(&epoch_one.id()).await.unwrap();
    let epoch_two = World::new("a/b", "c", 2, Vec::new());
    repository.apply_world(epoch_two.clone()).await.unwrap();
    repository.import_legacy_state(&epoch_two.id(), &files).await.unwrap();
    assert_eq!(repository.recover().await.unwrap().dirty.len(), 1);
    let colliding = World::new("a", "b/c", 1, Vec::new());
    repository.apply_world(colliding.clone()).await.unwrap();
    repository.import_legacy_state(&colliding.id(), &files).await.unwrap();
    let recovered = repository.recover().await.unwrap();
    assert_eq!(recovered.dirty.iter().filter(|row| row.world == colliding.id()).count(), 1);
}
#[tokio::test]
async fn render_job_update_and_legacy_import_are_atomic_strict_idempotent_and_read_only() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("state.sqlite");
    let files = dir.path().join("world");
    fs::create_dir_all(&files).unwrap();
    let dirty_path = files.join("dirty_chunks.json");
    let resume_path = files.join("resume_render.json");
    fs::write(&dirty_path, br#"[{"x":4,"z":8},{"x":4,"z":8},{"x":-2,"z":3}]"#).unwrap();
    fs::write(&resume_path, br#"[[{"x":1,"z":2},true],[{"x":3,"z":4},false]]"#).unwrap();
    let dirty_bytes = fs::read(&dirty_path).unwrap();
    let resume_bytes = fs::read(&resume_path).unwrap();
    let repository = Repository::open(&db).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    let imported = repository.recover().await.unwrap();
    assert_eq!(imported.dirty.len(), 2);
    assert_eq!(imported.jobs.len(), 1);
    assert_eq!(imported.jobs[0].state, JobState::Resumable);
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    assert_eq!(repository.recover().await.unwrap(), imported);
    assert_eq!(fs::read(&dirty_path).unwrap(), dirty_bytes);
    assert_eq!(fs::read(&resume_path).unwrap(), resume_bytes);
    let mut live_job = imported.jobs[0].clone();
    live_job.state = JobState::Running;
    live_job.payload = b"live-payload".to_vec();
    live_job.completed_chunks = 11;
    repository.update_render_job(live_job.clone()).await.unwrap();

    fs::write(&resume_path, br#"[[{"x":9,"z":9},true]]"#).unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    let after_live_import = repository.recover().await.unwrap();
    assert_eq!(after_live_import.jobs[0].payload, b"live-payload");
    assert_eq!(after_live_import.jobs[0].completed_chunks, 11);
    assert_eq!(after_live_import.jobs[0].state, JobState::Resumable);

    fs::write(&dirty_path, br#"[{"x":99,"z":99}]"#).unwrap();
    fs::write(&resume_path, br#"not-json"#).unwrap();
    let error = repository.import_legacy_state(&world.id(), &files).await.unwrap_err();
    assert!(error.to_string().contains("resume_render.json"));
    assert!(!repository.recover().await.unwrap().dirty.iter().any(|row| row.coordinate == chunk(99, 99)));
    fs::write(&resume_path, br#"[[{"x":1,"z":2},true]]"#).unwrap();
    let newer = World::new("minecraft", "overworld", 2, br"{}".to_vec());
    repository.apply_world(newer.clone()).await.unwrap();
    assert!(repository.import_legacy_state(&world.id(), &files).await.is_err());
    assert!(repository.recover().await.unwrap().dirty.is_empty());
    assert!(repository.recover().await.unwrap().jobs.is_empty());
}

#[tokio::test]
async fn legacy_import_preserves_unowned_zero_progress_resume_jobs() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("world");
    fs::create_dir_all(&files).unwrap();
    fs::write(files.join("dirty_chunks.json"), br#"[]"#).unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":1,"z":2},true]]"#).unwrap();
    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    let mut live = RenderJob::new(world.id(), JobKind::Resume, b"live-zero".to_vec());
    live.state = JobState::Resumable;
    repository.create_render_job(live.clone()).await.unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":9,"z":9},false]]"#).unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    let recovered = repository.recover().await.unwrap();
    assert_eq!(recovered.jobs[0].payload, b"live-zero");
    assert_eq!(recovered.jobs[0].completed_chunks, 0);
}

#[tokio::test]
async fn unrelated_full_job_update_does_not_clear_resume_ownership() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("world");
    fs::create_dir_all(&files).unwrap();
    fs::write(files.join("dirty_chunks.json"), br#"[]"#).unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":1,"z":2},true]]"#).unwrap();
    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    let mut full = RenderJob::new(world.id(), JobKind::Full, b"full".to_vec());
    full.state = JobState::Running;
    repository.create_render_job(full.clone()).await.unwrap();
    full.payload = b"full-live".to_vec();
    repository.update_render_job(full).await.unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":9,"z":9},false]]"#).unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    let recovered = repository.recover().await.unwrap();
    let resume = recovered.jobs.iter().find(|job| job.kind == JobKind::Resume).unwrap();
    assert_eq!(resume.payload, br#"[[{"x":9,"z":9},false]]"#);
}

#[tokio::test]
async fn render_job_updates_cannot_resurrect_terminal_state_or_reduce_progress() {
    let dir = tempdir().unwrap();
    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    let mut job = RenderJob::new(world.id(), JobKind::Full, b"job".to_vec());
    job.state = JobState::Running;
    job.completed_chunks = 5;
    repository.create_render_job(job.clone()).await.unwrap();
    let mut reduced = job.clone();
    reduced.completed_chunks = 3;
    assert!(repository.update_render_job(reduced).await.is_err());
    let mut terminal = job.clone();
    terminal.state = JobState::Completed;
    repository.update_render_job(terminal.clone()).await.unwrap();
    let mut resurrected = terminal.clone();
    resurrected.state = JobState::Running;
    assert!(repository.update_render_job(resurrected).await.is_err());
}

#[tokio::test]
async fn oversized_legacy_marker_hash_is_rejected_without_mutation() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("world");
    let db = dir.path().join("state.sqlite");
    fs::create_dir_all(&files).unwrap();
    fs::write(files.join("dirty_chunks.json"), br#"[]"#).unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":3,"z":4},true]]"#).unwrap();
    let repository = Repository::open(&db).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    drop(repository);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute("UPDATE legacy_imports SET content_sha256=zeroblob(?1)", [17 * 1024 * 1024_i64]).unwrap();
    drop(connection);
    let repository = Repository::open(&db).await.unwrap();
    let before = repository.recover().await.unwrap();
    assert!(repository.import_legacy_state(&world.id(), &files).await.is_err());
    assert_eq!(repository.recover().await.unwrap(), before);
}

#[tokio::test]
async fn malformed_legacy_marker_hash_is_rejected_without_mutation() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("world");
    let db = dir.path().join("state.sqlite");
    fs::create_dir_all(&files).unwrap();
    fs::write(files.join("dirty_chunks.json"), br#"[{"x":1,"z":2}]"#).unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":3,"z":4},true]]"#).unwrap();
    let repository = Repository::open(&db).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    drop(repository);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute("UPDATE legacy_imports SET content_sha256=?1", [vec![1_u8]]).unwrap();
    drop(connection);
    let repository = Repository::open(&db).await.unwrap();
    let before = repository.recover().await.unwrap();
    assert!(repository.import_legacy_state(&world.id(), &files).await.is_err());
    assert_eq!(repository.recover().await.unwrap(), before);
}

#[tokio::test]
async fn conflicting_job_ids_and_malformed_recovered_blobs_are_rejected() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("state.sqlite");
    let repository = Repository::open(&db).await.unwrap();
    let first_world = world(1);
    repository.apply_world(first_world.clone()).await.unwrap();
    let first = RenderJob::new(first_world.id(), JobKind::Full, b"first".to_vec());
    repository.create_render_job(first.clone()).await.unwrap();
    let mut conflicting = first.clone();
    conflicting.payload = b"conflict".to_vec();
    assert!(repository.create_render_job(conflicting).await.is_err());
    drop(repository);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute("UPDATE render_jobs SET state=1,payload=?1", [vec![0_u8; 17 * 1024 * 1024]]).unwrap();
    drop(connection);
    let repository = Repository::open(&db).await.unwrap();
    assert!(repository.recover().await.is_err());
}

#[tokio::test]
async fn unsupported_schema_versions_and_strict_legacy_bounds_are_rejected() {
    let dir = tempdir().unwrap();
    let newer_db = dir.path().join("newer.sqlite");
    let newer_repository = Repository::open(&newer_db).await.unwrap();
    drop(newer_repository);
    let connection = rusqlite::Connection::open(&newer_db).unwrap();
    connection.execute("UPDATE schema_version SET version=2", []).unwrap();
    drop(connection);
    assert!(Repository::open(&newer_db).await.is_err());

    let multiple_db = dir.path().join("multiple.sqlite");
    let multiple_repository = Repository::open(&multiple_db).await.unwrap();
    drop(multiple_repository);
    let connection = rusqlite::Connection::open(&multiple_db).unwrap();
    connection.execute_batch("DROP TABLE schema_version; CREATE TABLE schema_version(version INTEGER); INSERT INTO schema_version VALUES(1),(1);").unwrap();
    drop(connection);

    assert!(Repository::open(&multiple_db).await.is_err());

    let files = dir.path().join("legacy");
    fs::create_dir_all(&files).unwrap();
    let repository = Repository::open(dir.path().join("legacy.sqlite")).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    let dirty_path = files.join("dirty_chunks.json");
    fs::write(&dirty_path, br#"{}"#).unwrap();
    let error = repository.import_legacy_state(&world.id(), &files).await.unwrap_err();
    assert!(error.to_string().contains("dirty_chunks.json"));
    let oversized = vec![b' '; 33 * 1024 * 1024];
    fs::write(&dirty_path, oversized).unwrap();
    let error = repository.import_legacy_state(&world.id(), &files).await.unwrap_err();
    assert!(error.to_string().contains("dirty_chunks.json"));
    let entries = (0..200_001).map(|index| format!(r#"{{"x":{index},"z":0}}"#)).collect::<Vec<_>>().join(",");
    fs::write(&dirty_path, format!("[{entries}]")).unwrap();
    let error = repository.import_legacy_state(&world.id(), &files).await.unwrap_err();
    assert!(error.to_string().contains("dirty_chunks.json"));
}
#[tokio::test]
async fn deterministic_resume_id_identity_conflict_rolls_back_import() {
    let dir = tempdir().unwrap();
    let files = dir.path().join("world");
    let db = dir.path().join("state.sqlite");
    fs::create_dir_all(&files).unwrap();
    fs::write(files.join("dirty_chunks.json"), br#"[]"#).unwrap();
    fs::write(files.join("resume_render.json"), br#"[[{"x":1,"z":2},true]]"#).unwrap();
    let repository = Repository::open(&db).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    drop(repository);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute("UPDATE render_jobs SET namespace='other',value='world',kind=1 WHERE id=?1", [squaremap_state::RenderJob::new(world.id(), JobKind::Resume, Vec::new()).id]).unwrap();
    let before = connection.query_row("SELECT namespace,value,epoch,kind,payload FROM render_jobs", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, Vec<u8>>(4)?))).unwrap();
    drop(connection);
    fs::write(files.join("resume_render.json"), br#"[[{"x":9,"z":9},false]]"#).unwrap();
    let repository = Repository::open(&db).await.unwrap();
    assert!(repository.import_legacy_state(&world.id(), &files).await.is_err());
    drop(repository);
    let connection = rusqlite::Connection::open(&db).unwrap();
    let after = connection.query_row("SELECT namespace,value,epoch,kind,payload FROM render_jobs", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, Vec<u8>>(4)?))).unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn oversized_schema_valid_stored_values_fail_recovery_without_returning_blobs() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("state.sqlite");
    let repository = Repository::open(&db).await.unwrap();
    repository.apply_world(world(1)).await.unwrap();
    drop(repository);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute("UPDATE worlds SET config=zeroblob(?1)", [17 * 1024 * 1024_i64]).unwrap();
    drop(connection);
    let repository = Repository::open(&db).await.unwrap();
    assert!(repository.recover().await.is_err());
}

#[tokio::test]
async fn malformed_databases_and_u64_overflow_fail_without_mutation() {
    let dir = tempdir().unwrap();
    let wrong = dir.path().join("wrong.sqlite");
    let connection = rusqlite::Connection::open(&wrong).unwrap();
    connection.execute_batch("CREATE TABLE unrelated(value TEXT);").unwrap();
    drop(connection);
    assert!(Repository::open(&wrong).await.is_err());

    let repository = Repository::open(dir.path().join("state.sqlite")).await.unwrap();
    let world = world(1);
    repository.apply_world(world.clone()).await.unwrap();
    assert!(repository.mark_dirty(&world.id(), chunk(0, 0), u64::MAX, &[4_u8; 16], 1).await.is_err());
    assert!(repository.recover().await.unwrap().dirty.is_empty());
    assert!(repository.mark_dirty(&world.id(), chunk(0, 0), 1, &[4_u8; 16], u64::MAX).await.is_err());
    assert!(repository.recover().await.unwrap().checkpoints.is_empty());
}

#[allow(dead_code)]
fn _paths_are_relative(_path: &PathBuf) {
    let _ = Path::new("dirty_chunks.json");
}
