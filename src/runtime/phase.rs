use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio_util::sync::CancellationToken;

use crate::permission::{AskOutcome, PermissionSink};
use crate::runtime::observer::{InternalEvent, RuntimeObserver, TurnPhase};

use super::AgentRuntime;

/// Wraps a permission sink so L1 phase tracks permission wait enter/leave.
///
/// Enter: `PermissionAwaiting{true}` + `PhaseChanged(AwaitingPermission{…})`.
/// Leave: `PermissionAwaiting{false}` + leave phase by outcome
/// (Allow/Deny → `ExecutingTools`, Aborted → `Cancelling`).
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

    fn emit_phase(&self, phase: TurnPhase) {
        let step = self.step.load(Ordering::Relaxed);
        self.observer
            .on_internal(InternalEvent::PhaseChanged { phase, step });
    }

    fn leave_phase(&self, outcome: &AskOutcome) {
        self.observer
            .on_internal(InternalEvent::PermissionAwaiting { awaiting: false });
        match outcome {
            AskOutcome::Allow { .. } | AskOutcome::Deny => {
                self.emit_phase(TurnPhase::ExecutingTools);
            }
            AskOutcome::Aborted => {
                self.emit_phase(TurnPhase::Cancelling);
            }
        }
    }
}

impl PermissionSink for PhasePermissionSink {
    fn ask(
        &self,
        prompt: &crate::permission::AskPrompt<'_>,
        cancel: &CancellationToken,
    ) -> crate::permission::AskReply {
        // enter: bool + authoritative waiting phase (do not wait for PermissionAsk)
        self.observer
            .on_internal(InternalEvent::PermissionAwaiting { awaiting: true });
        self.emit_phase(TurnPhase::AwaitingPermission {
            tool: prompt.tool.into(),
            rule_id: prompt.rule_id.into(),
            summary: prompt.summary.into(),
        });

        let reply = self.inner.ask(prompt, cancel);

        // leave: bool false + leave AwaitingPermission by outcome
        self.leave_phase(&reply.outcome);
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
        // WP-188 observe-only: TLS step + last-turn heartbeat (does not touch PhasePermissionSink).
        crate::runtime::diagnostics::DiagContext::update_step(step);
        let phase_label = crate::runtime::diagnostics::turn_phase_label(&phase);
        self.emit_internal(InternalEvent::PhaseChanged { phase, step });
        let data_root = self.sessions.data_root_path();
        let turn_id = crate::runtime::diagnostics::DiagContext::snapshot()
            .map(|s| s.turn_id)
            .unwrap_or_default();
        crate::runtime::diagnostics::heartbeat_last_turn(
            &data_root,
            &self.session_id,
            &turn_id,
            step,
            &phase_label,
            None,
        );
    }

    pub(crate) fn emit_step_started(&self, step: u64) {
        self.emit_internal(InternalEvent::StepStarted {
            step,
            step_max: self.agent_config.max_steps,
        });
    }
}

#[cfg(test)]
mod tests {}
