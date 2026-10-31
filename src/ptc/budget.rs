//! Pausable program budget.
//!
//! Host code holds a [`BudgetPauseGuard`] across an approval wait. While any
//! guard is alive the active clock does not advance, so the interrupt handler
//! and the idle wait both ignore that interval.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct ClockInner {
    origin: Instant,
    limit: Duration,
    pause_count: u32,
    pause_started: Option<Instant>,
    paused: Duration,
}

impl ClockInner {
    fn active(&self, now: Instant) -> Duration {
        let mut paused = self.paused;
        if let Some(started) = self.pause_started {
            paused = paused.saturating_add(now.saturating_duration_since(started));
        }
        now.saturating_duration_since(self.origin)
            .saturating_sub(paused)
    }
}

#[derive(Debug)]
pub(crate) struct BudgetClock {
    inner: Mutex<ClockInner>,
}

impl BudgetClock {
    pub(crate) fn new(limit: Duration) -> Self {
        Self {
            inner: Mutex::new(ClockInner {
                origin: Instant::now(),
                limit,
                pause_count: 0,
                pause_started: None,
                paused: Duration::ZERO,
            }),
        }
    }

    pub(crate) fn handle(clock: &std::sync::Arc<Self>) -> BudgetPause {
        BudgetPause {
            clock: std::sync::Arc::clone(clock),
        }
    }

    pub(crate) fn wall_elapsed(&self) -> Duration {
        lock(&self.inner).origin.elapsed()
    }

    pub(crate) fn exhausted(&self) -> bool {
        let guard = lock(&self.inner);
        let now = Instant::now();
        guard.active(now) >= guard.limit
    }

    /// Remaining active budget. Zero once the limit has been reached.
    pub(crate) fn remaining(&self) -> Duration {
        let guard = lock(&self.inner);
        let now = Instant::now();
        guard.limit.saturating_sub(guard.active(now))
    }

    /// Interrupt-handler view. A contended pause lock skips this tick instead
    /// of blocking the engine.
    pub(crate) fn should_interrupt(&self) -> bool {
        let Ok(guard) = self.inner.try_lock() else {
            return false;
        };
        let now = Instant::now();
        guard.active(now) >= guard.limit
    }

    fn pause(&self) {
        let mut guard = lock(&self.inner);
        if guard.pause_count == 0 {
            guard.pause_started = Some(Instant::now());
        }
        guard.pause_count += 1;
    }

    fn resume(&self) {
        let mut guard = lock(&self.inner);
        guard.pause_count = guard.pause_count.saturating_sub(1);
        if guard.pause_count == 0
            && let Some(started) = guard.pause_started.take()
        {
            guard.paused = guard.paused.saturating_add(started.elapsed());
        }
    }
}

fn lock(mutex: &Mutex<ClockInner>) -> MutexGuard<'_, ClockInner> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Handle passed to a host function so it can pause the program budget.
#[derive(Debug, Clone)]
pub struct BudgetPause {
    clock: std::sync::Arc<BudgetClock>,
}

impl BudgetPause {
    /// Pause the program budget until the guard is dropped.
    ///
    /// Nested pauses keep the clock stopped until the last guard drops.
    /// Forgetting the guard leaves the budget paused.
    pub fn pause(&self) -> BudgetPauseGuard {
        self.clock.pause();
        BudgetPauseGuard {
            clock: std::sync::Arc::clone(&self.clock),
        }
    }
}

/// RAII pause of the program budget.
#[must_use = "dropping the guard resumes the budget clock"]
#[derive(Debug)]
pub struct BudgetPauseGuard {
    clock: std::sync::Arc<BudgetClock>,
}

impl Drop for BudgetPauseGuard {
    fn drop(&mut self) {
        self.clock.resume();
    }
}

#[cfg(test)]
mod tests {}
