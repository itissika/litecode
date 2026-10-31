//! WP-091 (K4/R6): single delete entry shared by `session/delete` (client
//! protocol) and the parent-side subagent delete tool (WP-092).
//!
//! Order: hydrate → descendant_running precheck → exclusive Delete lease →
//! `remove_session` (descendants sorted, root last) → per removed id
//! the caller's per-id cleanup (hub `purge_parent` + `forget_child`). Notices are NOT written here: the caller
//! decides (user delete → controller writes ChildDeleted; tool delete → the
//! tool writes it), so the same delete is never notified twice.

use std::sync::Arc;

use crate::session::manager::{SessionManager, SessionOperationKind};
use crate::types::LitecodeError;

/// Per-removed-id in-memory cleanup hook (the subagent hub implements it).
pub trait RemovedSessionCleanup {
    fn on_session_removed(&self, session_id: &str);
}

impl<T: RemovedSessionCleanup + ?Sized> RemovedSessionCleanup for Arc<T> {
    fn on_session_removed(&self, session_id: &str) {
        (**self).on_session_removed(session_id);
    }
}

/// Closures work too (tests, ad-hoc callers).
impl<F: Fn(&str)> RemovedSessionCleanup for F {
    fn on_session_removed(&self, session_id: &str) {
        self(session_id);
    }
}

/// Why a delete was refused or failed.
#[derive(Debug)]
pub enum DeleteSessionError {
    /// The target itself has a live turn / exclusive operation.
    AlreadyRunning,
    /// A descendant subagent session is busy; ids sorted.
    DescendantRunning { running_session_ids: Vec<String> },
    /// Anything else (not found, storage, ...).
    Other(LitecodeError),
}

impl std::fmt::Display for DeleteSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => f.write_str("agent already running"),
            Self::DescendantRunning { .. } => {
                f.write_str("cannot delete: a descendant subagent session is running")
            }
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DeleteSessionError {}

/// Successful delete outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedSessions {
    /// Every removed id: descendants (sorted) first, root last.
    pub removed: Vec<String>,
    /// Parent of the requested root (read before deletion); None for a root
    /// session. Callers use it for `ParentNotice::ChildDeleted`.
    pub parent_session_id: Option<String>,
}

/// Delete `session_id` and its whole subtree.
///
/// `cleanup.on_session_removed` runs once per removed id (descendants first,
/// root last). The subagent hub implements [`RemovedSessionCleanup`]
/// (`purge_parent` + `forget_child`; impl lives in client_protocol because
/// the session layer must not name the hub — architecture_layers).
pub async fn delete_session_tree<C: RemovedSessionCleanup + ?Sized>(
    sessions: &Arc<SessionManager>,
    cleanup: &C,
    session_id: &str,
) -> Result<DeletedSessions, DeleteSessionError> {
    // List is SQLite; the exclusive lease needs an in-memory record.
    sessions
        .ensure_entry(session_id)
        .await
        .map_err(DeleteSessionError::Other)?;

    let running = sessions.running_descendant_ids(session_id);
    if !running.is_empty() {
        return Err(DeleteSessionError::DescendantRunning {
            running_session_ids: running,
        });
    }

    let parent_session_id = sessions.parent_session_id_of(session_id);

    // Atomically reserve the idle session; a turn or compact cannot start
    // between the gate and the destructive delete.
    let _lease = sessions
        .try_begin_operation(session_id, SessionOperationKind::Delete)
        .map_err(|e| match e {
            LitecodeError::AgentAlreadyRunning => {
                // A descendant may have started between the precheck and the
                // lease; report it precisely when that is the case.
                let running = sessions.running_descendant_ids(session_id);
                if running.is_empty() {
                    DeleteSessionError::AlreadyRunning
                } else {
                    DeleteSessionError::DescendantRunning {
                        running_session_ids: running,
                    }
                }
            }
            other => DeleteSessionError::Other(other),
        })?;

    let removed = sessions
        .remove_session(session_id)
        .map_err(DeleteSessionError::Other)?;
    // Explicit, idempotent cleanup for every removed id; the hub router also
    // purges on SessionRemoved, but must not be the only path (Lagged).
    for id in &removed {
        cleanup.on_session_removed(id);
    }
    Ok(DeletedSessions {
        removed,
        parent_session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::session::LifecycleEvent;

    fn mgr(dir: &std::path::Path) -> Arc<SessionManager> {
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.join("sessions.db").to_string_lossy().to_string(),
        ))
    }

    #[tokio::test]
    async fn remove_session_returns_descendants_sorted_root_last() {
        let dir = tempfile::tempdir().unwrap();
        let m = mgr(dir.path());
        let root = m.open_session("/proj", "default", None).await.unwrap();
        let a = m
            .open_child_session("/proj", "default", None, &root, "c1")
            .unwrap();
        let b = m
            .open_child_session("/proj", "default", None, &a, "c2")
            .unwrap();
        let mut rx = m.subscribe_lifecycle();
        let removed = m.remove_session(&root).unwrap();
        let mut desc = vec![a.clone(), b.clone()];
        desc.sort();
        desc.push(root.clone());
        assert_eq!(removed, desc);
        // R7: each deleted event carries its parent.
        let mut seen = std::collections::HashMap::new();
        while let Ok(ev) = rx.try_recv() {
            if let LifecycleEvent::SessionRemoved {
                session_id,
                parent_session_id,
            } = ev
            {
                seen.insert(session_id, parent_session_id);
            }
        }
        assert_eq!(seen.get(&root), Some(&None));
        assert_eq!(seen.get(&a), Some(&Some(root.clone())));
        assert_eq!(seen.get(&b), Some(&Some(a.clone())));
    }

    #[tokio::test]
    async fn tombstone_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (root, child) = {
            let m = mgr(dir.path());
            let root = m.open_session("/proj", "default", None).await.unwrap();
            let child = m
                .open_child_session("/proj", "default", None, &root, "call_x")
                .unwrap();
            let out = delete_session_tree(&m, &|_: &str| {}, &child)
                .await
                .unwrap();
            assert_eq!(out.parent_session_id.as_deref(), Some(root.as_str()));
            assert_eq!(out.removed, vec![child.clone()]);
            (root, child)
        };
        let m = mgr(dir.path());
        let tomb = m.deleted_child_bindings_for_parent(&root);
        assert_eq!(tomb.get("call_x"), Some(&child));
        assert!(m.child_bindings_for_parent(&root).is_empty());
    }
}
