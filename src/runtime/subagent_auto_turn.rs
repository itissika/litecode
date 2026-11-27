//! Idle auto-turn when a parent session's background subagent exits and a UI is attached.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::PermissionSink;
use crate::session::SessionService;
use crate::tools::subagent::SubagentHub;

use super::idle_auto_turn::{self, IdleTurn};
use super::RuntimeHandle;

pub enum IdleAutoTurn {
    Prepared {
        session_id: String,
        turn_id: String,
        primary_agent: String,
        project: String,
        sink: Arc<dyn PermissionSink>,
    },
    SkippedBusy,
    SkippedNoUi,
    SkippedEmptyMailbox,
    SkippedSessionGone,
}

/// Decide whether an idle live session should start a turn for settled children.
/// On `Prepared`, the turn is reserved. Completions stay pending so the first
/// request seam can write `reminder/subagent_settled`.
pub fn try_begin_idle_auto_turn(
    hub: &SubagentHub,
    runtime: &RuntimeHandle,
    service: &SessionService,
    workspace_root: &Path,
    session_id: &str,
) -> IdleAutoTurn {
    match plan(hub, runtime, service, workspace_root, session_id) {
        IdleTurn::Prepared(prepared) => IdleAutoTurn::Prepared {
            session_id: prepared.session_id,
            turn_id: prepared.turn_id,
            primary_agent: prepared.primary_agent,
            project: prepared.project,
            sink: prepared.sink,
        },
        IdleTurn::SkippedBusy => IdleAutoTurn::SkippedBusy,
        IdleTurn::SkippedNoUi => IdleAutoTurn::SkippedNoUi,
        IdleTurn::SkippedEmptyMailbox => IdleAutoTurn::SkippedEmptyMailbox,
        IdleTurn::SkippedSessionGone => IdleAutoTurn::SkippedSessionGone,
    }
}

/// This listener's mailbox probe, plugged into the shared decision order.
fn plan(
    hub: &SubagentHub,
    runtime: &RuntimeHandle,
    service: &SessionService,
    workspace_root: &Path,
    session_id: &str,
) -> IdleTurn {
    idle_auto_turn::plan_idle_turn(runtime, service, workspace_root, session_id, || {
        hub.has_pending(session_id)
    })
}

fn maybe_spawn_idle_auto_turn(
    hub: &SubagentHub,
    runtime: &RwLock<RuntimeHandle>,
    service: &Arc<SessionService>,
    workspace_root: &Path,
    session_id: &str,
) {
    idle_auto_turn::spawn_from_snapshot(runtime, service, "subagent", |runtime| {
        plan(hub, runtime, service, workspace_root, session_id)
    });
}

pub fn install_subagent_auto_turn(
    hub: Arc<SubagentHub>,
    runtime: Arc<RwLock<RuntimeHandle>>,
    service: Arc<SessionService>,
    workspace_root: PathBuf,
) {
    let hub_for_completion = Arc::clone(&hub);
    let runtime_for_exit = Arc::clone(&runtime);
    let service_for_exit = Arc::clone(&service);
    let root_for_exit = workspace_root.clone();
    hub.set_exit_handler(Arc::new(move |completion| {
        maybe_spawn_idle_auto_turn(
            &hub_for_completion,
            &runtime_for_exit,
            &service_for_exit,
            &root_for_exit,
            &completion.parent_session_id,
        );
    }));

    let hub_for_life = Arc::clone(&hub);
    let runtime_for_life = Arc::clone(&runtime);
    let service_for_life = Arc::clone(&service);
    idle_auto_turn::install_turn_finished_flush(&service, "subagent-idle-turn-flush", move |sid| {
        maybe_spawn_idle_auto_turn(
            &hub_for_life,
            &runtime_for_life,
            &service_for_life,
            &workspace_root,
            sid,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
    use crate::engines::WorkspaceEngines;
    use crate::ide_base::IdeBaseHandle;
    use crate::optional::EngineManager;
    use crate::session::SessionManager;
    use crate::workspace::WorkspaceService;
    use std::sync::atomic::AtomicU64;

    fn test_runtime(
        root: &std::path::Path,
    ) -> (RuntimeHandle, Arc<SessionManager>, Arc<SubagentHub>) {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                ..Default::default()
            },
        );
        let workspace_state = WorkspaceState::new(root);
        let resolved = resolve_without_catalog(global, workspace_state.clone());
        let workspace = WorkspaceService::new(root.to_path_buf()).unwrap();
        let engines = Arc::new(WorkspaceEngines::new());
        let ide = IdeBaseHandle::new(
            workspace,
            Arc::clone(&engines),
            Arc::new(crate::terminal::TerminalHub::new()),
        );
        let runtime = RuntimeHandle::new(
            resolved,
            "default".into(),
            workspace_state,
            Arc::new(EngineManager::new()),
            engines,
            ide,
            Arc::new(AtomicU64::new(0)),
            root.join("global.db"),
        );
        let hub = Arc::clone(&runtime.subagent_hub);
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            root.join("sessions.db").to_string_lossy().to_string(),
        ));
        hub.attach_service(Arc::new(crate::session::SessionService::new(Arc::clone(
            &sessions,
        ))));
        (runtime, sessions, hub)
    }

    #[test]
    fn child_uses_deny_sink_regardless_of_last_sink() {
        use crate::permission::grants::AskPrompt;
        use crate::permission::{AskOutcome, RecordingPermissionSink};
        let dir = tempfile::tempdir().unwrap();
        let (_runtime, sessions, _hub) = test_runtime(dir.path());
        let service = Arc::new(SessionService::new(Arc::clone(&sessions)));
        let project = dir.path().display().to_string();
        let parent = sessions
            .open_session_sync(&project, "default", None)
            .unwrap();
        let child = sessions
            .test_child_session(&project, &parent, "call-1")
            .unwrap();
        let allow: Arc<dyn PermissionSink> = Arc::new(RecordingPermissionSink::new(
            AskOutcome::from_reply(true, false),
        ));
        sessions.set_last_permission_sink(&child, Arc::clone(&allow));
        sessions.set_last_permission_sink(&parent, allow);
        let cancel = tokio_util::sync::CancellationToken::new();
        let prompt = AskPrompt::permission("bash", "r", "s");
        let child_sink = service.auto_turn_permission_sink(&child);
        assert_eq!(child_sink.ask(&prompt, &cancel).outcome, AskOutcome::Deny);
        let parent_sink = service.auto_turn_permission_sink(&parent);
        assert_ne!(
            parent_sink.ask(&prompt, &cancel).outcome,
            AskOutcome::Deny,
            "primary keeps last sink (Q1)"
        );
        // try_begin_idle_auto_turn goes through the same helper (see sink selection in this module).
    }
}
