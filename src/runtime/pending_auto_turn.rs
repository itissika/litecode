//! Idle flush for queued user messages when a turn ends with the queue still
//! holding them.
//!
//! A message queued mid-turn is normally consumed at the next request seam
//! (`sync_request_seam`). If the turn ends first — cancel, clean
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

use crate::permission::PermissionSink;
use crate::runtime::{RuntimeHandle, TurnOptions, spawn_turn};
use crate::session::{LifecycleEvent, SessionManager};
use crate::types::{LitecodeError, UserInput};

pub enum PendingFlush {
    Prepared {
        session_id: String,
        turn_id: String,
        primary_agent: String,
        project: String,
        input: UserInput,
        /// `item/user` seq written for this flush, before mentions.
        anchor_user_seq: u64,
        sink: Arc<dyn PermissionSink>,
        /// WP-098: child session → spawn with the child's named identity.
        child: bool,
    },
    SkippedBusy,
    SkippedEmpty,
    SkippedSessionGone,
}

/// Decide whether an idle session should start a turn for its queued messages.
///
/// On `Prepared`, the turn is reserved, the queue drained, and the merged
/// message already appended as one ordinary `item/user` row. The spawn wakes
/// without writing a second user row, so a spawn failure leaves the message
/// durable instead of evaporating with the reservation.
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
    // WP-098 Q7: steer leftovers (pending) flush first; child follow-ups only
    // once pending is empty, one message per turn, FIFO.
    let use_follow_up = if sessions.has_pending_messages(sid) {
        false
    } else if sessions.has_follow_ups(sid) {
        true
    } else {
        return PendingFlush::SkippedEmpty;
    };

    // WP-098: a child runs its own (subagent) profile; only roots validate
    // against the primary-agent list.
    let child = sessions.parent_session_id_of(sid).is_some();
    let primary_agent = if child {
        match sessions.agent_id(sid) {
            Some(agent) if !agent.is_empty() => agent,
            _ => return PendingFlush::SkippedSessionGone,
        }
    } else {
        let default_primary = runtime.desired_primary_agent();
        match sessions.resolve_primary_agent(sid, default_primary, &runtime.resolved) {
            Ok(id) => id,
            Err(_) => return PendingFlush::SkippedSessionGone,
        }
    };
    let project = sessions
        .project(sid)
        .unwrap_or_else(|| workspace_root.display().to_string());
    let step_max = crate::config::bridge::agent_config_for(&runtime.resolved, &primary_agent)
        .map(|a| a.max_steps)
        .unwrap_or(50);
    let turn_id = uuid::Uuid::new_v4().to_string();
    if use_follow_up {
        let input = match sessions.reserve_turn_and_pop_follow_up(
            sid,
            turn_id.clone(),
            step_max,
            &primary_agent,
            &project,
        ) {
            Ok(Some((_progress, input))) => input,
            Ok(None) => return PendingFlush::SkippedEmpty,
            Err(LitecodeError::AgentAlreadyRunning) => return PendingFlush::SkippedBusy,
            Err(_) => return PendingFlush::SkippedSessionGone,
        };
        let anchor_user_seq =
            match sessions.append_user_message_with_mentions(sid, input.clone(), workspace_root) {
                Ok(seq) => seq,
                Err(error) => {
                    tracing::warn!(session_id = sid, %error, "failed to persist follow-up");
                    sessions.restore_follow_up_front(sid, input);
                    sessions.release_turn_reservation(sid, &turn_id);
                    return PendingFlush::SkippedSessionGone;
                }
            };
        let sink = crate::runtime::auto_turn_sink::auto_turn_permission_sink(sessions, sid);
        return PendingFlush::Prepared {
            session_id: sid.to_string(),
            turn_id,
            primary_agent,
            project,
            input,
            anchor_user_seq,
            sink,
            child,
        };
    }
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

    let input = crate::session::manager::merge_pending(&claimed);
    let anchor_user_seq =
        match sessions.append_user_message_with_mentions(sid, input.clone(), workspace_root) {
            Ok(seq) => seq,
            Err(error) => {
                tracing::warn!(session_id = sid, %error, "failed to persist pending messages");
                sessions.restore_pending_messages(sid, claimed);
                sessions.release_turn_reservation(sid, &turn_id);
                return PendingFlush::SkippedSessionGone;
            }
        };

    let sink = crate::runtime::auto_turn_sink::auto_turn_permission_sink(sessions, sid);
    PendingFlush::Prepared {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        input,
        anchor_user_seq,
        sink,
        child,
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
        input: _,
        anchor_user_seq,
        sink,
        child,
    } = decision
    else {
        return;
    };
    let sessions = Arc::clone(sessions);
    let handle = match spawn_turn(
        runtime,
        session_id.clone(),
        Arc::clone(&sessions),
        crate::runtime::TurnInput::Wake {
            anchor_user_seq: Some(anchor_user_seq),
        },
        sink,
        turn_id.clone(),
        if child {
            TurnOptions::child(primary_agent.clone())
        } else {
            TurnOptions::default()
        },
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
    let decision = try_begin_pending_flush(&runtime_snap, sessions, workspace_root, session_id);
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

    fn test_runtime(root: &std::path::Path) -> (RuntimeHandle, Arc<SessionManager>) {
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
                assert_eq!(input.text, "first\n\nsecond");
                assert!(input.images.is_empty());
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

    #[test]
    fn child_uses_deny_sink_regardless_of_last_sink() {
        use crate::permission::grants::AskPrompt;
        use crate::permission::{AskOutcome, RecordingPermissionSink};
        let dir = tempfile::tempdir().unwrap();
        let (_runtime, sessions) = test_runtime(dir.path());
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
        let child_sink =
            crate::runtime::auto_turn_sink::auto_turn_permission_sink(&sessions, &child);
        assert_eq!(child_sink.ask(&prompt, &cancel).outcome, AskOutcome::Deny);
        let parent_sink =
            crate::runtime::auto_turn_sink::auto_turn_permission_sink(&sessions, &parent);
        assert_ne!(
            parent_sink.ask(&prompt, &cancel).outcome,
            AskOutcome::Deny,
            "primary keeps last sink (Q1)"
        );
        // try_begin_pending_flush goes through the same helper (see sink selection in this module).
    }

    /// WP-098: a busy child with a queued follow-up. Returns (sessions, child, project).
    fn busy_child_with_follow_ups(
        dir: &std::path::Path,
        texts: &[&str],
    ) -> (RuntimeHandle, Arc<SessionManager>, String) {
        let (runtime, sessions) = test_runtime(dir);
        let project = dir.display().to_string();
        let parent = sessions
            .open_session_sync(&project, "default", None)
            .unwrap();
        let child = sessions
            .test_child_session(&project, &parent, "call-1")
            .unwrap();
        sessions
            .reserve_turn(&child, "turn-busy".into(), 10, "default", &project)
            .unwrap();
        for text in texts {
            sessions
                .enqueue_follow_up(&child, UserInput::text(*text))
                .unwrap();
        }
        assert!(sessions.release_turn_reservation(&child, "turn-busy"));
        (runtime, sessions, child)
    }

    fn prepared_input(flush: PendingFlush) -> (String, String) {
        match flush {
            PendingFlush::Prepared { input, turn_id, .. } => (input.text, turn_id),
            _ => panic!("expected Prepared"),
        }
    }

    #[test]
    fn runtime_queue_multiple_runs_one_turn_each_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions, child) =
            busy_child_with_follow_ups(dir.path(), &["one", "two", "three"]);
        let mut seen = Vec::new();
        for _ in 0..3 {
            let (text, turn_id) = prepared_input(try_begin_pending_flush(
                &runtime,
                &sessions,
                dir.path(),
                &child,
            ));
            // While that turn runs, nothing else may start.
            assert!(!matches!(
                try_begin_pending_flush(&runtime, &sessions, dir.path(), &child),
                PendingFlush::Prepared { .. }
            ));
            seen.push(text);
            sessions.release_turn_reservation(&child, &turn_id);
        }
        assert_eq!(
            seen,
            vec!["one", "two", "three"],
            "one turn per message, FIFO"
        );
        assert!(matches!(
            try_begin_pending_flush(&runtime, &sessions, dir.path(), &child),
            PendingFlush::SkippedEmpty
        ));
    }

    #[test]
    fn runtime_stop_and_send_starting_window() {
        use crate::session::manager::{CancelOutcome, FollowUpOutcome};
        let dir = tempfile::tempdir().unwrap();
        let (_runtime, sessions, child) = busy_child_with_follow_ups(dir.path(), &["older"]);
        let project = dir.path().display().to_string();
        sessions
            .reserve_turn(&child, "turn-start".into(), 10, "default", &project)
            .unwrap();
        let out = sessions
            .cancel_and_follow_up(
                &child,
                UserInput::text("urgent"),
                crate::types::StopOrigin::Parent,
            )
            .unwrap();
        assert_eq!(
            out,
            (
                CancelOutcome::Starting,
                FollowUpOutcome::Queued { position: 1 }
            )
        );
        assert_eq!(sessions.follow_ups_len(&child), 2);
        sessions.release_turn_reservation(&child, "turn-start");
        // Idle child: nothing cancelled, nothing queued.
        let out = sessions
            .cancel_and_follow_up(
                &child,
                UserInput::text("x"),
                crate::types::StopOrigin::Parent,
            )
            .unwrap();
        assert_eq!(out, (CancelOutcome::Idle, FollowUpOutcome::Idle));
        assert_eq!(sessions.follow_ups_len(&child), 2);
    }
}
