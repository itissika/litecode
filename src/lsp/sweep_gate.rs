//! Orphan language-server sweep gate (WP-167).
//!
//! `litecode serve` used to run [`crate::proc_lifetime::sweep_orphan_lsp_processes`]
//! inline before READY. It now runs on a background thread, and every
//! language-server spawn ([`crate::lsp::LspHub`]'s single spawn point) awaits
//! this gate first, so the sweep still finishes before the first language
//! server starts.
//!
//! The gate opens on every exit path of the sweep: normal return, early
//! return and panic (a drop guard plus `catch_unwind`). If the sweep thread
//! cannot be started, the sweep runs inline, as before. There is deliberately
//! no timeout: opening early would break the "sweep before spawn" guarantee.
//! A gate that was never armed (CLI, unit tests) lets every waiter through.
//!
//! Non-serve entries are unarmed on purpose (WP-167 plan §1, "CLI / 单测从不
//! arm → wait() 直通，等同今日"): only `serve::run` ever called the sweep, so
//! CLI never swept before WP-167 either, and arming there would add a sweep
//! that did not exist. `serve::run` (src/serve/run.rs) is the only production
//! caller of [`SweepGate::start_background`]; `cli.rs` reaches it only via the
//! `Serve` subcommand. Known gaps (unchanged by WP-167, not a regression):
//! a non-serve process never reaps orphans, and if a future non-serve entry
//! wants "sweep before spawn" it must arm the global gate itself, before its
//! first `LspHub` spawn; otherwise `wait()` passes immediately. The sweep
//! itself logs start / finish / passes / killed / elapsed
//! (`proc_lifetime::sweep_orphan_lsp_processes`).

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Unarmed,
    Running,
    Open,
}

pub struct SweepGate {
    phase: Mutex<Phase>,
    tx: watch::Sender<bool>,
}

impl Default for SweepGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide gate used by `serve` and by every hub that was not given its own.
pub fn global() -> &'static Arc<SweepGate> {
    static GATE: OnceLock<Arc<SweepGate>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(SweepGate::new()))
}

struct OpenOnDrop(Arc<SweepGate>);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

impl SweepGate {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self {
            phase: Mutex::new(Phase::Unarmed),
            tx,
        }
    }

    /// Arm the gate and run `sweep` on a background thread. Only the first
    /// call runs the sweep; later calls are no-ops.
    pub fn start_background(self: &Arc<Self>, sweep: fn()) {
        {
            let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
            if *phase != Phase::Unarmed {
                return;
            }
            *phase = Phase::Running;
        }
        let gate = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("lsp-orphan-sweep".into())
            .spawn(move || run_guarded(gate, sweep));
        if spawned.is_err() {
            tracing::warn!("could not start the orphan sweep thread; sweeping inline");
            run_guarded(Arc::clone(self), sweep);
        }
    }

    /// Wait until the sweep has finished. Returns at once when the gate was
    /// never armed or is already open.
    pub async fn wait(&self) {
        if self.is_passable() {
            return;
        }
        let mut rx = self.tx.subscribe();
        // `wait_for` checks the current value first, so an `open` between
        // `is_passable` and `subscribe` is not missed.
        let _ = rx.wait_for(|open| *open).await;
    }

    #[cfg(test)]
    pub(crate) fn running_for_test() -> Arc<Self> {
        let gate = Self::new();
        *gate.phase.lock().unwrap() = Phase::Running;
        Arc::new(gate)
    }

    #[cfg(test)]
    pub(crate) fn open_for_test(&self) {
        self.open();
    }

    fn is_passable(&self) -> bool {
        let phase = *self.phase.lock().unwrap_or_else(|e| e.into_inner());
        phase != Phase::Running
    }

    fn open(&self) {
        *self.phase.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Open;
        self.tx.send_replace(true);
    }
}

fn run_guarded(gate: Arc<SweepGate>, sweep: fn()) {
    let _open = OpenOnDrop(gate);
    if std::panic::catch_unwind(sweep).is_err() {
        tracing::warn!("orphaned language-server sweep panicked; gate opened");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn wait_passes_when_never_armed() {
        let gate = SweepGate::new();
        tokio::time::timeout(Duration::from_millis(50), gate.wait())
            .await
            .expect("an unarmed gate must not block");
    }

    static RELEASE: Mutex<bool> = Mutex::new(false);
    static RELEASE_CV: std::sync::Condvar = std::sync::Condvar::new();

    fn blocking_sweep() {
        let mut released = RELEASE.lock().unwrap();
        while !*released {
            released = RELEASE_CV.wait(released).unwrap();
        }
    }

    #[tokio::test]
    async fn wait_blocks_until_sweep_returns() {
        let gate = Arc::new(SweepGate::new());
        gate.start_background(blocking_sweep);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), gate.wait())
                .await
                .is_err(),
            "wait must block while the sweep runs"
        );
        *RELEASE.lock().unwrap() = true;
        RELEASE_CV.notify_all();
        tokio::time::timeout(Duration::from_secs(1), gate.wait())
            .await
            .expect("wait must return once the sweep finished");
    }

    fn panicking_sweep() {
        panic!("sweep failed");
    }

    #[tokio::test]
    async fn gate_opens_when_sweep_panics() {
        let gate = Arc::new(SweepGate::new());
        gate.start_background(panicking_sweep);
        tokio::time::timeout(Duration::from_secs(1), gate.wait())
            .await
            .expect("a panicking sweep must still open the gate");
    }

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_sweep() {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn second_start_is_noop_after_open() {
        let gate = Arc::new(SweepGate::new());
        gate.start_background(counting_sweep);
        tokio::time::timeout(Duration::from_secs(1), gate.wait())
            .await
            .expect("open");
        gate.start_background(counting_sweep);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
