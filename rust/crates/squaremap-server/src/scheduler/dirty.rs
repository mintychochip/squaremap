use super::{RenderDisposition, RunReport, Scheduler, SchedulerError};
use futures_util::stream::{self, StreamExt};

pub(super) async fn run_page(scheduler: &Scheduler) -> Result<RunReport, SchedulerError> {
    let now = current_seconds();
    let rows = scheduler.repository.dirty_page_at(scheduler.config.dirty_page_size, now).await?;
    let selected = rows.len();
    let concurrency = selected.max(1);
    let results = stream::iter(rows)
        .map(|dirty| async move {
            let disposition = scheduler.render_one(
                &dirty.world,
                dirty.coordinate,
                dirty.revision,
                None,
            ).await?;
            match disposition {
                RenderDisposition::Installed => {
                    scheduler.repository.complete_dirty(
                        &dirty.world,
                        dirty.coordinate,
                        dirty.revision,
                    ).await?;
                }
                RenderDisposition::Missing => {
                    scheduler.repository.defer_dirty(
                        &dirty.world,
                        dirty.coordinate,
                        dirty.revision,
                        now,
                    ).await?;
                }
                RenderDisposition::Stale | RenderDisposition::Cancelled => {}
            }
            Ok::<_, SchedulerError>(disposition)
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    let mut report = RunReport { selected, ..RunReport::default() };
    for result in results {
        match result? {
            RenderDisposition::Installed => report.completed += 1,
            RenderDisposition::Missing => {}
            RenderDisposition::Stale => report.stale += 1,
            RenderDisposition::Cancelled => report.cancelled = true,
        }
    }
    Ok(report)
}

fn current_seconds() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
