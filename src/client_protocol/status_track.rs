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
    // WP-099 P2 / Q5: turn_finished keeps `reason` and adds `cancelled_by`
    // next to it (explicit null).
    if let LifecycleEvent::TurnFinished {
        reason,
        cancelled_by,
        ..
    } = ev
    {
        // From WP-098's TurnFinished (same value as persisted in turn/end).
        let cancelled_by = (reason.as_deref() == Some("cancelled"))
            .then(|| cancelled_by.map(|b| b.as_str()))
            .flatten();
        if let Some(params) = base.get_mut("params").and_then(Value::as_object_mut) {
            params.insert("cancelled_by".into(), json!(cancelled_by));
        }
    }
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

/// WP-099: the latest durable `turn/end` record of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEndRecord {
    pub turn_id: String,
    pub reason: String,
    /// `"user"` | `"parent"`; only when `reason == "cancelled"` and the record
    /// carries `cancelled_by` (WP-098 §3). Old logs → None.
    pub cancelled_by: Option<String>,
}

/// Latest `turn/end` via WP-098 `SessionData::latest_turn_end_blocking`.
/// `cancelled_by` is kept only when `reason == "cancelled"`.
pub fn latest_turn_end(sessions: &SessionManager, session_id: &str) -> Option<TurnEndRecord> {
    let (turn_id, reason, by) = sessions
        .data()
        .latest_turn_end_blocking(session_id)
        .ok()
        .flatten()?;
    let cancelled_by = (reason == "cancelled")
        .then(|| by.map(|b| b.as_str().to_string()))
        .flatten();
    Some(TurnEndRecord {
        turn_id,
        reason,
        cancelled_by,
    })
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

    fn wp099_turn_end(m: &SessionManager, sid: &str, turn: &str, reason: &str, by: Option<&str>) {
        let mut data = json!({"turn": turn, "reason": reason, "final_text": ""});
        if let Some(by) = by {
            data["cancelled_by"] = json!(by);
        }
        m.apply(
            sid,
            crate::session::store::SessionApply::Append(crate::session::EventDraft {
                time: 1,
                event_type: crate::session::EventType::TurnEnd,
                data,
                surface_op: None,
                source_seqs: None,
                ignorable: false,
                state: crate::session::LogState::Final,
            }),
        )
        .unwrap();
    }

    fn wp099_mgr(dir: &std::path::Path) -> Arc<SessionManager> {
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.join("s.db").to_string_lossy().to_string(),
        ))
    }

    #[tokio::test]
    async fn turn_end_cancelled_by_user_parent_null() {
        let dir = tempfile::tempdir().unwrap();
        let m = wp099_mgr(dir.path());
        let parent = m.open_session("/proj", "default", None).await.unwrap();
        let child = m.test_child_session("/proj", &parent, "c").unwrap();

        wp099_turn_end(&m, &child, "t1", "cancelled", Some("user"));
        assert_eq!(
            latest_turn_end(&m, &child).unwrap().cancelled_by.as_deref(),
            Some("user")
        );
        wp099_turn_end(&m, &child, "t2", "cancelled", Some("parent"));
        assert_eq!(
            latest_turn_end(&m, &child).unwrap().cancelled_by.as_deref(),
            Some("parent")
        );
        // Not cancelled → null even if a stray key exists.
        wp099_turn_end(&m, &child, "t3", "completed", Some("user"));
        let end = latest_turn_end(&m, &child).unwrap();
        assert_eq!((end.reason.as_str(), end.cancelled_by), ("completed", None));
        // Old log without the key → null.
        wp099_turn_end(&m, &child, "t4", "cancelled", None);
        assert_eq!(latest_turn_end(&m, &child).unwrap().cancelled_by, None);
    }

    #[tokio::test]
    async fn last_turn_cancelled_by_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (parent, child) = {
            let m = wp099_mgr(dir.path());
            let parent = m.open_session("/proj", "default", None).await.unwrap();
            let child = m.test_child_session("/proj", &parent, "c").unwrap();
            wp099_turn_end(&m, &child, "t1", "cancelled", Some("user"));
            (parent, child)
        };
        let _ = parent;
        let m = wp099_mgr(dir.path());
        let end = latest_turn_end(&m, &child).unwrap();
        assert_eq!(end.turn_id, "t1");
        assert_eq!(end.cancelled_by.as_deref(), Some("user"));
    }

    #[tokio::test]
    async fn turn_finished_event_carries_cancelled_by() {
        let dir = tempfile::tempdir().unwrap();
        let m = wp099_mgr(dir.path());
        let parent = m.open_session("/proj", "default", None).await.unwrap();
        let child = m.test_child_session("/proj", &parent, "c").unwrap();
        wp099_turn_end(&m, &child, "t1", "cancelled", Some("parent"));
        let progress = crate::session::TurnProgress {
            turn_id: "t1".into(),
            phase: crate::runtime::observer::TurnPhase::Idle,
            step: 0,
            step_max: 1,
            started_at_ms: 0,
            awaiting_permission: false,
        };
        let mk = |reason: Option<&str>, progress: crate::session::TurnProgress| {
            LifecycleEvent::TurnFinished {
                session_id: child.clone(),
                progress,
                reason: reason.map(str::to_string),
                cancelled_by: Some(crate::types::StopOrigin::Parent),
            }
        };
        let v = project_lifecycle(&m, &mk(Some("cancelled"), progress.clone()));
        assert_eq!(v["params"]["reason"], "cancelled");
        assert_eq!(v["params"]["cancelled_by"], "parent");
        assert!(
            v["params"].get("last_turn_cancelled_by").is_none(),
            "one name per location"
        );
        // reason != cancelled → null even if an origin were attached.
        let v = project_lifecycle(&m, &mk(Some("completed"), progress));
        let params = v["params"].as_object().unwrap();
        assert!(params.contains_key("cancelled_by"), "explicit null");
        assert!(params["cancelled_by"].is_null());
    }
}
