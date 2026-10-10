use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio_util::sync::CancellationToken;

use crate::permission::PermissionSink;
use crate::runtime::observer::{InternalEvent, RuntimeObserver, TurnPhase};

use super::AgentRuntime;

/// Wraps a permission sink so L1 emits `PhaseChanged(ExecutingTools)` after approval.
pub(crate) struct PhasePermissionSink {
    inner: Arc<dyn PermissionSink>,
    observer: Arc<dyn RuntimeObserver>,
    step: Arc<AtomicU64>,
}

impl PhasePermissionSink {
    pub fn wrap(
        inner: Arc<dyn PermissionSink>,
        observer: Arc<dyn RuntimeObserver>,
        step: Arc<AtomicU64>,
    ) -> Arc<dyn PermissionSink> {
        Arc::new(Self {
            inner,
            observer,
            step,
        })
    }
}

impl PermissionSink for PhasePermissionSink {
    fn ask(
        &self,
        prompt: &crate::permission::AskPrompt<'_>,
        cancel: &CancellationToken,
    ) -> crate::permission::AskReply {
        self.observer
            .on_internal(InternalEvent::PermissionAwaiting { awaiting: true });
        let reply = self.inner.ask(prompt, cancel);
        self.observer
            .on_internal(InternalEvent::PermissionAwaiting { awaiting: false });
        if matches!(reply.outcome, crate::permission::AskOutcome::Allow { .. }) {
            let step = self.step.load(Ordering::Relaxed);
            self.observer.on_internal(InternalEvent::PhaseChanged {
                phase: TurnPhase::ExecutingTools,
                step,
            });
        }
        reply
    }
}

impl AgentRuntime {
    pub(crate) fn set_current_step(&self, step: u64) {
        self.current_step.store(step, Ordering::Relaxed);
    }

    pub(crate) fn current_step_value(&self) -> u64 {
        self.current_step.load(Ordering::Relaxed)
    }

    pub(crate) fn emit_phase(&self, phase: TurnPhase, step: u64) {
        self.emit_internal(InternalEvent::PhaseChanged { phase, step });
    }

    pub(crate) fn emit_step_started(&self, step: u64) {
        self.emit_internal(InternalEvent::StepStarted {
            step,
            step_max: self.agent_config.max_steps,
        });
    }
}
