//! Idle auto-turn when a parent session's background subagent exits and a UI is attached.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::PermissionSink;
use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::{LifecycleEvent, SessionService};
use crate::tools::subagent::SubagentHub;
use crate::types::LitecodeError;

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
    let sid = session_id;
    if sid.is_empty() || sid == "_" {
        return IdleAutoTurn::SkippedNoUi;
    }
    if !hub.has_pending(sid) {
        return IdleAutoTurn::SkippedEmptyMailbox;
    }
    if service.subscriber_count_blocking(sid) == 0 {
        return IdleAutoTurn::SkippedNoUi;
    }
    if service.is_session_busy_blocking(sid) {
        return IdleAutoTurn::SkippedBusy;
    }

    let default_primary = runtime.desired_primary_agent();
    let primary_agent = match service.resolve_primary_agent(sid, default_primary, &runtime.resolved)
    {
        Ok(id) => id,
        Err(_) => return IdleAutoTurn::SkippedSessionGone,
    };
    let project = service
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::config::bridge::agent_config_for(&runtime.resolved, &primary_agent)
        .map(|a| a.max_steps)
        .unwrap_or(50);
    let turn_id = uuid::Uuid::new_v4().to_string();
    match service.reserve_auto_turn(sid, turn_id.clone(), step_max, &primary_agent, &project) {
        Ok(_) => {}
        Err(LitecodeError::AgentAlreadyRunning) => return IdleAutoTurn::SkippedBusy,
        Err(_) => return IdleAutoTurn::SkippedSessionGone,
    }

    if !hub.has_pending(sid) {
        service.release_auto_turn(sid, &turn_id);
        return IdleAutoTurn::SkippedEmptyMailbox;
    }
    let sink = service.auto_turn_permission_sink(sid);
    IdleAutoTurn::Prepared {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        sink,
    }
}

fn spawn_prepared_idle_auto_turn(
    runtime: &RuntimeHandle,
    service: &Arc<SessionService>,
    decision: IdleAutoTurn,
) {
    let IdleAutoTurn::Prepared {
        session_id,
        turn_id,
        primary_agent,
        project,
        sink,
    } = decision
    else {
        return;
    };
    let service = Arc::clone(service);
    let handle = match service.spawn_auto_turn(
        runtime,
        session_id.clone(),
        crate::runtime::TurnInput::Wake {
            anchor_user_seq: None,
        },
        sink,
        turn_id.clone(),
        TurnOptions::default(),
    ) {
        Ok(h) => h,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "subagent idle auto-turn spawn failed"
            );
            service.release_auto_turn(&session_id, &turn_id);
            return;
        }
    };
    let session_id_err = session_id.clone();
    let turn_id_err = turn_id.clone();
    let service_err = Arc::clone(&service);
    let spawn_result = std::thread::Builder::new()
        .name(format!("subagent-idle-{session_id}"))
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(error) => {
                    tracing::error!(
                        session_id = %session_id,
                        error = %error,
                        "subagent idle auto-turn runtime failed"
                    );
                    service.release_auto_turn(&session_id, &turn_id);
                    return;
                }
            };
            if let Err(error) = rt.block_on(service.start_auto_turn_with_handle(
                &session_id,
                handle,
                &primary_agent,
                &project,
            )) {
                tracing::error!(
                    session_id = %session_id,
                    error = %error,
                    "subagent idle auto-turn start failed"
                );
                service.release_auto_turn(&session_id, &turn_id);
            }
        });
    if let Err(error) = spawn_result {
        tracing::error!(
            session_id = %session_id_err,
            error = %error,
            "failed to spawn subagent idle auto-turn thread"
        );
        service_err.release_auto_turn(&session_id_err, &turn_id_err);
    }
}

fn maybe_spawn_idle_auto_turn(
    hub: &SubagentHub,
    runtime: &RwLock<RuntimeHandle>,
    service: &Arc<SessionService>,
    workspace_root: &Path,
    session_id: &str,
) {
    let runtime_snap = runtime.read().expect("runtime lock").clone();
    let decision =
        try_begin_idle_auto_turn(hub, &runtime_snap, service, workspace_root, session_id);
    spawn_prepared_idle_auto_turn(&runtime_snap, service, decision);
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
    let mut rx = service.subscribe_lifecycle();
    if let Err(error) = std::thread::Builder::new()
        .name("subagent-idle-turn-flush".into())
        .spawn(move || {
            loop {
                match rx.blocking_recv() {
                    Ok(LifecycleEvent::TurnFinished { session_id, .. }) => {
                        maybe_spawn_idle_auto_turn(
                            &hub_for_life,
                            &runtime_for_life,
                            &service_for_life,
                            &workspace_root,
                            &session_id,
                        );
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    {
        tracing::error!(
            error = %error,
            "failed to spawn subagent idle-turn-flush thread"
        );
    }
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
