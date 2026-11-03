//! Phase-1 session facade.
//!
//! `SessionService` is deliberately a thin delegate over the process-owned
//! `SessionManager`. It exists so subagent tools depend on one stable facade
//! instead of the manager's session primitives. This phase adds no policy:
//! no auth, idempotency, replay, caching, aggregation, retry, or second store.
//! Keep that boundary until the UI/external phases explicitly widen it.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::permission::PermissionSink;
use crate::runtime::observer::InternalEvent;
use crate::runtime::{RuntimeHandle, TurnHandle, TurnInput, TurnOptions, spawn_turn};
use crate::session::delete::{DeleteSessionError, DeletedSessions, RemovedSessionCleanup};
use crate::session::live::{LifecycleEvent, TurnProgress};
use crate::session::manager::{
    CancelOutcome, FollowUpOutcome, PendingMessage, SessionManager, SessionStatus,
};
use crate::session::model::{SessionMeta, TurnResult};
use crate::session::transcript_file::TranscriptFile;
use crate::types::{Result, StopOrigin, UserInput};

/// Process-level facade over the one `SessionManager`.
#[derive(Clone)]
pub struct SessionService {
    sessions: Arc<SessionManager>,
}

impl SessionService {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self { sessions }
    }

    // ── Session facts ─────────────────────────────────────────────────────

    pub(crate) fn project(&self, session_id: &str) -> Option<String> {
        self.sessions.project(session_id)
    }

    pub(crate) fn agent_id(&self, session_id: &str) -> Option<String> {
        self.sessions.agent_id(session_id)
    }

    pub(crate) fn meta(&self, session_id: &str) -> Result<SessionMeta> {
        self.sessions.reader().meta_blocking(session_id)
    }

    pub(crate) fn transcript_file(&self, session_id: &str) -> Result<TranscriptFile> {
        self.sessions.reader().transcript_file_blocking(session_id)
    }

    pub(crate) fn status(&self, session_id: &str) -> Option<SessionStatus> {
        self.sessions.session_status(session_id)
    }

    pub(crate) fn descendant_session_ids(&self, session_id: &str) -> Vec<String> {
        self.sessions.descendant_session_ids(session_id)
    }

    pub(crate) fn cached_progress(&self, session_id: &str) -> Option<TurnProgress> {
        self.sessions.get_cached_progress(session_id)
    }

    pub(crate) fn latest_turn_result(&self, session_id: &str) -> Result<Option<TurnResult>> {
        self.sessions.data().latest_turn_result_blocking(session_id)
    }

    pub(crate) fn turn_result(&self, session_id: &str, turn_id: &str) -> Result<TurnResult> {
        self.sessions
            .data()
            .turn_result_blocking(session_id, turn_id)
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn latest_turn_end(
        &self,
        session_id: &str,
    ) -> Result<Option<(String, String, Option<StopOrigin>)>> {
        self.sessions.data().latest_turn_end_blocking(session_id)
    }

    pub(crate) fn latest_turn_end_reason(
        &self,
        session_id: &str,
    ) -> Result<Option<(String, String)>> {
        self.sessions
            .data()
            .latest_turn_end_reason_blocking(session_id)
    }

    pub(crate) fn subscribe_lifecycle(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.sessions.subscribe_lifecycle()
    }

    pub(crate) fn is_turn_running_blocking(&self, session_id: &str) -> bool {
        self.sessions.is_turn_running_blocking(session_id)
    }

    // ── Session actions ───────────────────────────────────────────────────

    pub(crate) async fn ensure_entry(&self, session_id: &str) -> Result<()> {
        self.sessions.ensure_entry(session_id).await
    }

    pub(crate) fn open_child_session_with_responsibility(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: &str,
        parent_call_id: &str,
        responsibility: &str,
    ) -> Result<String> {
        self.sessions.open_child_session_with_responsibility(
            project,
            agent_id,
            model_id,
            parent_session_id,
            parent_call_id,
            responsibility,
        )
    }

    pub(crate) fn publish_internal(&self, session_id: &str, event: InternalEvent) -> bool {
        self.sessions.publish_internal(session_id, event)
    }

    pub(crate) fn enqueue_follow_up(
        &self,
        session_id: &str,
        input: UserInput,
    ) -> Result<FollowUpOutcome> {
        self.sessions.enqueue_follow_up(session_id, input)
    }

    pub(crate) fn enqueue_user_input(
        &self,
        session_id: &str,
        input: UserInput,
    ) -> Result<PendingMessage> {
        self.sessions.enqueue_user_input(session_id, input)
    }

    pub(crate) fn cancel_turn(&self, session_id: &str, by: StopOrigin) -> CancelOutcome {
        self.sessions.cancel_turn_sync(session_id, by)
    }

    pub(crate) fn cancel_and_follow_up(
        &self,
        session_id: &str,
        input: UserInput,
        by: StopOrigin,
    ) -> Result<(CancelOutcome, FollowUpOutcome)> {
        self.sessions.cancel_and_follow_up(session_id, input, by)
    }

    pub(crate) fn remove_session(&self, session_id: &str) -> Result<Vec<String>> {
        self.sessions.remove_session(session_id)
    }

    pub(crate) async fn delete_session_tree<C: RemovedSessionCleanup + ?Sized>(
        &self,
        cleanup: &C,
        session_id: &str,
    ) -> std::result::Result<DeletedSessions, DeleteSessionError> {
        crate::session::delete::delete_session_tree(&self.sessions, cleanup, session_id).await
    }

    // ── Turn lifecycle ────────────────────────────────────────────────────

    pub(crate) fn reserve_turn(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<TurnProgress> {
        self.sessions
            .reserve_turn(session_id, turn_id, step_max, primary_agent, project)
    }

    pub(crate) fn release_turn_reservation(&self, session_id: &str, turn_id: &str) -> bool {
        self.sessions.release_turn_reservation(session_id, turn_id)
    }

    pub(crate) fn spawn_turn(
        &self,
        runtime: &RuntimeHandle,
        session_id: String,
        input: impl Into<TurnInput>,
        permission_sink: Arc<dyn PermissionSink>,
        turn_id: String,
        opts: TurnOptions,
    ) -> anyhow::Result<TurnHandle> {
        spawn_turn(
            runtime,
            session_id,
            Arc::clone(&self.sessions),
            input,
            permission_sink,
            turn_id,
            opts,
        )
    }

    pub(crate) async fn start_turn(
        &self,
        session_id: &str,
        handle: TurnHandle,
        primary_agent: &str,
        project: &str,
    ) -> Result<()> {
        self.sessions
            .start_turn(
                session_id,
                handle,
                primary_agent,
                project,
                Arc::clone(&self.sessions),
            )
            .await
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
