use super::{RenderDisposition, RunReport, Scheduler, SchedulerError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use squaremap_state::{ChunkCoordinate, JobKind, JobState, RenderJob, WorldId};
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JobCursor {
    coordinates: Vec<ChunkCoordinate>,
    next: usize,
    #[serde(default)]
    revision: u64,
}
#[derive(Clone, Debug, Deserialize)]
struct LegacyResumeEntry(ChunkCoordinate, bool);

fn decode_cursor(job: &RenderJob) -> Result<JobCursor, SchedulerError> {
    match serde_json::from_slice(&job.payload) {
        Ok(cursor) => Ok(cursor),
        Err(error) if job.kind == JobKind::Resume => {
            let entries: Vec<LegacyResumeEntry> = serde_json::from_slice(&job.payload)
                .map_err(|_| SchedulerError::Serialization(error))?;
            Ok(JobCursor {
                coordinates: entries
                    .into_iter()
                    .filter_map(|LegacyResumeEntry(coordinate, completed)| (!completed).then_some(coordinate))
                    .collect(),
                next: 0,
                revision: 0,
            })
        }
        Err(error) => Err(SchedulerError::Serialization(error)),
    }
}

pub(super) async fn start(
    scheduler: &Scheduler,
    world: WorldId,
    kind: JobKind,
    coordinates: Vec<ChunkCoordinate>,
    revision: u64,
) -> Result<RenderJob, SchedulerError> {
    if !matches!(kind, JobKind::Full | JobKind::Radius) {
        return Err(SchedulerError::InvalidJob("scheduler can only start full or radius jobs"));
    }
    if coordinates.is_empty() {
        return Err(SchedulerError::InvalidJob("render job requires at least one coordinate"));
    }
    let nonce = scheduler.next_job_nonce.fetch_add(1, Ordering::Relaxed);
    let mut id = Sha256::new();
    id.update(world.namespace.as_bytes());
    id.update([0]);
    id.update(world.value.as_bytes());
    id.update([0]);
    id.update(world.epoch.to_le_bytes());
    id.update([kind as u8]);
    id.update(nonce.to_le_bytes());
    let cursor = JobCursor { coordinates, next: 0, revision };
    let payload = serde_json::to_vec(&cursor)?;
    let job = RenderJob {
        id: id.finalize().to_vec(),
        world,
        kind,
        state: JobState::Running,
        payload,
        completed_chunks: 0,
    };
    scheduler.repository.create_render_job(job.clone()).await?;
    Ok(job)
}

pub(super) async fn run(
    scheduler: &Scheduler,
    id: &[u8],
    max_steps: usize,
) -> Result<RunReport, SchedulerError> {
    let mut job = scheduler.repository.load_render_job(id).await?.ok_or(SchedulerError::MissingJob)?;
    if job.state == JobState::Cancelled {
        return Ok(RunReport { cancelled: true, ..RunReport::default() });
    }
    if matches!(job.state, JobState::Completed | JobState::Failed) {
        return Ok(RunReport::default());
    }
    let mut cursor = decode_cursor(&job)?;
    if cursor.next > cursor.coordinates.len() || job.completed_chunks != cursor.next as u64 {
        return Err(SchedulerError::InvalidJob("render job cursor and completed count disagree"));
    }
    if cursor.coordinates.is_empty() {
        job.state = JobState::Completed;
        scheduler.repository.update_render_job(job).await?;
        return Ok(RunReport::default());
    }
    if scheduler.is_cancelled(id) {
        return Ok(RunReport { cancelled: true, ..RunReport::default() });
    }
    job.state = JobState::Running;
    scheduler.repository.update_render_job(job.clone()).await?;
    let mut report = RunReport::default();
    while cursor.next < cursor.coordinates.len() && report.selected < max_steps {
        scheduler.wait_unpaused().await;
        if scheduler.is_cancelled(id) {
            report.cancelled = true;
            break;
        }
        let coordinate = cursor.coordinates[cursor.next];
        report.selected += 1;
        match scheduler.render_one(&job.world, coordinate, cursor.revision, Some(id)).await? {
            RenderDisposition::Installed | RenderDisposition::Missing => {
                if scheduler.is_cancelled(id) {
                    report.cancelled = true;
                    break;
                }
                cursor.next += 1;
                job.completed_chunks = cursor.next as u64;
                job.payload = serde_json::to_vec(&cursor)?;
                job.state = if cursor.next == cursor.coordinates.len() {
                    JobState::Completed
                } else {
                    JobState::Running
                };
                scheduler.repository.update_render_job(job.clone()).await?;
                report.completed += 1;
            }
            RenderDisposition::Stale => {
                report.stale += 1;
                break;
            }
            RenderDisposition::Cancelled => {
                report.cancelled = true;
                break;
            }
        }
    }
    if report.cancelled {
        let current = scheduler.repository.load_render_job(id).await?;
        if let Some(mut current) = current.filter(|job| job.state != JobState::Cancelled) {
            current.state = JobState::Cancelled;
            scheduler.repository.update_render_job(current).await?;
        }
    } else if cursor.next < cursor.coordinates.len() {
        job.state = JobState::Resumable;
        job.payload = serde_json::to_vec(&cursor)?;
        scheduler.repository.update_render_job(job).await?;
    }
    Ok(report)
}

pub(super) async fn cancel(scheduler: &Scheduler, id: &[u8]) -> Result<(), SchedulerError> {
    scheduler.cancelled_jobs.lock()
        .map_err(|_| SchedulerError::InvalidJob("cancel set lock poisoned"))?
        .insert(id.to_vec());
    let mut job = scheduler.repository.load_render_job(id).await?.ok_or(SchedulerError::MissingJob)?;
    if !matches!(job.state, JobState::Completed | JobState::Failed | JobState::Cancelled) {
        job.state = JobState::Cancelled;
        scheduler.repository.update_render_job(job).await?;
    }
    Ok(())
}

pub(super) async fn resume_all(scheduler: &Scheduler) -> Result<RunReport, SchedulerError> {
    let jobs = scheduler.repository.recover().await?.jobs;
    let mut total = RunReport::default();
    for job in jobs {
        let report = run(scheduler, &job.id, usize::MAX).await?;
        total.selected += report.selected;
        total.completed += report.completed;
        total.stale += report.stale;
        total.cancelled |= report.cancelled;
    }
    Ok(total)
}
