//! Completion routing client for child Session turns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::session::live::LifecycleEvent;
use crate::session::manager::SessionManager;

use super::jobs::{CompletionInbox, CompletionRef, NoticeInbox, ParentNotice};

type ExitHandler = Arc<dyn Fn(CompletionRef) + Send + Sync>;

pub struct SubagentHub {
    pub completions: Arc<CompletionInbox>,
    /// WP-090 K5: non-waking parent notices (separate from completions).
    notices: Arc<NoticeInbox>,
    sessions: Mutex<Option<Arc<SessionManager>>>,
    exit_handler: Arc<Mutex<Option<ExitHandler>>>,
}

impl Default for SubagentHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentHub {
    pub fn new() -> Self {
        Self {
            completions: Arc::new(CompletionInbox::default()),
            notices: Arc::new(NoticeInbox::default()),
            sessions: Mutex::new(None),
            exit_handler: Arc::new(Mutex::new(None)),
        }
    }

    pub fn attach_sessions(&self, sessions: Arc<SessionManager>) {
        let mut slot = self.sessions.lock().expect("hub sessions lock");
        let first = slot.is_none();
        *slot = Some(Arc::clone(&sessions));
        drop(slot);
        if first {
            spawn_completion_router(Router::new(
                Arc::clone(&self.completions),
                Arc::clone(&self.notices),
                sessions,
                Arc::clone(&self.exit_handler),
            ));
        }
    }

    pub fn set_exit_handler(&self, handler: ExitHandler) {
        *self.exit_handler.lock().expect("exit handler lock") = Some(handler);
    }

    pub fn purge_parent(&self, parent_session_id: &str) {
        self.completions.purge_parent(parent_session_id);
        self.notices.purge_parent(parent_session_id);
    }

    /// Drops completions for `child_id`. Notices *about* the child are kept so
    /// a ChildDeleted written by the delete flow is not eaten by that flow.
    pub fn forget_child(&self, child_id: &str) {
        self.completions.forget_child(child_id);
    }

    pub fn has_pending(&self, parent_session_id: &str) -> bool {
        self.completions.pending(parent_session_id)
    }

    pub fn take_completions(&self, parent_session_id: &str) -> Vec<CompletionRef> {
        self.completions.take_all(parent_session_id)
    }

    pub fn restore_completions(&self, parent_session_id: &str, completions: Vec<CompletionRef>) {
        self.completions
            .restore_front(parent_session_id, completions);
    }

    /// Write a notice into the parent's notice mailbox. NEVER wakes the parent:
    /// no exit handler, no CompletionInbox, no `has_pending`, no auto-turn.
    /// Identical queued notices are not duplicated. Empty parent id → no-op.
    pub fn notify_parent_no_wake(&self, parent_session_id: &str, notice: ParentNotice) {
        if parent_session_id.is_empty() || notice.child_session_id().is_empty() {
            return;
        }
        self.notices.push(parent_session_id, notice);
    }

    /// Step-seam consumption (Runtime exec only).
    pub fn take_notices(&self, parent_session_id: &str) -> Vec<ParentNotice> {
        self.notices.take_all(parent_session_id)
    }

    pub fn restore_notices(&self, parent_session_id: &str, notices: Vec<ParentNotice>) {
        self.notices.restore_front(parent_session_id, notices);
    }

    /// Observation/tests only; never part of wake decisions.
    pub fn has_notices(&self, parent_session_id: &str) -> bool {
        self.notices.pending(parent_session_id)
    }
}

/// Router state. `delivered` remembers the last routed turn per child so a
/// `Lagged` rescan (R11) redelivers only what was missed.
struct Router {
    completions: Arc<CompletionInbox>,
    notices: Arc<NoticeInbox>,
    sessions: Arc<SessionManager>,
    exit_handler: Arc<Mutex<Option<ExitHandler>>>,
    delivered: HashMap<String, String>,
}

impl Router {
    fn new(
        completions: Arc<CompletionInbox>,
        notices: Arc<NoticeInbox>,
        sessions: Arc<SessionManager>,
        exit_handler: Arc<Mutex<Option<ExitHandler>>>,
    ) -> Self {
        Self {
            completions,
            notices,
            sessions,
            exit_handler,
            delivered: HashMap::new(),
        }
    }

    /// WP-098 R-W: a cancelled turn lands in the mailbox (overwriting any
    /// unread older result) but never runs the exit handler / wakes the parent.
    fn deliver(&mut self, completion: CompletionRef) {
        self.delivered.insert(
            completion.child_session_id.clone(),
            completion.turn_id.clone(),
        );
        let cancelled = completion.cancelled;
        self.completions.push(completion.clone());
        if cancelled {
            return;
        }
        if let Some(handler) = self.exit_handler.lock().expect("exit handler lock").clone() {
            handler(completion);
        }
    }

    /// Returns false when the channel is closed.
    fn handle(&mut self, received: Result<LifecycleEvent, broadcast::error::RecvError>) -> bool {
        match received {
            Ok(LifecycleEvent::TurnFinished {
                session_id,
                progress,
                reason,
                cancelled_by,
            }) => {
                let Ok(meta) = self.sessions.reader().meta_blocking(&session_id) else {
                    return true;
                };
                let Some(parent_session_id) = meta.parent_session_id else {
                    return true;
                };
                self.deliver(CompletionRef {
                    parent_session_id,
                    child_session_id: session_id,
                    turn_id: progress.turn_id,
                    cancelled: reason.as_deref() == Some("cancelled"),
                    cancelled_by,
                });
            }
            Ok(LifecycleEvent::SessionRemoved { session_id, .. }) => {
                self.completions.purge_parent(&session_id);
                self.completions.forget_child(&session_id);
                self.notices.purge_parent(&session_id);
                self.delivered.remove(&session_id);
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "subagent completion router lagged; rescanning");
                self.rescan();
            }
            Err(broadcast::error::RecvError::Closed) => return false,
        }
        true
    }

    /// R11: redeliver the latest durable turn of every idle in-memory child
    /// whose turn id differs from the last routed one. CompletionInbox dedups
    /// (child, turn), and `delivered` prevents repeat wakes.
    fn rescan(&mut self) {
        let mut children = self.sessions.idle_child_session_ids();
        children.sort();
        for (child, parent) in children {
            let Ok(Some((turn_id, reason, cancelled_by))) =
                self.sessions.data().latest_turn_end_blocking(&child)
            else {
                continue;
            };
            if self.delivered.get(&child) == Some(&turn_id) {
                continue;
            }
            self.deliver(CompletionRef {
                parent_session_id: parent,
                child_session_id: child,
                turn_id,
                cancelled: reason == "cancelled",
                cancelled_by,
            });
        }
    }
}

fn spawn_completion_router(mut router: Router) {
    let mut rx = router.sessions.subscribe_lifecycle();
    if let Err(error) = std::thread::Builder::new()
        .name("subagent-completion-router".into())
        .spawn(move || while router.handle(rx.blocking_recv()) {})
    {
        tracing::error!(%error, "failed to spawn subagent completion router");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::session::TurnProgress;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture() -> (tempfile::TempDir, Arc<SessionManager>, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("sessions.db").to_str().unwrap().to_string(),
        ));
        let parent = sessions
            .open_session_sync("/proj", "default", None)
            .unwrap();
        let child = sessions
            .open_child_session("/proj", "reviewer", None, &parent, "call-1")
            .unwrap();
        (dir, sessions, parent, child)
    }

    fn counting_router(sessions: &Arc<SessionManager>) -> (Router, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let handler: ExitHandler = Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let router = Router::new(
            Arc::new(CompletionInbox::default()),
            Arc::new(NoticeInbox::default()),
            Arc::clone(sessions),
            Arc::new(Mutex::new(Some(handler))),
        );
        (router, wakes)
    }

    fn append_turn_end(sessions: &SessionManager, sid: &str, turn: &str) {
        sessions
            .apply(
                sid,
                crate::session::store::SessionApply::Append(crate::session::EventDraft {
                    time: 1,
                    event_type: crate::session::EventType::TurnEnd,
                    data: serde_json::json!({"turn": turn, "reason": "completed", "final_text": "ok"}),
                    surface_op: None,
                    source_seqs: None,
                    ignorable: false,
                    state: crate::session::LogState::Final,
                }),
            )
            .unwrap();
    }

    fn notice_without_wake(notice: ParentNotice) {
        let hub = SubagentHub::new();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        hub.set_exit_handler(Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        hub.notify_parent_no_wake("parent", notice.clone());
        hub.notify_parent_no_wake("parent", notice.clone()); // dedup
        hub.notify_parent_no_wake("", notice.clone()); // no-op
        assert!(hub.has_notices("parent"));
        assert!(
            !hub.has_pending("parent"),
            "notice must not count as pending"
        );
        assert_eq!(wakes.load(Ordering::SeqCst), 0, "exit handler must not run");
        hub.forget_child(notice.child_session_id());
        assert!(hub.has_notices("parent"), "forget_child keeps the notice");
        let taken = hub.take_notices("parent");
        assert_eq!(taken, vec![notice.clone()]);
        hub.restore_notices("parent", taken);
        assert_eq!(hub.take_notices("parent"), vec![notice]);
        assert!(!hub.has_notices("parent"));
    }

    #[test]
    fn child_stopped_notice_writes_mailbox_without_wake() {
        notice_without_wake(ParentNotice::ChildStopped {
            child_session_id: "child-a".into(),
            by: crate::types::StopOrigin::User,
        });
    }

    #[test]
    fn child_deleted_notice_writes_mailbox_without_wake() {
        notice_without_wake(ParentNotice::ChildDeleted {
            child_session_id: "child-a".into(),
            by: crate::types::StopOrigin::User,
        });
        let hub = SubagentHub::new();
        hub.notify_parent_no_wake(
            "parent",
            ParentNotice::ChildDeleted {
                child_session_id: "c".into(),
                by: crate::types::StopOrigin::Parent,
            },
        );
        hub.purge_parent("parent");
        assert!(!hub.has_notices("parent"), "purge_parent clears notices");
    }

    #[test]
    fn child_turn_finished_still_wakes_parent() {
        let (_dir, sessions, parent, child) = fixture();
        let (mut router, wakes) = counting_router(&sessions);
        router.notices.push(
            &parent,
            ParentNotice::ChildStopped {
                child_session_id: child.clone(),
                by: crate::types::StopOrigin::User,
            },
        );
        let progress = TurnProgress {
            turn_id: "t1".into(),
            phase: crate::runtime::observer::TurnPhase::Starting,
            step: 1,
            step_max: 5,
            started_at_ms: 0,
            awaiting_permission: false,
        };
        assert!(router.handle(Ok(LifecycleEvent::TurnFinished {
            session_id: child.clone(),
            progress,
            reason: None,
            cancelled_by: None,
        })));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(router.completions.pending(&parent));
        assert_eq!(router.completions.take_all(&parent)[0].turn_id, "t1");
    }

    #[test]
    fn lagged_router_rescans_and_redelivers() {
        let (_dir, sessions, parent, child) = fixture();
        let (mut router, wakes) = counting_router(&sessions);
        append_turn_end(&sessions, &child, "t-missed");
        assert!(router.handle(Err(broadcast::error::RecvError::Lagged(7))));
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            1,
            "missed completion redelivered"
        );
        let taken = router.completions.take_all(&parent);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].child_session_id, child);
        assert_eq!(taken[0].turn_id, "t-missed");
        // A second lag with nothing new must not redeliver.
        assert!(router.handle(Err(broadcast::error::RecvError::Lagged(1))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(!router.completions.pending(&parent));
        // Parent (no parent_session_id) is never rescanned as a child.
        append_turn_end(&sessions, &parent, "p-turn");
        assert!(router.handle(Err(broadcast::error::RecvError::Lagged(1))));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    fn progress(turn: &str) -> TurnProgress {
        TurnProgress {
            turn_id: turn.into(),
            phase: crate::runtime::observer::TurnPhase::Starting,
            step: 1,
            step_max: 5,
            started_at_ms: 0,
            awaiting_permission: false,
        }
    }

    #[test]
    fn hub_stopped_turn_result_mailbox_only_no_wake() {
        let (_dir, sessions, parent, child) = fixture();
        let (mut router, wakes) = counting_router(&sessions);
        assert!(router.handle(Ok(LifecycleEvent::TurnFinished {
            session_id: child.clone(),
            progress: progress("t1"),
            reason: Some("completed".into()),
            cancelled_by: None,
        })));
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "normal completion wakes");
        assert!(router.handle(Ok(LifecycleEvent::TurnFinished {
            session_id: child.clone(),
            progress: progress("t2"),
            reason: Some("cancelled".into()),
            cancelled_by: Some(crate::types::StopOrigin::User),
        })));
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "cancelled must not wake");
        assert!(
            !router.notices.pending(&parent),
            "no ChildStopped from router"
        );
        let taken = router.completions.take_all(&parent);
        assert_eq!(taken.len(), 1, "cancelled overwrote unread completed");
        assert_eq!(taken[0].turn_id, "t2");
        assert!(taken[0].cancelled);
        assert_eq!(taken[0].cancelled_by, Some(crate::types::StopOrigin::User));
    }

    #[test]
    fn hub_notice_carries_by_user_parent() {
        use crate::types::StopOrigin;
        let hub = SubagentHub::new();
        hub.notify_parent_no_wake(
            "p",
            ParentNotice::ChildStopped {
                child_session_id: "a".into(),
                by: StopOrigin::User,
            },
        );
        hub.notify_parent_no_wake(
            "p",
            ParentNotice::ChildDeleted {
                child_session_id: "b".into(),
                by: StopOrigin::Parent,
            },
        );
        let taken = hub.take_notices("p");
        assert_eq!(taken[0].by(), StopOrigin::User);
        assert_eq!(taken[1].by(), StopOrigin::Parent);
        assert!(!hub.has_pending("p"));
    }

    #[test]
    fn hub_lagged_rescan_dedups_per_child() {
        let (_dir, sessions, parent, child) = fixture();
        let (mut router, wakes) = counting_router(&sessions);
        append_turn_end(&sessions, &child, "t1");
        assert!(router.handle(Err(broadcast::error::RecvError::Lagged(1))));
        // Simulate a restart that lost `delivered` while the result is unread.
        router.delivered.clear();
        assert!(router.handle(Err(broadcast::error::RecvError::Lagged(1))));
        assert_eq!(
            router.completions.take_all(&parent).len(),
            1,
            "unread same turn is not duplicated"
        );
        // Wake count still rises on the replay (ISSUE-054 remains partial).
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    }
}
