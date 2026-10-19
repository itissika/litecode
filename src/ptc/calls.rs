//! Host-call ledger. Every submitted call ends as completed, cancelled and
//! finished, or cancelled and still running when the grace period ends.

use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::{CallRecord, CallState};

struct Slot {
    seq: u64,
    namespace: String,
    name: String,
    started: Duration,
    ended: Option<Duration>,
    done: bool,
    frozen: bool,
    state: CallState,
    token: CancellationToken,
}

struct Inner {
    in_flight: usize,
    next_seq: u64,
    slots: Vec<Slot>,
}

pub(crate) struct Ledger {
    max_in_flight: usize,
    parent: CancellationToken,
    inner: Mutex<Inner>,
}

pub(crate) struct Ticket {
    pub idx: usize,
    pub token: CancellationToken,
}

impl Ledger {
    pub(crate) fn new(max_in_flight: usize, parent: CancellationToken) -> Self {
        Self {
            max_in_flight,
            parent,
            inner: Mutex::new(Inner {
                in_flight: 0,
                next_seq: 1,
                slots: Vec::new(),
            }),
        }
    }

    pub(crate) fn in_flight(&self) -> usize {
        lock(&self.inner).in_flight
    }

    /// Reserve a slot and a child cancellation token. `None` when the in-flight
    /// cap is already reached; the host is not called.
    pub(crate) fn begin(&self, namespace: &str, name: &str, started: Duration) -> Option<Ticket> {
        let mut guard = lock(&self.inner);
        if guard.in_flight >= self.max_in_flight {
            return None;
        }
        guard.in_flight += 1;
        let seq = guard.next_seq;
        guard.next_seq += 1;
        let token = self.parent.child_token();
        guard.slots.push(Slot {
            seq,
            namespace: namespace.to_string(),
            name: name.to_string(),
            started,
            ended: None,
            done: false,
            frozen: false,
            state: CallState::Completed,
            token: token.clone(),
        });
        Some(Ticket {
            idx: guard.slots.len() - 1,
            token,
        })
    }

    pub(crate) fn finish(&self, idx: usize, ended: Duration) {
        let mut guard = lock(&self.inner);
        let slot = &mut guard.slots[idx];
        if !slot.frozen {
            slot.done = true;
            slot.ended = Some(ended);
            slot.state = if slot.token.is_cancelled() {
                CallState::CancelledFinished
            } else {
                CallState::Completed
            };
        }
        guard.in_flight = guard.in_flight.saturating_sub(1);
    }

    /// Signal cancellation to calls that have not finished. Does not decide
    /// their terminal state; [`seal`](Self::seal) does that after the grace period.
    pub(crate) fn cancel_open(&self) {
        let guard = lock(&self.inner);
        for slot in &guard.slots {
            if !slot.done && !slot.frozen {
                slot.token.cancel();
            }
        }
    }

    /// Cancel open calls, wait out the grace period, then freeze whatever is
    /// still running as [`CallState::CancelledUnfinished`].
    pub(crate) fn seal(&self, grace: Duration) {
        self.cancel_open();
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if self.in_flight() == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let mut guard = lock(&self.inner);
        for slot in &mut guard.slots {
            if !slot.done {
                slot.frozen = true;
                slot.state = CallState::CancelledUnfinished;
                slot.ended = None;
            }
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<CallRecord> {
        let guard = lock(&self.inner);
        guard
            .slots
            .iter()
            .map(|slot| CallRecord {
                seq: slot.seq,
                namespace: slot.namespace.clone(),
                name: slot.name.clone(),
                started: slot.started,
                ended: slot.ended,
                state: if slot.done || slot.frozen {
                    slot.state
                } else {
                    CallState::CancelledUnfinished
                },
            })
            .collect()
    }
}

fn lock(mutex: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}
