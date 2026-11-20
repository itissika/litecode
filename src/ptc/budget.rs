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
    human_pause_count: u32,
    human_pause_started: Option<Instant>,
    human_paused: Duration,
}

impl ClockInner {
    fn paused_at(&self, now: Instant) -> Duration {
        let mut paused = self.paused;
        if let Some(started) = self.pause_started {
            paused = paused.saturating_add(now.saturating_duration_since(started));
        }
        paused
    }

    fn human_paused_at(&self, now: Instant) -> Duration {
        let mut paused = self.human_paused;
        if let Some(started) = self.human_pause_started {
            paused = paused.saturating_add(now.saturating_duration_since(started));
        }
        paused
    }

    fn active(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.origin)
            .saturating_sub(self.paused_at(now))
    }

    fn wall_elapsed_at(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.origin)
            .saturating_sub(self.human_paused_at(now))
    }

    fn wall_expired(&self, now: Instant) -> bool {
        self.wall_limit
            .is_some_and(|wall| self.wall_elapsed_at(now) >= wall)
    }

    fn remaining_at(&self, now: Instant) -> Duration {
        let active_rem = self.limit.saturating_sub(self.active(now));
        match self.wall_limit {
            Some(wall) => {
                let wall_rem = wall.saturating_sub(self.wall_elapsed_at(now));
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
                human_pause_count: 0,
                human_pause_started: None,
                human_paused: Duration::ZERO,
            }),
        }
    }

    pub(crate) fn handle(clock: &std::sync::Arc<Self>) -> BudgetPause {
        BudgetPause {
            clock: std::sync::Arc::clone(clock),
        }
    }

    pub(crate) fn wall_elapsed(&self) -> Duration {
        lock(&self.inner).wall_elapsed_at(Instant::now())
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
        Self::pause_active_inner(&mut guard);
    }

    fn resume(&self) {
        let mut guard = lock(&self.inner);
        Self::resume_active_inner(&mut guard);
    }

    fn pause_human(&self) {
        let mut guard = lock(&self.inner);
        Self::pause_active_inner(&mut guard);
        if guard.human_pause_count == 0 {
            guard.human_pause_started = Some(Instant::now());
        }
        guard.human_pause_count += 1;
    }

    fn resume_human(&self) {
        let mut guard = lock(&self.inner);
        guard.human_pause_count = guard.human_pause_count.saturating_sub(1);
        if guard.human_pause_count == 0
            && let Some(started) = guard.human_pause_started.take()
        {
            guard.human_paused = guard.human_paused.saturating_add(started.elapsed());
        }
        Self::resume_active_inner(&mut guard);
    }

    fn pause_active_inner(guard: &mut ClockInner) {
        if guard.pause_count == 0 {
            guard.pause_started = Some(Instant::now());
        }
        guard.pause_count += 1;
    }

    fn resume_active_inner(guard: &mut ClockInner) {
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
    /// Pause the program active budget until the guard is dropped.
    ///
    /// Nested pauses keep the active clock stopped until the last guard drops
    /// (while `wall_limit` keeps ticking unless [`Self::pause_human`] is held).
    pub fn pause(&self) -> BudgetPauseGuard {
        self.clock.pause();
        BudgetPauseGuard {
            clock: std::sync::Arc::clone(&self.clock),
        }
    }

    /// Pause both active and wall budgets while waiting for human input.
    pub fn pause_human(&self) -> HumanPauseGuard {
        self.clock.pause_human();
        HumanPauseGuard {
            clock: std::sync::Arc::clone(&self.clock),
        }
    }
}

/// RAII pause of the program active budget.
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

/// RAII pause of both active and wall budgets across human confirmation/input.
#[must_use = "dropping the guard resumes the budget clocks"]
#[derive(Debug)]
pub struct HumanPauseGuard {
    clock: std::sync::Arc<BudgetClock>,
}

impl Drop for HumanPauseGuard {
    fn drop(&mut self) {
        self.clock.resume_human();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_human_freezes_both_active_and_wall_clocks_without_double_counting() {
        let clock = std::sync::Arc::new(BudgetClock::with_wall_limit(
            Duration::from_millis(80),
            Some(Duration::from_millis(80)),
        ));
        let pause = BudgetClock::handle(&clock);
        let _nested = pause.pause();
        let human = pause.pause_human();
        std::thread::sleep(Duration::from_millis(120));
        assert!(!clock.wall_exhausted(), "wall clock must freeze across human wait");
        assert!(!clock.exhausted(), "active clock must freeze too");
        drop(human);
        assert!(clock.wall_elapsed() < Duration::from_millis(60));
    }
}
