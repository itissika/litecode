//! Session business facade for the subagent and UI-backend callers.
//!
//! `SessionService` owns the session business operations over the process-owned
//! `SessionManager`: callers depend on this facade instead of the manager's
//! primitives. The manager remains the only writer and lock owner. The service
//! adds no transport, projection, auth, idempotency, replay, caching, retry, or
//! second store.
//!
//! Business surface (the mapping target for a future narrow external contract):
//! - [`lifecycle`]: create/list/open, metadata, folders, config and delete.
//! - [`conversation`]: prompt/queue delivery, follow-ups, stop, revert and turns.
//! - [`permissions`]: pending interactions and permission state.
//! - [`observation`]: session facts, history and usage reads.
//!
//! [`support`] is internal infrastructure for projections, runtime construction,
//! compaction workers, startup/GC/shutdown and permission routing. It is not a
//! business contract and must not be used by future external adapters.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::config::ResolvedConfig;
use crate::context_pipeline::{
    BudgetPolicy, ContextEnvelope, ContextMeter, ProviderPromptBaseline, latest_persisted_envelope,
};
use crate::permission::{PermissionRoute, PermissionSink};
use crate::runtime::observer::{
    CompactionTrigger, InternalEnvelope, InternalEvent, RuntimeObserver,
};
use crate::runtime::{AgentRuntime, RuntimeHandle, TurnHandle, TurnInput, TurnOptions, spawn_turn};
use crate::session::data::command::{
    SessionFolderRow, SessionListRow, UserAnchorWindow, WorkingSetState,
};
use crate::session::data::sqlite::session::{ProviderUsageSeries, WorkspaceUsage};
use crate::session::delete::{DeleteSessionError, DeletedSessions, RemovedSessionCleanup};
use crate::session::estimate::ItemTokenBreakdown;
use crate::session::event::Seq;
use crate::session::event::SessionEvent;
use crate::session::live::{LifecycleEvent, TurnProgress};
use crate::session::manager::{
    CancelOutcome, CompactionEpisodeLease, CompactionStateSnapshot, FollowUpOutcome,
    PendingMessage, SessionManager, SessionOperationKind, SessionOperationLease, SessionStatus,
    TurnStatusHook,
};
use crate::session::model::{SessionMeta, TurnResult};
use crate::session::pending::PendingPermissionRecord;
use crate::session::task_state::TaskReminders;
use crate::session::transcript_file::TranscriptFile;
use crate::session::working::WorkingRow;
use crate::types::{LitecodeError, Result, StopOrigin, UserInput};

/// Process-level facade over the one `SessionManager`.
#[derive(Clone)]
pub struct SessionService {
    sessions: Arc<SessionManager>,
}

impl SessionService {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self { sessions }
    }

    /// Shared identity query: which bot, if any, owns this ordinary session.
    /// Session list/search, ACP/IM, and UI consumers use this one entry; the
    /// derived index lives on the shared manager, so every facade sees the same
    /// response-time answer.
    pub fn bot_owner_for_session(
        &self,
        session_id: &str,
    ) -> Option<crate::session::SessionBotOwner> {
        self.sessions.bot_owner_for_session(session_id)
    }

    pub(crate) fn replace_bot_owners(
        &self,
        owners: impl IntoIterator<Item = crate::session::SessionBotOwner>,
    ) {
        self.sessions.replace_bot_owners(owners);
    }

    pub(crate) fn upsert_bot_owner(&self, owner: crate::session::SessionBotOwner) {
        self.sessions.upsert_bot_owner(owner);
    }

    pub(crate) fn remove_bot_owner(&self, session_id: &str) {
        self.sessions.remove_bot_owner(session_id);
    }

    pub(crate) fn is_bot_id(&self, bot_id: &str) -> bool {
        self.sessions.is_bot_id(bot_id)
    }
}

mod conversation;
mod lifecycle;
mod observation;
mod permissions;
mod support;

#[derive(Debug)]
pub(crate) enum TurnStartError {
    AgentAlreadyRunning,
    Runtime(anyhow::Error),
}

/// Business-level failure from a transcript/file revert.
#[derive(Debug)]
pub(crate) enum RevertError {
    Busy,
    InvalidAnchor(String),
    Internal(String),
    Fatal(LitecodeError),
}

/// Exclusive file-revert claim plus the snapshot stem resolved from the user
/// anchor. Holding this value keeps the session operation lease alive.
#[must_use = "hold the revert lease until the projection refresh is emitted"]
pub(crate) struct RevertGuard {
    _lease: Option<SessionOperationLease>,
}

/// Projection facts returned before a manual compaction gate decision. The
/// controller publishes these while the service still owns the lease.
pub(crate) struct ManualCompactionPreview {
    pub context_window: usize,
    pub envelope: ContextEnvelope,
    pub meter: ContextMeter,
    pub breakdown: ItemTokenBreakdown,
}

pub(crate) struct ManualCompactionStart {
    pub operation_id: String,
}

pub(crate) enum ManualCompactionError {
    NothingToCompact,
    Runtime(LitecodeError),
}

impl From<LitecodeError> for ManualCompactionError {
    fn from(error: LitecodeError) -> Self {
        Self::Runtime(error)
    }
}

#[must_use = "hold the file-revert lease through the workspace restore"]
pub(crate) struct FileRevertPlan {
    _lease: SessionOperationLease,
    stem: i64,
}

impl FileRevertPlan {
    pub(crate) fn stem(&self) -> i64 {
        self.stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::TurnGuard;

    fn fixture() -> (tempfile::TempDir, Arc<SessionManager>, SessionService) {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("sessions.db").to_string_lossy().to_string(),
        ));
        let service = SessionService::new(Arc::clone(&manager));
        (dir, manager, service)
    }

    #[tokio::test]
    async fn child_facts_delegate_to_the_same_manager() {
        let (_dir, manager, service) = fixture();
        let parent = manager
            .open_session_sync("/proj", "default", None)
            .expect("parent");
        let child = service
            .open_child_session_with_responsibility(
                "/proj", "reviewer", None, &parent, "call-1", "review",
            )
            .expect("child");

        assert_eq!(service.project(&child).as_deref(), Some("/proj"));
        assert_eq!(service.agent_id(&child).as_deref(), Some("reviewer"));
        assert_eq!(service.status(&child), Some(SessionStatus::Idle));
        assert_eq!(service.descendant_session_ids(&parent), vec![child.clone()]);
        let meta = service.meta(&child).expect("child meta");
        assert_eq!(meta.parent_session_id.as_deref(), Some(parent.as_str()));
        assert_eq!(meta.parent_call_id.as_deref(), Some("call-1"));
        assert_eq!(meta.responsibility, "review");
        assert!(service.cached_progress(&child).is_none());
        assert!(service.latest_turn_end(&child).expect("turn end").is_none());
    }

    #[tokio::test]
    async fn ui_facts_delegate_without_projection_state() {
        let (_dir, manager, service) = fixture();
        let session = manager
            .open_session_sync("/proj", "default", None)
            .expect("session");
        assert_eq!(service.entry_wire_seq_cursor(&session).1, 0);
        assert!(
            service
                .list_sessions()
                .expect("sessions")
                .iter()
                .any(|row| row.id == session)
        );
        assert!(service.pending_messages_snapshot(&session).is_empty());

        let folder = service
            .create_session_folder("Inbox", false)
            .expect("folder");
        assert!(
            service
                .list_folders()
                .expect("folders")
                .iter()
                .any(|row| row.id == folder)
        );
        service
            .set_session_folder(&session, Some(&folder))
            .expect("move");
        service
            .set_session_title(&session, Some("Renamed"))
            .expect("rename");
        let meta = service.meta(&session).expect("meta");
        assert_eq!(meta.folder_id.as_deref(), Some(folder.as_str()));
        assert_eq!(meta.title.as_deref(), Some("Renamed"));
        assert_eq!(service.list_sessions().expect("sessions").len(), 1);
    }

    #[tokio::test]
    async fn revert_is_one_business_operation_with_pending_cleanup() {
        let (_dir, manager, service) = fixture();
        let session = manager
            .open_session_sync("/proj", "default", None)
            .expect("session");
        manager
            .insert_detail_rows(
                &session,
                &[
                    crate::types::user_text("first"),
                    crate::types::user_text("second"),
                ],
            )
            .expect("rows");
        let events = manager.data().events_blocking(&session).expect("events");
        let anchor = events
            .iter()
            .find(|event| event.event_type == crate::session::EventType::ItemUser)
            .expect("user anchor")
            .seq as u64;
        service
            .enqueue_user_input(&session, UserInput::text("queued"))
            .expect("pending");
        assert_eq!(service.pending_messages_snapshot(&session).len(), 1);

        let guard = service
            .revert_to_user_anchor(&session, anchor)
            .expect("revert");
        assert!(
            manager
                .reserve_turn(&session, "blocked".into(), 1, "default", "/proj")
                .is_err(),
            "the revert lease must block a new turn until the caller refreshes"
        );
        drop(guard);

        assert!(service.pending_messages_snapshot(&session).is_empty());
        assert!(
            service.entry_wire_seq_cursor(&session).0 <= i64::try_from(anchor).expect("anchor")
        );
    }

    #[tokio::test]
    async fn delete_tree_uses_the_same_manager_and_cleanup() {
        let (_dir, manager, service) = fixture();
        let parent = manager
            .open_session_sync("/proj", "default", None)
            .expect("parent");
        let child = service
            .open_child_session_with_responsibility(
                "/proj", "reviewer", None, &parent, "call-2", "review",
            )
            .expect("child");
        let removed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&removed);
        service
            .delete_session_tree(
                &move |id: &str| seen.lock().unwrap().push(id.to_string()),
                &child,
            )
            .await
            .expect("delete");
        assert_eq!(*removed.lock().unwrap(), vec![child.clone()]);
        assert!(service.meta(&child).is_err());
    }
}
