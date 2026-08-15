//! Idle auto-turn when a session background bash job exits and a UI is attached.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::permission::{PermissionSink, deny_permission_sink};
use crate::runtime::{RuntimeHandle, spawn_turn};
use crate::session::SessionManager;
use crate::terminal::{ExitNotice, TerminalHub};
use crate::tools::bash_status;
use crate::types::LitecodeError;

pub enum IdleAutoTurn {
    Prepared {
        session_id: String,
        turn_id: String,
        primary_agent: String,
        project: String,
        input: String,
        sink: Arc<dyn PermissionSink>,
    },
    SkippedBusy,
    SkippedNoUi,
    SkippedEmptyMailbox,
    SkippedSessionGone,
}

/// Decide whether an idle live session should start a turn for mailbox exits.
/// On `Prepared`, the turn is reserved and the mailbox is drained.
pub fn try_begin_idle_auto_turn(
    hub: &TerminalHub,
    runtime: &RuntimeHandle,
    sessions: &SessionManager,
    workspace_root: &std::path::Path,
    notice: &ExitNotice,
) -> IdleAutoTurn {
    let sid = notice.session_id.as_str();
    if sid.is_empty() || sid == "_" {
        return IdleAutoTurn::SkippedNoUi;
    }
    if sessions.subscriber_count_blocking(sid) == 0 {
        return IdleAutoTurn::SkippedNoUi;
    }
    if sessions.is_session_busy_blocking(sid) {
        return IdleAutoTurn::SkippedBusy;
    }

    let default_primary = runtime.desired_primary_agent();
    let primary_agent =
        match sessions.resolve_primary_agent(sid, default_primary, &runtime.resolved) {
            Ok(id) => id,
            Err(_) => return IdleAutoTurn::SkippedSessionGone,
        };
    let project = sessions
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::config::bridge::agent_config_for(&runtime.resolved, &primary_agent)
        .map(|a| a.max_steps)
        .unwrap_or(50);
    let turn_id = uuid::Uuid::new_v4().to_string();
    match sessions.reserve_turn(sid, turn_id.clone(), step_max, &primary_agent, &project) {
        Ok(_) => {}
        Err(LitecodeError::AgentAlreadyRunning) => return IdleAutoTurn::SkippedBusy,
        Err(_) => return IdleAutoTurn::SkippedSessionGone,
    }

    let notices = hub.jobs.take_mailbox(sid);
    if notices.is_empty() {
        sessions.release_turn_reservation(sid, &turn_id);
        return IdleAutoTurn::SkippedEmptyMailbox;
    }
    let jobs = hub.jobs.running(sid);
    let input = bash_status::format_exit_reminder(&notices, &jobs, workspace_root);
    let sink = sessions
        .last_permission_sink(sid)
        .unwrap_or_else(deny_permission_sink);
    IdleAutoTurn::Prepared {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        input,
        sink,
    }
}

pub fn install_idle_auto_turn(
    hub: Arc<TerminalHub>,
    runtime: Arc<RwLock<RuntimeHandle>>,
    sessions: Arc<SessionManager>,
    workspace_root: PathBuf,
) {
    let hub_for_jobs = Arc::clone(&hub);
    hub.jobs.set_exit_handler(Arc::new(move |notice| {
        let runtime_snap = runtime.read().expect("runtime lock").clone();
        let decision = try_begin_idle_auto_turn(
            &hub_for_jobs,
            &runtime_snap,
            &sessions,
            &workspace_root,
            &notice,
        );
        let IdleAutoTurn::Prepared {
            session_id,
            turn_id,
            primary_agent,
            project,
            input,
            sink,
        } = decision
        else {
            return;
        };
        let sessions = Arc::clone(&sessions);
        let handle = match spawn_turn(
            &runtime_snap,
            session_id.clone(),
            Arc::clone(&sessions),
            input,
            sink,
            turn_id.clone(),
        ) {
            Ok(h) => h,
            Err(error) => {
                tracing::warn!(error = %error, "bash idle auto-turn spawn failed");
                sessions.release_turn_reservation(&session_id, &turn_id);
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
                    sessions.release_turn_reservation(&session_id, &turn_id);
                    return;
                }
            };
            if let Err(error) = rt.block_on(sessions.start_turn(
                &session_id,
                handle,
                &primary_agent,
                &project,
                Arc::clone(&sessions),
            )) {
                tracing::warn!(error = %error, "bash idle auto-turn start failed");
                sessions.release_turn_reservation(&session_id, &turn_id);
            }
        });
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
    use crate::engines::WorkspaceEngines;
    use crate::ide_base::IdeBaseHandle;
    use crate::optional::EngineManager;
    use crate::session::store::Session;
    use crate::workspace::WorkspaceService;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

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
        let resolved = resolve(global, workspace_state.clone());
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
        let sessions = Arc::new(SessionManager::new(
            Arc::new(TurnGuard::new()),
            root.join("sessions.db").to_string_lossy().to_string(),
        ));
        (runtime, sessions, hub)
    }

    fn wait_job_exit(hub: &TerminalHub, id: &str) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            if hub.jobs.get(id).is_some_and(|(alive, _, _, _)| !alive) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("job {id} did not exit");
    }

    fn spawn_echo(hub: &TerminalHub, root: &std::path::Path, sid: &str) -> String {
        let cmd = if cfg!(windows) { "echo hi" } else { "echo hi" };
        hub.spawn_command(cmd, None, root, sid).expect("spawn").id
    }

    #[test]
    fn idle_without_subscribers_does_not_reserve() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let session =
            Session::ephemeral(&dir.path().display().to_string(), "default", None).unwrap();
        let sid = session.id.clone();
        sessions.register_for_test(session);
        let id = spawn_echo(&hub, dir.path(), &sid);
        wait_job_exit(&hub, &id);
        let notice = ExitNotice {
            bash_id: id,
            session_id: sid.clone(),
            exit_code: Some(0),
            output_path: dir.path().join("x.output"),
            command_preview: "echo hi".into(),
        };
        match try_begin_idle_auto_turn(&hub, &runtime, &sessions, dir.path(), &notice) {
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
        let session =
            Session::ephemeral(&dir.path().display().to_string(), "default", None).unwrap();
        let sid = session.id.clone();
        sessions.register_for_test(session);
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
        let id = spawn_echo(&hub, dir.path(), &sid);
        wait_job_exit(&hub, &id);
        let notice = ExitNotice {
            bash_id: id,
            session_id: sid.clone(),
            exit_code: Some(0),
            output_path: dir.path().join("x.output"),
            command_preview: "echo hi".into(),
        };
        match try_begin_idle_auto_turn(&hub, &runtime, &sessions, dir.path(), &notice) {
            IdleAutoTurn::SkippedBusy => {}
            _ => panic!("expected busy"),
        }
        assert!(!hub.jobs.take_mailbox(&sid).is_empty());
    }

    #[test]
    fn idle_with_subscribers_prepares_turn_using_exit_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let session =
            Session::ephemeral(&dir.path().display().to_string(), "default", None).unwrap();
        let sid = session.id.clone();
        sessions.register_for_test(session);
        let _ = sessions.attach(&sid);
        let id = spawn_echo(&hub, dir.path(), &sid);
        wait_job_exit(&hub, &id);
        let notice = hub.jobs.notice_snapshot(&id).expect("notice");
        let expected = bash_status::format_exit_reminder(
            std::slice::from_ref(&notice),
            &hub.jobs.running(&sid),
            dir.path(),
        );
        match try_begin_idle_auto_turn(&hub, &runtime, &sessions, dir.path(), &notice) {
            IdleAutoTurn::Prepared {
                input,
                turn_id,
                session_id,
                ..
            } => {
                assert_eq!(session_id, sid);
                assert_eq!(input, expected);
                assert!(input.starts_with("<system-reminder>"));
                assert!(!input.contains("status: exited"));
                sessions.release_turn_reservation(&sid, &turn_id);
            }
            _ => panic!("expected prepared, got non-prepared variant"),
        }
        assert!(hub.jobs.take_mailbox(&sid).is_empty());
        assert!(!sessions.is_session_busy_blocking(&sid));
    }
}
