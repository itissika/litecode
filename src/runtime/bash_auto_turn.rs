//! Idle auto-turn when a session background bash job exits and a UI is attached.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::PermissionSink;
use crate::session::SessionService;
use crate::terminal::TerminalHub;

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
    hub: &TerminalHub,
    runtime: &RuntimeHandle,
    service: &SessionService,
    workspace_root: &Path,
    session_id: &str,
) -> IdleTurn {
    idle_auto_turn::plan_idle_turn(runtime, service, workspace_root, session_id, || {
        hub.jobs.mailbox_pending(session_id)
    })
}

fn maybe_spawn_idle_auto_turn(
    hub: &TerminalHub,
    runtime: &RwLock<RuntimeHandle>,
    service: &Arc<SessionService>,
    workspace_root: &Path,
    session_id: &str,
) {
    idle_auto_turn::spawn_from_snapshot(runtime, service, "bash", |runtime| {
        plan(hub, runtime, service, workspace_root, session_id)
    });
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
    idle_auto_turn::install_turn_finished_flush(&service, "bash-idle-turn-flush", move |sid| {
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
