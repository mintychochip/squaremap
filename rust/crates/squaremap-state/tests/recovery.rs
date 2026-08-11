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
        .mark_dirty(&world.id(), chunk(4, 8), 7, b"session-b", 7)
        .await
        .unwrap();
    repository
        .mark_dirty(&world.id(), chunk(4, 8), 7, b"session-b", 7)
        .await
        .unwrap();
    repository
        .mark_dirty(&world.id(), chunk(4, 8), 6, b"session-b", 8)
        .await
        .unwrap();
    let checkpoint = repository.recover().await.unwrap().checkpoints;
    assert_eq!(checkpoint[0].durable_sequence, 8);
    assert_eq!(dirty_rows(&repository.recover().await.unwrap()), vec![(7, 4, 8)]);

    repository
        .mark_dirty(&world.id(), chunk(4, 8), 8, b"session-b", 9)
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
    repository.mark_dirty(&first.id(), chunk(1, 1), 3, b"session", 1).await.unwrap();
    let old_job = RenderJob::new(first.id(), JobKind::Full, vec![1]);
    repository.create_render_job(old_job).await.unwrap();
    let second = world(2);
    repository.apply_world(second.clone()).await.unwrap();
    assert!(repository.remove_world(&first.id()).await.unwrap() == false);
    repository.mark_dirty(&first.id(), chunk(2, 2), 4, b"session", 2).await.unwrap_err();
    repository.complete_dirty(&first.id(), chunk(1, 1), 3).await.unwrap_err();
    let recovered = repository.recover().await.unwrap();
    assert_eq!(recovered.worlds, vec![second]);
    assert!(recovered.dirty.is_empty());
    assert!(recovered.jobs.is_empty());
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
    fs::write(&resume_path, br#"[[{"x":1,"z":2},true],[{"x":1,"z":2},false],[{"x":3,"z":4},true]]"#).unwrap();
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

    fs::write(&resume_path, br#"[[{"x":9,"z":9},true]]"#).unwrap();
    repository.import_legacy_state(&world.id(), &files).await.unwrap();
    assert_eq!(repository.recover().await.unwrap().jobs.len(), 1);

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
    assert!(repository.mark_dirty(&world.id(), chunk(0, 0), u64::MAX, b"s", 1).await.is_err());
    assert!(repository.recover().await.unwrap().dirty.is_empty());
    assert!(repository.mark_dirty(&world.id(), chunk(0, 0), 1, b"s", u64::MAX).await.is_err());
    assert!(repository.recover().await.unwrap().checkpoints.is_empty());
}

#[allow(dead_code)]
fn _paths_are_relative(_path: &PathBuf) {
    let _ = Path::new("dirty_chunks.json");
}
