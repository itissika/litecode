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
    wall_limit: Option<Duration>,
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

    fn wall_expired(&self, now: Instant) -> bool {
        self.wall_limit
            .is_some_and(|wall| now.saturating_duration_since(self.origin) >= wall)
    }

    fn remaining_at(&self, now: Instant) -> Duration {
        let active_rem = self.limit.saturating_sub(self.active(now));
        match self.wall_limit {
            Some(wall) => {
                let wall_rem = wall.saturating_sub(now.saturating_duration_since(self.origin));
                active_rem.min(wall_rem)
            }
            None => active_rem,
        }
    }
}

#[derive(Debug)]
pub(crate) struct BudgetClock {
    inner: Mutex<ClockInner>,
}

impl BudgetClock {
    #[cfg(test)]
    pub(crate) fn new(limit: Duration) -> Self {
        Self::with_wall_limit(limit, None)
    }

    pub(crate) fn with_wall_limit(limit: Duration, wall_limit: Option<Duration>) -> Self {
        Self {
            inner: Mutex::new(ClockInner {
                origin: Instant::now(),
                limit,
                wall_limit,
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

    pub(crate) fn wall_exhausted(&self) -> bool {
        let guard = lock(&self.inner);
        guard.wall_expired(Instant::now())
    }

    pub(crate) fn exhausted(&self) -> bool {
        let guard = lock(&self.inner);
        let now = Instant::now();
        guard.active(now) >= guard.limit || guard.wall_expired(now)
    }

    /// Remaining active or wall budget. Zero once either limit has been reached.
    pub(crate) fn remaining(&self) -> Duration {
        let guard = lock(&self.inner);
        guard.remaining_at(Instant::now())
    }

    /// Interrupt-handler view. A contended pause lock skips this tick instead
    /// of blocking the engine.
    pub(crate) fn should_interrupt(&self) -> bool {
        let Ok(guard) = self.inner.try_lock() else {
            return false;
        };
        let now = Instant::now();
        guard.active(now) >= guard.limit || guard.wall_expired(now)
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
