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

use crate::permission::{PermissionSink, deny_permission_sink};
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

    let sink = sessions
        .last_permission_sink(sid)
        .unwrap_or_else(deny_permission_sink);
    PendingFlush::Prepared {
        session_id: sid.to_string(),
        turn_id,
        primary_agent,
        project,
        input,
        anchor_user_seq,
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
        input: _,
        anchor_user_seq,
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
        crate::runtime::TurnInput::Wake {
            anchor_user_seq: Some(anchor_user_seq),
        },
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
    fn idle_flush_keeps_images_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let first = format!("litecode-media:{}.jpg", "aa".repeat(32));
        let second = format!("litecode-media:{}.png", "bb".repeat(32));
        sessions
            .enqueue_user_input(
                &sid,
                UserInput {
                    text: "look".into(),
                    images: vec![first.clone()],
                },
            )
            .unwrap();
        sessions
            .enqueue_user_input(
                &sid,
                UserInput {
                    text: String::new(),
                    images: vec![second.clone()],
                },
            )
            .unwrap();
        match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::Prepared { input, turn_id, .. } => {
                assert_eq!(input.text, "look\n\n");
                assert_eq!(input.images, vec![first.clone(), second.clone()]);
                let events = sessions.data().events_blocking(&sid).unwrap();
                let row = events
                    .iter()
                    .find(|event| event.event_type == crate::session::EventType::ItemUser)
                    .expect("user row");
                let body = row.data.to_string();
                assert!(body.contains(&first));
                assert!(body.contains(&second));
                sessions.release_turn_reservation(&sid, &turn_id);
            }
            _ => panic!("expected prepared"),
        }
    }

    #[test]
    fn idle_flush_writes_the_mentions_reminder_after_the_user_row() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "fn save() {\n    let n = 1;\n}\n",
        )
        .unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let text = crate::knowledge::mentions::symbol_mention_source(
            "src/a.rs",
            Some("fn save"),
            None,
            "fn save",
        );
        sessions.enqueue_pending_message(&sid, &text).unwrap();
        match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::Prepared { turn_id, .. } => {
                let events = sessions.data().events_blocking(&sid).unwrap();
                let kinds: Vec<_> = events
                    .iter()
                    .map(|event| event.event_type.as_str().to_string())
                    .collect();
                let user = kinds.iter().position(|kind| kind == "item/user").unwrap();
                assert_eq!(kinds[user + 1], "reminder/mentions");
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

    #[test]
    fn flush_with_a_mention_names_the_snapshot_after_the_user_row() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "fn save() {\n    let n = 1;\n}\n",
        )
        .unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let text = crate::knowledge::mentions::symbol_mention_source(
            "src/a.rs",
            Some("fn save"),
            None,
            "fn save",
        );
        sessions.enqueue_pending_message(&sid, &text).unwrap();
        let (anchor, turn_id) = match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid)
        {
            PendingFlush::Prepared {
                anchor_user_seq,
                turn_id,
                ..
            } => (anchor_user_seq, turn_id),
            _ => panic!("expected prepared"),
        };
        let events = sessions.data().events_blocking(&sid).unwrap();
        let user = events
            .iter()
            .find(|event| event.event_type == crate::session::EventType::ItemUser)
            .expect("user row");
        assert_eq!(user.seq, anchor);
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Reminder(_))),
            "a file mention appends a reminder after the user row"
        );
        sessions
            .apply(
                &sid,
                crate::session::store::SessionApply::Append(crate::session::event::EventDraft {
                    time: 0,
                    event_type: crate::session::EventType::TurnStart,
                    data: serde_json::json!({ "turn": "queued" }),
                    surface_op: None,
                    source_seqs: None,
                    ignorable: false,
                    state: crate::session::LogState::Final,
                }),
            )
            .unwrap();
        let (_last, next_seq) = sessions.entry_wire_seq_cursor(&sid);
        let stem = crate::runtime::snapshot_stem_for_turn(Some(anchor), next_seq);
        assert_eq!(stem, i64::try_from(anchor).unwrap() + 1);
        assert_ne!(stem as u64, next_seq);
        let snaps = runtime.workspace.paths.snapshots_dir.clone();
        crate::session::snapshot::snapshot_track(dir.path(), &snaps, &sid, stem).unwrap();
        assert!(crate::session::snapshot::snapshot_exists(
            &snaps, &sid, stem
        ));
        let restored =
            crate::session::snapshot::snapshot_restore(dir.path(), &snaps, &sid, stem).unwrap();
        assert!(
            !matches!(
                restored,
                crate::session::snapshot::RestoreOutcome::Unavailable {
                    reason: crate::session::snapshot::RestoreUnavailable::MissingTrackRef,
                }
            ),
            "restore must find the ref named after the user row"
        );
        sessions.release_turn_reservation(&sid, &turn_id);
    }

    #[test]
    fn flush_without_a_mention_still_names_the_snapshot_after_the_user_row() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        sessions.enqueue_pending_message(&sid, "hello").unwrap();
        let anchor = match try_begin_pending_flush(&runtime, &sessions, dir.path(), &sid) {
            PendingFlush::Prepared {
                anchor_user_seq,
                turn_id,
                ..
            } => {
                sessions.release_turn_reservation(&sid, &turn_id);
                anchor_user_seq
            }
            _ => panic!("expected prepared"),
        };
        let (_last, next_seq) = sessions.entry_wire_seq_cursor(&sid);
        assert_eq!(next_seq, anchor + 1);
        assert_eq!(
            crate::runtime::snapshot_stem_for_turn(Some(anchor), next_seq),
            i64::try_from(anchor).unwrap() + 1
        );
        assert_eq!(
            crate::runtime::snapshot_stem_for_turn(None, next_seq),
            i64::try_from(next_seq).unwrap()
        );
    }

    #[test]
    fn user_turn_with_a_mention_names_the_snapshot_after_the_committed_row() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "fn save() {\n    let n = 1;\n}\n",
        )
        .unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let sid = sessions
            .open_session_sync(&dir.path().display().to_string(), "default", None)
            .unwrap();
        let text = crate::knowledge::mentions::symbol_mention_source(
            "src/a.rs",
            Some("fn save"),
            None,
            "fn save",
        );
        let context = crate::context_pipeline::build_context(
            &runtime.resolved,
            dir.path(),
            &runtime.workspace.paths,
        );
        let pipeline = crate::context_pipeline::ContextPipeline::new(
            128_000,
            context,
            sessions.data_root_path(),
        );
        let mut working = pipeline
            .begin_turn_with_id(&sessions, &sid, Some("turn".into()))
            .unwrap();
        sessions
            .apply(
                &sid,
                crate::session::store::SessionApply::Append(crate::session::event::EventDraft {
                    time: 0,
                    event_type: crate::session::EventType::TurnStart,
                    data: serde_json::json!({ "turn": "turn" }),
                    surface_op: None,
                    source_seqs: None,
                    ignorable: false,
                    state: crate::session::LogState::Final,
                }),
            )
            .unwrap();
        working.push(crate::session::working::WorkingRow::pending(
            crate::types::user_message(&text, &[]),
        ));
        pipeline.commit_step(&sessions, &sid, &mut working).unwrap();
        let user_seq = working
            .iter()
            .rev()
            .find(|row| row.kind == crate::session::model::SessionKind::ItemUser)
            .and_then(|row| row.log_seq)
            .expect("committed user row");
        sessions.append_mentions_for(&sid, dir.path(), &text);
        let (_last, next_seq) = sessions.entry_wire_seq_cursor(&sid);
        let stem = crate::runtime::snapshot_stem_for_turn(Some(user_seq), next_seq);
        assert_eq!(stem, i64::try_from(user_seq).unwrap() + 1);
        assert_ne!(
            stem as u64, next_seq,
            "the mentions row must not move the snapshot stem"
        );
        let snaps = runtime.workspace.paths.snapshots_dir.clone();
        crate::session::snapshot::snapshot_track(dir.path(), &snaps, &sid, stem).unwrap();
        let restored =
            crate::session::snapshot::snapshot_restore(dir.path(), &snaps, &sid, stem).unwrap();
        assert!(!matches!(
            restored,
            crate::session::snapshot::RestoreOutcome::Unavailable {
                reason: crate::session::snapshot::RestoreUnavailable::MissingTrackRef,
            }
        ));
    }
}
