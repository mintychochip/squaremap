use super::{RenderDisposition, RunReport, Scheduler, SchedulerError};
use futures_util::stream::{self, StreamExt};

pub(super) async fn run_page(scheduler: &Scheduler) -> Result<RunReport, SchedulerError> {
    let now = current_seconds();
    let rows = scheduler
        .repository
        .dirty_page_at(scheduler.config.dirty_page_size, now)
        .await?;
    let selected = rows.len();
    let concurrency = selected.max(1);
    let results = stream::iter(rows)
        .map(|dirty| async move {
            let disposition = scheduler
                .render_one(&dirty.world, dirty.coordinate, dirty.revision, None, true, true)
                .await?;
            match disposition {
                RenderDisposition::Installed => {
                    scheduler
                        .repository
                        .complete_dirty(&dirty.world, dirty.coordinate, dirty.revision)
                        .await?;
                }
                RenderDisposition::Missing => {
                    scheduler
                        .repository
                        .defer_dirty(&dirty.world, dirty.coordinate, dirty.revision, now)
                        .await?;
                }
                RenderDisposition::Stale | RenderDisposition::Cancelled => {}
            }
            Ok::<_, SchedulerError>(disposition)
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    let mut report = RunReport {
        selected,
        ..RunReport::default()
    };
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

/// Owner-scoped dirty page: leases rows for `bridge_id`, renders each, and only
/// completes or defers rows owned by that bridge. Other owners' rows are never
/// selected, leased, completed, or deferred.
pub(super) async fn run_owner_page(
    scheduler: &Scheduler,
    bridge_id: &[u8],
    focus: Option<(i32, i32)>,
) -> Result<RunReport, SchedulerError> {
    let now = current_seconds();
    let limit = scheduler.config.dirty_page_size.min(super::LIVE_DIRTY_PAGE_SIZE);
    let rows = scheduler
        .repository
        .dirty_page_for_owner_near(bridge_id, limit, now, focus)
        .await?;
    let selected = rows.len();
    let concurrency = selected.min(super::LIVE_DIRTY_CONCURRENCY).max(1);
    let bridge_id = bridge_id.to_vec();
    // Player-focused pages already picked the loaded neighborhood. Allow
    // snapshot/load so an off-thread loaded_only miss cannot skip those cells.
    let loaded_only = focus.is_none();
    let results = stream::iter(rows)
        .map(|dirty| {
            let bridge_id = bridge_id.clone();
            async move {
                let disposition = match scheduler
                    .render_one(
                        &dirty.world,
                        dirty.coordinate,
                        dirty.revision,
                        None,
                        loaded_only,
                        false,
                    )
                    .await
                {
                    Ok(disposition) => disposition,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            x = dirty.coordinate.x,
                            z = dirty.coordinate.z,
                            "skipping live dirty chunk after render error"
                        );
                        RenderDisposition::Stale
                    }
                };
                match disposition {
                    RenderDisposition::Installed => {
                        if !scheduler
                            .repository
                            .complete_dirty_for_owner(
                                &dirty.world,
                                dirty.coordinate,
                                dirty.revision,
                                &bridge_id,
                            )
                            .await?
                        {
                            return Ok(RenderDisposition::Stale);
                        }
                    }
                    RenderDisposition::Missing => {
                        scheduler
                            .repository
                            .defer_dirty_for_owner(
                                &dirty.world,
                                dirty.coordinate,
                                dirty.revision,
                                &bridge_id,
                                now,
                            )
                            .await?;
                    }
                    RenderDisposition::Stale => {
                        // Live ticks must not pin the newest page on Unavailable
                        // (Java loaded-only false misses). Defer so the player's
                        // already-loaded interior can enter the next page.
                        scheduler
                            .repository
                            .defer_dirty_for_owner(
                                &dirty.world,
                                dirty.coordinate,
                                dirty.revision,
                                &bridge_id,
                                now,
                            )
                            .await?;
                    }
                    RenderDisposition::Cancelled => {}
                }
                Ok::<_, SchedulerError>(disposition)
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    let mut report = RunReport {
        selected,
        ..RunReport::default()
    };
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
