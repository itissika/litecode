//! Idle auto-turn when a parent session's background subagent exits and a UI is attached.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::{PermissionSink, deny_permission_sink};
use crate::runtime::{RuntimeHandle, TurnOptions, spawn_turn};
use crate::session::{LifecycleEvent, SessionManager};
use crate::tools::subagent::{SubagentHub, format_exit_reminder};
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
    hub: &SubagentHub,
    runtime: &RuntimeHandle,
    sessions: &SessionManager,
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
    let input = format_exit_reminder(&notices, &jobs);
    let append_result = sessions.append_job_exit(sid, &crate::types::user_text(&input));
    if let Err(error) = append_result {
        tracing::warn!(session_id = sid, %error, "failed to persist subagent exit reminder");
        sessions.release_turn_reservation(sid, &turn_id);
        return IdleAutoTurn::SkippedSessionGone;
    }
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

fn spawn_prepared_idle_auto_turn(
    runtime: &RuntimeHandle,
    sessions: &Arc<SessionManager>,
    decision: IdleAutoTurn,
) {
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
    let sessions = Arc::clone(sessions);
    let handle = match spawn_turn(
        runtime,
        session_id.clone(),
        Arc::clone(&sessions),
        input,
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
            sessions.release_turn_reservation(&session_id, &turn_id);
            return;
        }
    };
    let session_id_err = session_id.clone();
    let turn_id_err = turn_id.clone();
    let sessions_err = Arc::clone(&sessions);
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
                tracing::error!(
                    session_id = %session_id,
                    error = %error,
                    "subagent idle auto-turn start failed"
                );
                sessions.release_turn_reservation(&session_id, &turn_id);
            }
        });
    if let Err(error) = spawn_result {
        tracing::error!(
            session_id = %session_id_err,
            error = %error,
            "failed to spawn subagent idle auto-turn thread"
        );
        sessions_err.release_turn_reservation(&session_id_err, &turn_id_err);
    }
}

fn maybe_spawn_idle_auto_turn(
    hub: &SubagentHub,
    runtime: &RwLock<RuntimeHandle>,
    sessions: &Arc<SessionManager>,
    workspace_root: &Path,
    session_id: &str,
) {
    let runtime_snap = runtime.read().expect("runtime lock").clone();
    let decision =
        try_begin_idle_auto_turn(hub, &runtime_snap, sessions, workspace_root, session_id);
    spawn_prepared_idle_auto_turn(&runtime_snap, sessions, decision);
}

pub fn install_subagent_auto_turn(
    hub: Arc<SubagentHub>,
    runtime: Arc<RwLock<RuntimeHandle>>,
    sessions: Arc<SessionManager>,
    workspace_root: PathBuf,
) {
    let hub_for_jobs = Arc::clone(&hub);
    let runtime_for_exit = Arc::clone(&runtime);
    let sessions_for_exit = Arc::clone(&sessions);
    let root_for_exit = workspace_root.clone();
    hub.set_exit_handler(Arc::new(move |notice| {
        maybe_spawn_idle_auto_turn(
            &hub_for_jobs,
            &runtime_for_exit,
            &sessions_for_exit,
            &root_for_exit,
            &notice.parent_session_id,
        );
    }));

    let hub_for_life = Arc::clone(&hub);
    let runtime_for_life = Arc::clone(&runtime);
    let sessions_for_life = Arc::clone(&sessions);
    let mut rx = sessions.subscribe_lifecycle();
    if let Err(error) = std::thread::Builder::new()
        .name("subagent-idle-turn-flush".into())
        .spawn(move || {
            loop {
                match rx.blocking_recv() {
                    Ok(LifecycleEvent::TurnFinished { session_id, .. }) => {
                        maybe_spawn_idle_auto_turn(
                            &hub_for_life,
                            &runtime_for_life,
                            &sessions_for_life,
                            &workspace_root,
                            &session_id,
                        );
                    }
                    Ok(LifecycleEvent::SessionRemoved { session_id }) => {
                        hub_for_life.forget_child(&session_id);
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
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
    use crate::engines::WorkspaceEngines;
    use crate::ide_base::IdeBaseHandle;
    use crate::optional::EngineManager;
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
        let resolved = resolve(global, workspace_state.clone());
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
        hub.attach_sessions(Arc::clone(&sessions));
        (runtime, sessions, hub)
    }

    #[test]
    fn idle_without_subscribers_does_not_reserve() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        hub.jobs
            .insert_running_for_test(&sid, "child-a", "reviewer", "go");
        hub.jobs.finish("child-a", true, false, "done".into());
        match try_begin_idle_auto_turn(&hub, &runtime, &sessions, dir.path(), &sid) {
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
        hub.jobs
            .insert_running_for_test(&sid, "child-a", "reviewer", "go");
        hub.jobs.finish("child-a", true, false, "done".into());
        match try_begin_idle_auto_turn(&hub, &runtime, &sessions, dir.path(), &sid) {
            IdleAutoTurn::SkippedBusy => {}
            _ => panic!("expected busy"),
        }
        assert!(!hub.jobs.take_mailbox(&sid).is_empty());
    }

    #[test]
    fn idle_with_subscribers_prepares_turn_using_exit_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, hub) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let _ = sessions.attach(&sid);
        hub.jobs
            .insert_running_for_test(&sid, "child-a", "reviewer", "review this");
        hub.jobs.finish("child-a", true, false, "done".into());
        let notice = hub.jobs.notice_snapshot("child-a").expect("notice");
        let expected =
            format_exit_reminder(std::slice::from_ref(&notice), &hub.jobs.running(&sid));
        match try_begin_idle_auto_turn(&hub, &runtime, &sessions, dir.path(), &sid) {
            IdleAutoTurn::Prepared {
                input,
                turn_id,
                session_id,
                ..
            } => {
                assert_eq!(session_id, sid);
                assert_eq!(input, expected);
                assert!(input.starts_with("<system-reminder>"));
                sessions.release_turn_reservation(&sid, &turn_id);
            }
            _ => panic!("expected prepared, got non-prepared variant"),
        }
        assert!(hub.jobs.take_mailbox(&sid).is_empty());
        assert!(!sessions.is_session_busy_blocking(&sid));
    }
}
