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
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::permission::{
        AskOutcome, AskPrompt, CancellingPermissionSink, RecordingPermissionSink,
    };

    struct RecordingObserver {
        events: Mutex<Vec<InternalEvent>>,
    }

    impl RecordingObserver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
            })
        }

        fn snapshot(&self) -> Vec<InternalEvent> {
            self.events.lock().expect("events").clone()
        }
    }

    impl RuntimeObserver for RecordingObserver {
        fn on_internal(&self, ev: InternalEvent) {
            self.events.lock().expect("events").push(ev);
        }
    }

    fn wrap_with(
        inner: Arc<dyn PermissionSink>,
        observer: Arc<RecordingObserver>,
        step: u64,
    ) -> Arc<dyn PermissionSink> {
        PhasePermissionSink::wrap(inner, observer, Arc::new(AtomicU64::new(step)))
    }

    fn prompt() -> AskPrompt<'static> {
        AskPrompt::permission("bash", "rule-shell", "run ls")
    }

    /// T3: Deny leave → PermissionAwaiting{false} + ExecutingTools (leave AwaitingPermission).
    #[test]
    fn t3_deny_leave_executing_tools() {
        let observer = RecordingObserver::new();
        let inner = Arc::new(RecordingPermissionSink::new(AskOutcome::Deny));
        let sink = wrap_with(inner, observer.clone(), 2);

        let reply = sink.ask(&prompt(), &CancellationToken::new());
        assert_eq!(reply.outcome, AskOutcome::Deny);

        let events = observer.snapshot();
        assert!(matches!(
            events[2],
            InternalEvent::PermissionAwaiting { awaiting: false }
        ));
        assert!(
            matches!(
                events[3],
                InternalEvent::PhaseChanged {
                    phase: TurnPhase::ExecutingTools,
                    step: 2,
                }
            ),
            "Deny must leave AwaitingPermission → ExecutingTools, got {:?}",
            events[3]
        );
        // Must not still be AwaitingPermission after leave
        assert!(
            !events.iter().skip(2).any(|e| matches!(
                e,
                InternalEvent::PhaseChanged {
                    phase: TurnPhase::AwaitingPermission { .. },
                    ..
                }
            )),
            "post-leave events must not re-assert AwaitingPermission: {events:?}"
        );
    }

    /// T4: Aborted leave → PermissionAwaiting{false} + Cancelling.
    #[test]
    fn t4_aborted_leave_cancelling() {
        let observer = RecordingObserver::new();
        let inner = Arc::new(RecordingPermissionSink::new(AskOutcome::Aborted));
        let sink = wrap_with(inner, observer.clone(), 4);

        let reply = sink.ask(&prompt(), &CancellationToken::new());
        assert_eq!(reply.outcome, AskOutcome::Aborted);

        let events = observer.snapshot();
        assert!(matches!(
            events[2],
            InternalEvent::PermissionAwaiting { awaiting: false }
        ));
        assert!(
            matches!(
                events[3],
                InternalEvent::PhaseChanged {
                    phase: TurnPhase::Cancelling,
                    step: 4,
                }
            ),
            "Aborted must leave → Cancelling, got {:?}",
            events[3]
        );
    }

    /// T5: CancellingPermissionSink (inner) + cancelled token → Aborted + T4 leave events.
    #[test]
    fn t5_cancelling_wrapper_aborted_leave_cancelling() {
        let observer = RecordingObserver::new();
        let allow_inner = Arc::new(RecordingPermissionSink::new(AskOutcome::Allow {
            always: false,
        }));
        let cancel = CancellationToken::new();
        cancel.cancel();
        // Production order: Phase outer, Cancelling inner (mod.rs wrap chain).
        let cancelling: Arc<dyn PermissionSink> = Arc::new(CancellingPermissionSink::new(
            allow_inner.clone(),
            cancel.clone(),
        ));
        let sink = wrap_with(cancelling, observer.clone(), 5);

        let reply = sink.ask(&prompt(), &cancel);
        assert_eq!(reply.outcome, AskOutcome::Aborted);
        // Cancelling short-circuited — inner Allow sink never called.
        assert_eq!(allow_inner.calls.lock().unwrap().len(), 0);

        let events = observer.snapshot();
        assert!(
            matches!(
                events.get(0),
                Some(InternalEvent::PermissionAwaiting { awaiting: true })
            ),
            "enter bool, got {:?}",
            events.first()
        );
        assert!(
            matches!(
                events.get(1),
                Some(InternalEvent::PhaseChanged {
                    phase: TurnPhase::AwaitingPermission { .. },
                    step: 5,
                })
            ),
            "enter phase, got {:?}",
            events.get(1)
        );
        assert!(matches!(
            events.get(2),
            Some(InternalEvent::PermissionAwaiting { awaiting: false })
        ));
        assert!(
            matches!(
                events.get(3),
                Some(InternalEvent::PhaseChanged {
                    phase: TurnPhase::Cancelling,
                    step: 5,
                })
            ),
            "Aborted leave → Cancelling, got {:?}",
            events.get(3)
        );
    }

}
