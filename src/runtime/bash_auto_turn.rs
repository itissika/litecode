//! Idle auto-turn when a session background bash job exits and a UI is attached.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::PermissionSink;
use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::{LifecycleEvent, SessionService};
use crate::terminal::TerminalHub;
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

/// Decide whether an idle live session should start a turn for mailbox exits.
/// On `Prepared`, the turn is reserved. The mailbox stays pending so the first
/// request seam can write `reminder/bash_exit`.
pub fn try_begin_idle_auto_turn(
    hub: &TerminalHub,
    runtime: &RuntimeHandle,
    service: &SessionService,
    workspace_root: &Path,
    session_id: &str,
) -> IdleAutoTurn {
    let sid = session_id;
    if sid.is_empty() || sid == "_" {
        return IdleAutoTurn::SkippedNoUi;
    }
    if !hub.jobs.mailbox_pending(sid) {
        return IdleAutoTurn::SkippedEmptyMailbox;
    }
    if service.subscriber_count_blocking(sid) == 0 {
        return IdleAutoTurn::SkippedNoUi;
    }
    if service.is_session_busy_blocking(sid) {
        return IdleAutoTurn::SkippedBusy;
    }

    let default_primary = runtime.desired_primary_agent();
    let primary_agent = match service.resolve_primary_agent_for_session(runtime, sid, default_primary)
    {
        Ok(id) => id,
        Err(_) => return IdleAutoTurn::SkippedSessionGone,
    };
    let project = service
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::bot::step_max_for_session(
        &runtime.resolved,
        runtime.bots().map(|bots| bots.as_ref()),
        sid,
        &primary_agent,
    );
    let turn_id = uuid::Uuid::new_v4().to_string();
    match service.reserve_auto_turn(sid, turn_id.clone(), step_max, &primary_agent, &project) {
        Ok(_) => {}
        Err(LitecodeError::AgentAlreadyRunning) => return IdleAutoTurn::SkippedBusy,
        Err(_) => return IdleAutoTurn::SkippedSessionGone,
    }

    if !hub.jobs.mailbox_pending(sid) {
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
            tracing::warn!(error = %error, "bash idle auto-turn spawn failed");
            service.release_auto_turn(&session_id, &turn_id);
            return;
        }
    };
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(error) => {
                tracing::warn!(error = %error, "bash idle auto-turn runtime failed");
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
            tracing::warn!(error = %error, "bash idle auto-turn start failed");
            service.release_auto_turn(&session_id, &turn_id);
        }
    });
}

fn maybe_spawn_idle_auto_turn(
    hub: &TerminalHub,
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

pub fn install_idle_auto_turn(
    hub: Arc<TerminalHub>,
    runtime: Arc<RwLock<RuntimeHandle>>,
    service: Arc<SessionService>,
    workspace_root: PathBuf,
) {
    let hub_for_jobs = Arc::clone(&hub);
    let runtime_for_exit = Arc::clone(&runtime);
    let service_for_exit = Arc::clone(&service);
    let root_for_exit = workspace_root.clone();
    hub.jobs.set_exit_handler(Arc::new(move |notice| {
        maybe_spawn_idle_auto_turn(
            &hub_for_jobs,
            &runtime_for_exit,
            &service_for_exit,
            &root_for_exit,
            &notice.session_id,
        );
    }));

    let hub_for_life = Arc::clone(&hub);
    let runtime_for_life = Arc::clone(&runtime);
    let service_for_life = Arc::clone(&service);
    let mut rx = service.subscribe_lifecycle();
    let _ = std::thread::Builder::new()
        .name("bash-idle-turn-flush".into())
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
    ) -> (RuntimeHandle, Arc<SessionManager>, Arc<TerminalHub>) {
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
        let hub = Arc::new(TerminalHub::new());
        let ide = IdeBaseHandle::new(workspace, Arc::clone(&engines), Arc::clone(&hub));
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
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            root.join("sessions.db").to_string_lossy().to_string(),
        ));
        (runtime, sessions, hub)
    }

    fn test_service(sessions: &Arc<SessionManager>) -> Arc<SessionService> {
        Arc::new(SessionService::new(Arc::clone(sessions)))
    }

    fn finish_echo(hub: &TerminalHub, root: &std::path::Path, sid: &str) {
        hub.jobs
            .insert_finished_pipe_job("bg_ec000001", sid, "", "echo hi", root, Some(0), false)
            .expect("finished pipe job");
    }

    fn finish_user_killed(hub: &TerminalHub, root: &std::path::Path, sid: &str) {
        hub.jobs
            .insert_finished_pipe_job("bg_ec000002", sid, "", "sleep 20", root, Some(1), true)
            .expect("user-killed pipe job");
    }

    #[test]
    fn idle_without_subscribers_does_not_reserve() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let service = test_service(&sessions);
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        finish_echo(&hub, dir.path(), &sid);
        match try_begin_idle_auto_turn(&hub, &runtime, &service, dir.path(), &sid) {
            IdleAutoTurn::SkippedNoUi => {}
            _ => panic!("expected no UI"),
        }
        assert!(!sessions.is_session_busy_blocking(&sid));
        assert!(!hub.jobs.take_mailbox(&sid).is_empty());
    }

    #[test]
    fn busy_session_leaves_mailbox() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let service = test_service(&sessions);
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let _ = sessions.attach(&sid);
        sessions
            .reserve_turn(
                &sid,
                "turn-busy".into(),
                10,
                "default",
                &dir.path().display().to_string(),
            )
            .unwrap();
        finish_echo(&hub, dir.path(), &sid);
        match try_begin_idle_auto_turn(&hub, &runtime, &service, dir.path(), &sid) {
            IdleAutoTurn::SkippedBusy => {}
            _ => panic!("expected busy"),
        }
        assert!(!hub.jobs.take_mailbox(&sid).is_empty());
    }

    #[test]
    fn ui_kill_while_busy_prepares_user_stopped_reminder_once_idle() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let service = test_service(&sessions);
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let _ = sessions.attach(&sid);
        sessions
            .reserve_turn(
                &sid,
                "turn-busy".into(),
                10,
                "default",
                &dir.path().display().to_string(),
            )
            .unwrap();
        finish_user_killed(&hub, dir.path(), &sid);
        match try_begin_idle_auto_turn(&hub, &runtime, &service, dir.path(), &sid) {
            IdleAutoTurn::SkippedBusy => {}
            _ => panic!("expected busy while turn reserved"),
        }
        assert!(hub.jobs.mailbox_pending(&sid));
        sessions.release_turn_reservation(&sid, "turn-busy");
        match try_begin_idle_auto_turn(&hub, &runtime, &service, dir.path(), &sid) {
            IdleAutoTurn::Prepared { turn_id, .. } => {
                assert!(hub.jobs.mailbox_pending(&sid));
                sessions.release_turn_reservation(&sid, &turn_id);
            }
            _ => panic!("expected prepared after idle"),
        }
    }
}
