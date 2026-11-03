//! In-process writer metrics. No external collector: a snapshot is enough to
//! see the queue and commit path, and a fixed recent window keeps the p50/p95
//! calculation bounded.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SAMPLE_WINDOW: usize = 512;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriterState {
    #[default]
    Starting = 0,
    Ready = 1,
    Draining = 2,
    Closed = 3,
}

impl WriterState {
    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Ready,
            2 => Self::Draining,
            3 => Self::Closed,
            _ => Self::Starting,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriterMetricsSnapshot {
    pub state: WriterState,
    pub queue_depth: usize,
    pub queue_capacity: usize,
    /// Number of recent duration samples behind the p50/p95 fields.
    pub sample_window: usize,
    pub enqueued: u64,
    pub committed: u64,
    pub failed: u64,
    pub conflicts: u64,
    pub backpressure: u64,
    pub timeouts: u64,
    pub canceled: u64,
    pub closed: u64,
    pub shutdowns: u64,
    pub writer_panics: u64,
    pub writer_alive: bool,
    pub last_commit_at_ms: Option<i64>,
    pub queue_wait_p50_ms: Option<u64>,
    pub queue_wait_p95_ms: Option<u64>,
    pub commit_p50_ms: Option<u64>,
    pub commit_p95_ms: Option<u64>,
}

pub(crate) struct WriterMetrics {
    queue_depth: AtomicUsize,
    queue_capacity: usize,
    enqueued: AtomicU64,
    committed: AtomicU64,
    failed: AtomicU64,
    conflicts: AtomicU64,
    backpressure: AtomicU64,
    timeouts: AtomicU64,
    canceled: AtomicU64,
    closed: AtomicU64,
    shutdowns: AtomicU64,
    writer_panics: AtomicU64,
    alive: AtomicBool,
    last_commit_at_ms: AtomicI64,
    queue_wait_ms: Mutex<VecDeque<u64>>,
    commit_ms: Mutex<VecDeque<u64>>,
}

impl WriterMetrics {
    pub(crate) fn new(queue_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            queue_depth: AtomicUsize::new(0),
            queue_capacity,
            enqueued: AtomicU64::new(0),
            committed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            conflicts: AtomicU64::new(0),
            backpressure: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            canceled: AtomicU64::new(0),
            closed: AtomicU64::new(0),
            shutdowns: AtomicU64::new(0),
            writer_panics: AtomicU64::new(0),
            alive: AtomicBool::new(false),
            last_commit_at_ms: AtomicI64::new(0),
            queue_wait_ms: Mutex::new(VecDeque::with_capacity(SAMPLE_WINDOW)),
            commit_ms: Mutex::new(VecDeque::with_capacity(SAMPLE_WINDOW)),
        })
    }

    pub(crate) fn set_alive(&self, alive: bool) {
        self.alive.store(alive, Ordering::SeqCst);
    }

    /// Reserve a queue-depth slot before attempting the send. The writer can
    /// dequeue only after the send succeeds, so incrementing first removes the
    /// "dequeue before enqueue accounting" race. The guard also rolls back if
    /// the caller future is cancelled while waiting for channel capacity.
    pub(crate) fn admit(&self) -> EnqueueAdmission<'_> {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        EnqueueAdmission {
            metrics: self,
            confirmed: false,
        }
    }

    pub(crate) fn record_dequeue(&self) {
        let _ = self
            .queue_depth
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                Some(depth.saturating_sub(1))
            });
    }

    pub(crate) fn record_queue_wait(&self, elapsed: Duration) {
        push_sample(&self.queue_wait_ms, elapsed);
    }

    pub(crate) fn record_commit(&self, elapsed: Duration) {
        self.committed.fetch_add(1, Ordering::Relaxed);
        self.last_commit_at_ms.store(now_ms(), Ordering::Relaxed);
        push_sample(&self.commit_ms, elapsed);
    }

    pub(crate) fn record_failed(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_conflict(&self) {
        self.conflicts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_backpressure(&self) {
        self.backpressure.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_timeout(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_canceled(&self) {
        self.canceled.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_closed(&self) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_panic(&self) {
        self.writer_panics.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self, state: WriterState) -> WriterMetricsSnapshot {
        let queue_wait = samples(&self.queue_wait_ms);
        let commit = samples(&self.commit_ms);
        let last_commit_at_ms = match self.last_commit_at_ms.load(Ordering::Relaxed) {
            0 => None,
            value => Some(value),
        };
        WriterMetricsSnapshot {
            state,
            queue_depth: self
                .queue_depth
                .load(Ordering::Relaxed)
                .min(self.queue_capacity),
            queue_capacity: self.queue_capacity,
            sample_window: SAMPLE_WINDOW,
            enqueued: self.enqueued.load(Ordering::Relaxed),
            committed: self.committed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            conflicts: self.conflicts.load(Ordering::Relaxed),
            backpressure: self.backpressure.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            canceled: self.canceled.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Relaxed),
            shutdowns: self.shutdowns.load(Ordering::Relaxed),
            writer_panics: self.writer_panics.load(Ordering::Relaxed),
            writer_alive: self.alive.load(Ordering::Relaxed),
            last_commit_at_ms,
            queue_wait_p50_ms: percentile(&queue_wait, 0.50),
            queue_wait_p95_ms: percentile(&queue_wait, 0.95),
            commit_p50_ms: percentile(&commit, 0.50),
            commit_p95_ms: percentile(&commit, 0.95),
        }
    }
}

/// One queue admission in progress. Confirming keeps the reserved depth slot
/// and counts the command; dropping without confirmation rolls the slot back.
pub(crate) struct EnqueueAdmission<'a> {
    metrics: &'a WriterMetrics,
    confirmed: bool,
}

impl EnqueueAdmission<'_> {
    pub(crate) fn confirm(mut self) {
        self.metrics.enqueued.fetch_add(1, Ordering::Relaxed);
        self.confirmed = true;
    }
}

impl Drop for EnqueueAdmission<'_> {
    fn drop(&mut self) {
        if !self.confirmed {
            self.metrics.record_dequeue();
        }
    }
}

/// Set alive on construction and clear it on every exit path, including a
/// panic in the writer thread.
pub(crate) struct WriterAliveGuard(Arc<WriterMetrics>);

impl WriterAliveGuard {
    pub(crate) fn new(metrics: Arc<WriterMetrics>) -> Self {
        metrics.set_alive(true);
        Self(metrics)
    }
}

impl Drop for WriterAliveGuard {
    fn drop(&mut self) {
        self.0.set_alive(false);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn push_sample(window: &Mutex<VecDeque<u64>>, elapsed: Duration) {
    let mut window = window.lock().unwrap_or_else(|e| e.into_inner());
    if window.len() == SAMPLE_WINDOW {
        window.pop_front();
    }
    window.push_back(elapsed.as_millis() as u64);
}

fn samples(window: &Mutex<VecDeque<u64>>) -> Vec<u64> {
    window
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .copied()
        .collect()
}

fn percentile(sorted: &[u64], quantile: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let mut sorted = sorted.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    sorted.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_bounded_and_monotonic() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7], 0.95), Some(7));
        let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let p50 = percentile(&values, 0.5).unwrap();
        let p95 = percentile(&values, 0.95).unwrap();
        assert!(p50 <= p95);
        assert_eq!(p50, 6);
        assert_eq!(p95, 10);
    }

    #[test]
    fn snapshot_starts_empty() {
        let metrics = WriterMetrics::new(4);
        let snapshot = metrics.snapshot(WriterState::Ready);
        assert_eq!(snapshot.state, WriterState::Ready);
        assert_eq!(snapshot.queue_capacity, 4);
        assert_eq!(snapshot.sample_window, SAMPLE_WINDOW);
        assert_eq!(snapshot.queue_depth, 0);
        assert_eq!(snapshot.last_commit_at_ms, None);
    }
}
