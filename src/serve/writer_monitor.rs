//! Process-level session writer monitor.
//!
//! One task per serve process, deliberately outside the writer and Websocket
//! paths. Every ten seconds it reads the in-memory snapshot. A healthy process
//! writes one INFO heartbeat per minute; a new or sustained abnormal state
//! writes WARN immediately and then at most once per five minutes, so a stuck
//! queue cannot flood the log. The final line is written when the serve loop
//! stops or the writer closes.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::session::SessionManager;
use crate::session::data::{WriterMetricsSnapshot, WriterState};

const TICK: Duration = Duration::from_secs(10);
const HEARTBEAT: Duration = Duration::from_secs(60);
const ALERT_REPEAT: Duration = Duration::from_secs(5 * 60);
const QUEUE_ALERT_PERCENT: u64 = 80;

macro_rules! log_snapshot {
    ($level:ident, $message:literal, $snapshot:expr, $reason:expr, $final_entry:expr, $recovered:expr) => {
        tracing::$level!(
            target: "litecode::session_writer",
            reason = $reason,
            final_entry = $final_entry,
            recovered = $recovered,
            state = ?$snapshot.state,
            queue_depth = $snapshot.queue_depth,
            queue_capacity = $snapshot.queue_capacity,
            queue_utilization_pct = queue_utilization_pct(&$snapshot),
            enqueued = $snapshot.enqueued,
            committed = $snapshot.committed,
            failed = $snapshot.failed,
            conflicts = $snapshot.conflicts,
            backpressure = $snapshot.backpressure,
            timeouts = $snapshot.timeouts,
            closed = $snapshot.closed,
            canceled = $snapshot.canceled,
            shutdowns = $snapshot.shutdowns,
            writer_panics = $snapshot.writer_panics,
            writer_alive = $snapshot.writer_alive,
            last_commit_at_ms = ?$snapshot.last_commit_at_ms,
            last_commit_age_ms = ?last_commit_age_ms(&$snapshot),
            queue_wait_p50_ms = ?$snapshot.queue_wait_p50_ms,
            queue_wait_p95_ms = ?$snapshot.queue_wait_p95_ms,
            commit_p50_ms = ?$snapshot.commit_p50_ms,
            commit_p95_ms = ?$snapshot.commit_p95_ms,
            sample_window = $snapshot.sample_window,
            $message
        );
    };
}

/// Raw writer telemetry is infrastructure, not a session business operation.
pub(crate) fn spawn(sessions: Arc<SessionManager>, stop: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(run(sessions, stop))
}

async fn run(sessions: Arc<SessionManager>, mut stop: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut previous: Option<WriterMetricsSnapshot> = None;
    let mut last_reason: Option<&'static str> = None;
    let mut last_info: Option<Instant> = None;
    let mut last_alert: Option<Instant> = None;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let snapshot = sessions.data().writer_metrics();
                if snapshot.state == WriterState::Closed {
                    if snapshot.shutdowns == 0 || snapshot.writer_panics > 0 {
                        log_snapshot!(
                            warn,
                            "session writer closed unexpectedly",
                            snapshot,
                            "writer_closed",
                            true,
                            false
                        );
                    } else {
                        log_snapshot!(info, "session writer final", snapshot, "", true, false);
                    }
                    break;
                }
                let reason = alert_reason(&snapshot, previous.as_ref());
                let now = Instant::now();
                match reason {
                    Some(reason) => {
                        if last_reason != Some(reason)
                            || last_alert.map_or(true, |last| {
                                now.duration_since(last) >= ALERT_REPEAT
                            })
                        {
                            log_snapshot!(warn, "session writer alert", snapshot, reason, false, false);
                            last_alert = Some(now);
                            last_info = Some(now);
                        }
                        last_reason = Some(reason);
                    }
                    None => {
                        if let Some(reason) = last_reason.take() {
                            log_snapshot!(info, "session writer recovered", snapshot, reason, false, true);
                            last_info = Some(now);
                        } else if last_info.map_or(true, |last| {
                            now.duration_since(last) >= HEARTBEAT
                        }) {
                            log_snapshot!(info, "session writer heartbeat", snapshot, "", false, false);
                            last_info = Some(now);
                        }
                    }
                }
                previous = Some(snapshot);
            }
            changed = stop.changed() => {
                let _ = changed;
                let snapshot = sessions.data().writer_metrics();
                log_snapshot!(info, "session writer final", snapshot, "", true, false);
                break;
            }
        }
    }
}

fn alert_reason(
    current: &WriterMetricsSnapshot,
    previous: Option<&WriterMetricsSnapshot>,
) -> Option<&'static str> {
    if !current.writer_alive {
        return Some("writer_not_alive");
    }
    if !matches!(current.state, WriterState::Ready | WriterState::Draining) {
        return Some("writer_state");
    }
    if current.queue_capacity > 0
        && current.queue_depth as u64 * 100 / current.queue_capacity as u64 >= QUEUE_ALERT_PERCENT
    {
        return Some("queue_depth");
    }
    if current.backpressure > previous.map_or(0, |p| p.backpressure) {
        return Some("backpressure");
    }
    if current.timeouts > previous.map_or(0, |p| p.timeouts) {
        return Some("timeout");
    }
    if current.writer_panics > previous.map_or(0, |p| p.writer_panics) {
        return Some("writer_panic");
    }
    None
}

fn queue_utilization_pct(snapshot: &WriterMetricsSnapshot) -> u8 {
    if snapshot.queue_capacity == 0 {
        return 0;
    }
    (snapshot.queue_depth as u64 * 100 / snapshot.queue_capacity as u64).min(100) as u8
}

fn last_commit_age_ms(snapshot: &WriterMetricsSnapshot) -> Option<u64> {
    let last = snapshot.last_commit_at_ms?;
    let last = u64::try_from(last).ok()?;
    Some(now_ms().saturating_sub(last))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(state: WriterState, depth: usize) -> WriterMetricsSnapshot {
        WriterMetricsSnapshot {
            state,
            queue_depth: depth,
            queue_capacity: 100,
            sample_window: 512,
            writer_alive: true,
            ..WriterMetricsSnapshot::default()
        }
    }

    #[test]
    fn queue_alert_starts_at_eighty_percent() {
        assert_eq!(alert_reason(&snapshot(WriterState::Ready, 79), None), None);
        assert_eq!(
            alert_reason(&snapshot(WriterState::Ready, 80), None),
            Some("queue_depth")
        );
        assert_eq!(queue_utilization_pct(&snapshot(WriterState::Ready, 80)), 80);
    }

    #[test]
    fn counter_deltas_raise_alerts_but_normal_states_do_not() {
        let mut previous = snapshot(WriterState::Ready, 1);
        let mut current = previous.clone();
        current.backpressure = 1;
        assert_eq!(
            alert_reason(&current, Some(&previous)),
            Some("backpressure")
        );
        previous.backpressure = 1;
        current.backpressure = 1;
        previous.timeouts = 1;
        current.timeouts = 2;
        assert_eq!(alert_reason(&current, Some(&previous)), Some("timeout"));
        assert_eq!(
            alert_reason(&snapshot(WriterState::Draining, 1), None),
            None
        );
    }

    #[test]
    fn not_alive_is_an_alert() {
        let mut current = snapshot(WriterState::Ready, 0);
        current.writer_alive = false;
        assert_eq!(alert_reason(&current, None), Some("writer_not_alive"));
    }

    #[tokio::test]
    async fn stop_signal_ends_the_process_monitor() {
        let sessions = Arc::new(crate::session::manager::SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let (stop_tx, stop_rx) = watch::channel(false);
        let monitor = spawn(sessions, stop_rx);
        stop_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), monitor)
            .await
            .expect("monitor must stop promptly")
            .expect("monitor task");
    }
}
