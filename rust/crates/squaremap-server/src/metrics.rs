use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Default)]
pub struct Metrics {
    queue_depth: AtomicU64,
    render_latency_nanos: AtomicU64,
    http_requests: AtomicU64,
    restarts: AtomicU64,
    protocol_errors: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct MetricsSnapshot {
    pub queue_depth: u64,
    pub render_latency_nanos: u64,
    pub http_requests: u64,
    pub restarts: u64,
    pub protocol_errors: u64,
}

impl Metrics {
    pub fn set_queue_depth(&self, value: u64) {
        self.queue_depth.store(value, Ordering::Relaxed);
    }
    pub fn observe_render_latency(&self, value: Duration) {
        self.render_latency_nanos.store(
            value.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
    }
    pub fn inc_http_requests(&self) {
        self.http_requests.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_restarts(&self) {
        self.restarts.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_protocol_errors(&self) {
        self.protocol_errors.fetch_add(1, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            render_latency_nanos: self.render_latency_nanos.load(Ordering::Relaxed),
            http_requests: self.http_requests.load(Ordering::Relaxed),
            restarts: self.restarts.load(Ordering::Relaxed),
            protocol_errors: self.protocol_errors.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Metrics;
    use std::time::Duration;

    #[test]
    fn snapshot_is_bounded_and_deterministic() {
        let metrics = Metrics::default();
        metrics.set_queue_depth(4);
        metrics.observe_render_latency(Duration::from_nanos(9));
        metrics.inc_http_requests();
        metrics.inc_restarts();
        metrics.inc_protocol_errors();
        assert_eq!(metrics.snapshot().queue_depth, 4);
        assert_eq!(metrics.snapshot().render_latency_nanos, 9);
        assert_eq!(metrics.snapshot().http_requests, 1);
        assert_eq!(metrics.snapshot().restarts, 1);
        assert_eq!(metrics.snapshot().protocol_errors, 1);
    }
}
