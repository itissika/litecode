//! Idle flush for queued user messages when a turn ends with the queue still
//! holding them.
//!
//! A message queued mid-turn is normally consumed at the next request seam
//! (`inject_background_reminders`). If the turn ends first — cancel, clean
//! completion, error, step limit — the queue must still be delivered: this
//! listener starts the follow-up turn on the user's behalf, exactly like the
//! bash / subagent auto-turns, but without their UI-subscriber gate: the
//! message came from the user, so it is delivered even if nobody is watching.
//!
//! Ordering vs the other flushes: all three race on `reserve_turn` /
//! `reserve_turn_and_claim_pending`, which is the single CAS boundary. A loser
//! simply does nothing, and the winner's turn consumes the queue at its first
//! seam.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::permission::{PermissionSink, deny_permission_sink};
use crate::runtime::{RuntimeHandle, TurnOptions, spawn_turn};
use crate::session::{LifecycleEvent, SessionManager};
use crate::types::LitecodeError;

pub enum PendingFlush {
    Prepared {
        session_id: String,
        turn_id: String,
        primary_agent: String,
        project: String,
        input: String,
        sink: Arc<dyn PermissionSink>,
    },
    SkippedBusy,
    SkippedEmpty,
    SkippedSessionGone,
}

/// Decide whether an idle session should start a turn for its queued messages.
///
/// On `Prepared`, the turn is reserved, the queue drained, and the merged
/// message already appended as one ordinary `item/user` row — the spawn then
/// dedupes against it by text (`already_last_user`), so a spawn failure leaves
/// the message durable instead of evaporating with the reservation.
pub fn try_begin_pending_flush(
    runtime: &RuntimeHandle,
    sessions: &SessionManager,
    workspace_root: &Path,
    session_id: &str,
) -> PendingFlush {
    let sid = session_id;
    if sid.is_empty() || sid == "_" {
        return PendingFlush::SkippedSessionGone;
    }
    if !sessions.has_pending_messages(sid) {
        return PendingFlush::SkippedEmpty;
    }

    let default_primary = runtime.desired_primary_agent();
    let primary_agent =
        match sessions.resolve_primary_agent(sid, default_primary, &runtime.resolved) {
            Ok(id) => id,
            Err(_) => return PendingFlush::SkippedSessionGone,
        };
    let project = sessions
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::config::bridge::agent_config_for(&runtime.resolved, &primary_agent)
        .map(|a| a.max_steps)
        .unwrap_or(50);
    let turn_id = uuid::Uuid::new_v4().to_string();
    let claimed = match sessions.reserve_turn_and_claim_pending(
        sid,
        turn_id.clone(),
        step_max,
        &primary_agent,
        &project,
    ) {
        Ok(Some((_progress, claimed))) => claimed,
        Ok(None) => return PendingFlush::SkippedEmpty,
        Err(LitecodeError::AgentAlreadyRunning) => return PendingFlush::SkippedBusy,
        Err(_) => return PendingFlush::SkippedSessionGone,
    };

    let input = claimed
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if let Err(error) = sessions.append_user_message(sid, &input) {
        tracing::warn!(session_id = sid, %error, "failed to persist pending messages");
        sessions.restore_pending_messages(sid, claimed);
        sessions.release_turn_reservation(sid, &turn_id);
        return PendingFlush::SkippedSessionGone;
    }

    let sink = sessions
        .last_permission_sink(sid)
        .unwrap_or_else(deny_permission_sink);
    PendingFlush::Prepared {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        input,
        sink,
    }
}

fn spawn_prepared_pending_flush(
    runtime: &RuntimeHandle,
    sessions: &Arc<SessionManager>,
    decision: PendingFlush,
) {
    let PendingFlush::Prepared {
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
            tracing::warn!(error = %error, "pending flush spawn failed");
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
                tracing::warn!(error = %error, "pending flush runtime failed");
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
            tracing::warn!(error = %error, "pending flush start failed");
            sessions.release_turn_reservation(&session_id, &turn_id);
        }
    });
}

fn maybe_spawn_pending_flush(
    runtime: &RwLock<RuntimeHandle>,
    sessions: &Arc<SessionManager>,
    workspace_root: &Path,
    session_id: &str,
) {
    let runtime_snap = runtime.read().expect("runtime lock").clone();
    let decision =
        try_begin_pending_flush(&runtime_snap, sessions, workspace_root, session_id);
    spawn_prepared_pending_flush(&runtime_snap, sessions, decision);
}

pub fn install_pending_flush(
    runtime: Arc<RwLock<RuntimeHandle>>,
    sessions: Arc<SessionManager>,
    workspace_root: PathBuf,
) {
    let mut rx = sessions.subscribe_lifecycle();
    let _ = std::thread::Builder::new()
        .name("pending-flush".into())
        .spawn(move || {
            loop {
                match rx.blocking_recv() {
                    Ok(LifecycleEvent::TurnFinished { session_id, .. }) => {
                        maybe_spawn_pending_flush(
                            &runtime,
                            &sessions,
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
    use crate::workspace::WorkspaceService;
    use std::sync::atomic::AtomicU64;

    fn test_runtime(
        root: &std::path::Path,
    ) -> (RuntimeHandle, Arc<SessionManager>) {
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
        let hub = Arc::new(crate::terminal::TerminalHub::new());
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
        (runtime, sessions)
    }

    #[test]
    fn empty_queue_prepares_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::SkippedEmpty => {}
            _ => panic!("expected empty"),
        }
        assert!(!sessions.is_session_busy_blocking(&sid));
    }

    #[test]
    fn busy_session_leaves_the_queue_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        sessions
            .reserve_turn(
                &sid,
                "turn-busy".into(),
                10,
                "default",
                &dir.path().display().to_string(),
            )
            .unwrap();
        sessions.enqueue_pending_message(&sid, "steer").unwrap();
        match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::SkippedBusy => {}
            _ => panic!("expected busy"),
        }
        assert_eq!(sessions.pending_messages_snapshot(&sid).len(), 1);
        sessions.release_turn_reservation(&sid, "turn-busy");
    }

    #[test]
    fn idle_flush_reserves_claims_and_persists_one_user_row() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        sessions.enqueue_pending_message(&sid, "first").unwrap();
        sessions.enqueue_pending_message(&sid, "second").unwrap();
        match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::Prepared {
                input,
                turn_id,
                session_id,
                ..
            } => {
                assert_eq!(session_id, sid);
                assert_eq!(input, "first\n\nsecond");
                assert!(sessions.pending_messages_snapshot(&sid).is_empty());
                assert!(sessions.is_turn_running_blocking(&sid));
                let events = sessions.data().events_blocking(&sid).unwrap();
                let user_rows = events
                    .iter()
                    .filter(|event| event.event_type == crate::session::EventType::ItemUser)
                    .count();
                assert_eq!(user_rows, 1, "merged queue must be exactly one user row");
                sessions.release_turn_reservation(&sid, &turn_id);
            }
            _ => panic!("expected prepared"),
        }
    }

    /// A revert takes the log back to anchor `k`; whatever was queued belonged to
    /// the state the user discarded, so the flush that follows the next turn end
    /// must find nothing and deliver nothing.
    #[test]
    fn a_flush_after_a_revert_delivers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        sessions
            .insert_detail_rows(
                &sid,
                &[
                    crate::types::user_text("anchor"),
                    crate::types::user_text("dropped by the revert"),
                ],
            )
            .unwrap();
        sessions.enqueue_pending_message(&sid, "discarded").unwrap();

        let lease = sessions
            .try_begin_revert(&sid)
            .expect("revert acquires a lease")
            .expect("lease");
        sessions
            .entry_revert_to_user_anchor(&sid, 1)
            .expect("truncate to the anchor");
        assert!(sessions.discard_pending_messages_for_revert(&sid, lease.operation_id()));
        drop(lease);

        match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::SkippedEmpty => {}
            _ => panic!("a reverted queue must not start a turn"),
        }
        assert_eq!(sessions.pending_messages_snapshot(&sid).len(), 0);
        assert!(!sessions.is_session_busy_blocking(&sid));
        let user_rows = sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .filter(|event| event.event_type == crate::session::EventType::ItemUser)
            .count();
        assert_eq!(user_rows, 1, "the discarded message must not reach the log");
    }
}
