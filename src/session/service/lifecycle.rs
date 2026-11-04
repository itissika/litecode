//! Business: session lifecycle.
//!
//! Create/list/open, metadata, title/folders, model/config controls and delete.
//! Runtime construction, permission routing and process plumbing are in
//! [`super::support`].
//!
//! Candidate contract operations: `open_session`,
//! `open_child_session_with_responsibility`, `list_sessions`,
//! `set_session_title`, `set_session_folder`, `delete_session_tree`.

use super::*;

impl SessionService {
    // ── UI session controls ───────────────────────────────────────────────

    pub(crate) async fn open_session(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
    ) -> Result<String> {
        self.sessions
            .open_session(project, agent_id, model_id)
            .await
    }

    pub(crate) fn set_agent_id(&self, session_id: &str, agent_id: String) -> Result<()> {
        self.sessions.set_agent_id(session_id, agent_id)
    }

    pub(crate) fn session_model_id(&self, session_id: &str) -> Option<String> {
        self.sessions.session_model_id(session_id)
    }

    /// Persist the sticky session model without appending a control event.
    /// Bot capability edits use this to keep the bot's default session on the
    /// model the owned profile now declares.
    pub(crate) fn set_session_model_id(
        &self,
        session_id: &str,
        model_id: Option<String>,
    ) -> Result<()> {
        self.sessions.set_session_model_id(session_id, model_id)
    }

    pub(crate) fn set_session_model_with_change(
        &self,
        session_id: &str,
        model_id: Option<String>,
        record: &serde_json::Value,
    ) -> Result<Seq> {
        self.sessions
            .set_session_model_with_change(session_id, model_id, record)
    }

    pub(crate) fn set_thinking_tier(
        &self,
        session_id: &str,
        tier: crate::platform_knobs::ThinkingTier,
    ) -> Result<()> {
        self.sessions.set_thinking_tier(session_id, tier)
    }

    pub(crate) fn set_context_mode(
        &self,
        session_id: &str,
        mode: crate::platform_knobs::ContextMode,
    ) -> Result<()> {
        self.sessions.set_context_mode(session_id, mode)
    }

    pub(crate) fn resolve_primary_agent(
        &self,
        session_id: &str,
        default_primary: &str,
        resolved: &ResolvedConfig,
    ) -> Result<String> {
        self.sessions
            .resolve_primary_agent(session_id, default_primary, resolved)
    }

    pub(crate) fn list_folders(&self) -> Result<Vec<SessionFolderRow>> {
        self.sessions.data().list_folders_blocking()
    }

    pub(crate) fn list_sessions(&self) -> Result<Vec<SessionListRow>> {
        self.sessions.data().list_sessions_blocking()
    }

    pub(crate) fn has_conversation_rows(&self, session_id: &str) -> Result<bool> {
        self.sessions
            .data()
            .has_conversation_rows_blocking(session_id)
    }

    pub(crate) fn set_session_title(&self, session_id: &str, title: Option<&str>) -> Result<()> {
        self.sessions
            .data()
            .set_session_title_blocking(session_id, title)
    }

    pub(crate) fn set_session_folder(
        &self,
        session_id: &str,
        folder_id: Option<&str>,
    ) -> Result<()> {
        self.sessions
            .data()
            .set_session_folder_blocking(session_id, folder_id)
    }

    pub(crate) fn create_session_folder(&self, name: &str) -> Result<String> {
        self.sessions.data().create_session_folder_blocking(name)
    }

    pub(crate) fn rename_session_folder(&self, id: &str, name: &str) -> Result<()> {
        self.sessions
            .data()
            .rename_session_folder_blocking(id, name)
    }

    pub(crate) fn delete_session_folder(&self, id: &str) -> Result<()> {
        self.sessions.data().delete_session_folder_blocking(id)
    }

    pub(crate) fn reorder_session_folders(&self, ids: &[String]) -> Result<()> {
        self.sessions.data().reorder_session_folders_blocking(ids)
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
}
