//! WP-091 R8 / P7: raw `status` on `turn_started` / `turn_finished`, plus a
//! `status_changed` lifecycle event when an ancestor's aggregate status
//! (`running_with_subagent`) really changes.
//!
//! The change detection runs in Runtime's `TurnStatusHook` (WP-090,
//! `SessionManager::set_turn_status_hook`), installed once by the serve layer
//! via [`install_parent_status_hook`]; it emits
//! `LifecycleEvent::SessionStatusChanged`, which every connection projects.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use serde_json::{Value, json};

use crate::session::manager::SessionStatus;
use crate::session::{LifecycleEvent, SessionManager};

/// `session/lifecycle {event:"status_changed", session_id, status}`.
pub fn session_lifecycle_status_changed(session_id: &str, status: &str) -> Value {
    super::project::notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "status_changed",
            "status": status,
        }),
    )
}

/// Project one lifecycle event to its wire frame; `turn_started` /
/// `turn_finished` get the session's raw `status` (Q1). `turn_updated` /
/// `turn_step` stay without it (Q2).
pub fn project_lifecycle(sessions: &SessionManager, ev: &LifecycleEvent) -> Value {
    if let LifecycleEvent::SessionStatusChanged { session_id, status } = ev {
        return session_lifecycle_status_changed(session_id, status);
    }
    let mut base = super::project::lifecycle_event_to_wire(ev);
    if let LifecycleEvent::TurnStarted { session_id, .. }
    | LifecycleEvent::TurnFinished { session_id, .. } = ev
    {
        let own = sessions
            .session_status(session_id)
            .unwrap_or(SessionStatus::Idle);
        if let Some(params) = base.get_mut("params").and_then(Value::as_object_mut) {
            params.insert("status".into(), json!(own.as_str()));
        }
    }
    base
}

/// Process-wide memory of the last aggregate pushed per ancestor.
#[derive(Default)]
pub struct ParentStatusTracker {
    last: Mutex<HashMap<String, SessionStatus>>,
}

impl ParentStatusTracker {
    /// Handle one hook call for `session_id`: recompute every ancestor's
    /// aggregate status and emit `status_changed` only where it changed.
    /// Unknown previous value counts as `Idle`.
    pub fn on_turn_status(&self, sessions: &SessionManager, session_id: &str) {
        let mut cursor = sessions.parent_session_id_of(session_id);
        let mut depth = 0;
        while let Some(parent) = cursor {
            depth += 1;
            if depth > 64 {
                break;
            }
            let status = sessions
                .session_status(&parent)
                .unwrap_or(SessionStatus::Idle);
            let changed = {
                let mut last = self.last.lock().unwrap();
                let prev = last.insert(parent.clone(), status);
                prev.unwrap_or(SessionStatus::Idle) != status
            };
            if changed {
                sessions.emit_status_changed(&parent, status);
            }
            cursor = sessions.parent_session_id_of(&parent);
        }
    }
}

/// Register Session's R8 handler on Runtime's hook (serve layer, once).
pub fn install_parent_status_hook(sessions: &Arc<SessionManager>) -> Arc<ParentStatusTracker> {
    let tracker = Arc::new(ParentStatusTracker::default());
    let weak: Weak<SessionManager> = Arc::downgrade(sessions);
    let t = Arc::clone(&tracker);
    sessions.set_turn_status_hook(Arc::new(move |sid: &str, _status: SessionStatus| {
        if let Some(sessions) = weak.upgrade() {
            t.on_turn_status(&sessions, sid);
        }
    }));
    tracker
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;

    #[tokio::test]
    async fn turn_events_carry_raw_status_and_parent_status_changed() {
        let dir = tempfile::tempdir().unwrap();
        let m = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("s.db").to_string_lossy().to_string(),
        ));
        let parent = m.open_session("/proj", "default", None).await.unwrap();
        let child = m.test_child_session("/proj", &parent, "c").unwrap();
        let tracker = ParentStatusTracker::default();
        let mut rx = m.subscribe_lifecycle();

        // begin_turn does not fire Runtime's hook; drive the handler directly.
        m.begin_turn(
            &child,
            "t".into(),
            tokio_util::sync::CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        tracker.on_turn_status(&m, &child);
        let mut frames = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            frames.push(project_lifecycle(&m, &ev));
        }
        let started = frames
            .iter()
            .find(|f| f["params"]["event"] == "turn_started")
            .expect("turn_started");
        assert_eq!(started["params"]["status"], "running");
        let changed: Vec<_> = frames
            .iter()
            .filter(|f| f["params"]["event"] == "status_changed")
            .collect();
        assert_eq!(changed.len(), 1, "{frames:?}");
        assert_eq!(changed[0]["params"]["session_id"], parent.as_str());
        assert_eq!(changed[0]["params"]["status"], "running_with_subagent");

        // Same state again → no duplicate.
        tracker.on_turn_status(&m, &child);
        assert!(rx.try_recv().is_err());

        m.finish_turn(&child, "t").unwrap();
        tracker.on_turn_status(&m, &child);
        let mut frames = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            frames.push(project_lifecycle(&m, &ev));
        }
        let finished = frames
            .iter()
            .find(|f| f["params"]["event"] == "turn_finished")
            .expect("turn_finished");
        assert_eq!(finished["params"]["status"], "idle");
        let changed = frames
            .iter()
            .find(|f| f["params"]["event"] == "status_changed")
            .expect("status_changed back to idle");
        assert_eq!(changed["params"]["status"], "idle");
    }

    #[tokio::test]
    async fn installed_hook_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let m = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("s.db").to_string_lossy().to_string(),
        ));
        let parent = m.open_session("/proj", "default", None).await.unwrap();
        let child = m.test_child_session("/proj", &parent, "c").unwrap();
        let mut rx = m.subscribe_lifecycle();
        let _tracker = install_parent_status_hook(&m);
        // reserve + release fires the hook (release → Idle): parent stays idle,
        // so no status_changed is pushed.
        m.reserve_turn(&child, "t".into(), 5, "default", "/proj")
            .unwrap();
        assert!(m.release_turn_reservation(&child, "t"));
        while let Ok(ev) = rx.try_recv() {
            assert!(
                !matches!(ev, LifecycleEvent::SessionStatusChanged { .. }),
                "unchanged aggregate must not push: {ev:?}"
            );
        }
    }
}
