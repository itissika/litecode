//! Per-call scope the executor installs around one tool future.
//!
//! Carries two facts a tool cannot see from [`super::trait_::ToolExecutionContext`]
//! without widening every constructor:
//! - `deadline`: when the enclosing `run_code` program budget ends (nested calls
//!   only). `run` caps its foreground window to it.
//! - `spawn_locks`: the resource-gate and write-lock guards for a tool that
//!   [`super::Tool::releases_lock_after_spawn`]. The tool calls
//!   [`SpawnLocks::release`] once its child process is spawned; the executor's
//!   own clone drops them on every other path (spawn failure, cancel, timeout,
//!   panic unwind), so a lock can never outlive the call.

use std::any::Any;
use std::sync::{Arc, Mutex};

tokio::task_local! {
    static RUN_SCOPE: RunScope;
}

#[derive(Clone, Default)]
pub struct RunScope {
    pub deadline: Option<tokio::time::Instant>,
    pub spawn_locks: SpawnLocks,
}

/// Shared slot of type-erased lock guards. Dropping the last clone or calling
/// [`SpawnLocks::release`] drops the guards (their `Drop` releases the locks).
#[derive(Clone, Default)]
pub struct SpawnLocks(Arc<Mutex<Vec<Box<dyn Any + Send>>>>);

impl SpawnLocks {
    pub fn hold(&self, guard: Box<dyn Any + Send>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(guard);
    }

    /// Drop every held guard now. Idempotent; returns how many were released.
    pub fn release(&self) -> usize {
        let drained: Vec<_> = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect();
        let n = drained.len();
        drop(drained);
        n
    }

    pub fn held(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Scope of the current tool future, if the executor installed one.
pub fn current() -> Option<RunScope> {
    RUN_SCOPE.try_with(|scope| scope.clone()).ok()
}

/// Run `fut` with `scope` visible to [`current`].
pub async fn scope<F: std::future::Future>(scope: RunScope, fut: F) -> F::Output {
    RUN_SCOPE.scope(scope, fut).await
}
