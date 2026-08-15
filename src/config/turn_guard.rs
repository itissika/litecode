//! Turn-level guard blocking settings writes while an agent turn is active.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

/// Blocks configuration writes while at least one turn is in progress.
#[derive(Debug, Default)]
pub struct TurnGuard {
    active: AtomicUsize,
}

impl TurnGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_turn_in_progress(&self) -> bool {
        self.active.load(Ordering::Acquire) > 0
    }

    pub fn begin_turn(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
    }

    pub fn end_turn(&self) {
        let prev = self.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(prev > 0, "end_turn without matching begin_turn");
    }
}

static CLI_TURN_GUARD: LazyLock<Arc<TurnGuard>> = LazyLock::new(|| Arc::new(TurnGuard::new()));

/// Process-wide turn guard for CLI (`litecode` single-process single-turn).
pub fn cli_turn_guard() -> Arc<TurnGuard> {
    CLI_TURN_GUARD.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_end_cycle() {
        let guard = TurnGuard::new();
        assert!(!guard.is_turn_in_progress());
        guard.begin_turn();
        assert!(guard.is_turn_in_progress());
        guard.end_turn();
        assert!(!guard.is_turn_in_progress());
    }

    #[test]
    fn nested_turns() {
        let guard = TurnGuard::new();
        guard.begin_turn();
        guard.begin_turn();
        assert!(guard.is_turn_in_progress());
        guard.end_turn();
        assert!(guard.is_turn_in_progress());
        guard.end_turn();
        assert!(!guard.is_turn_in_progress());
    }
}
