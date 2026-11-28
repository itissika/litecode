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

    /// Ordinary session identity switch. Rejects bot-owned sessions; only the
    /// bot lifecycle / startup reconciliation may write its one mirror.
    pub(crate) async fn set_agent_id(&self, session_id: &str, agent_id: String) -> Result<()> {
        if let Some(owner) = self.bot_owner_for_session(session_id) {
            return Err(crate::types::LitecodeError::BotOwnedSession(owner.bot_id));
        }
        let target_is_bot = self.is_bot_id(&agent_id);
        if target_is_bot {
            return Err(crate::types::LitecodeError::BotOwnedSession(agent_id));
        }
        self.sessions.set_agent_id(session_id, agent_id).await
    }

    /// Bot lifecycle write: bind the session's agent_id mirror to its bot id.
    ///
    /// Sync: only `BotService::new` startup reconciliation calls it, and that
    /// constructor runs outside any Tokio runtime.
    pub(crate) fn bind_bot_agent_id(&self, session_id: &str, bot_id: String) -> Result<()> {
        self.sessions.set_agent_id_sync(session_id, bot_id)
    }

    pub(crate) fn session_model_id(&self, session_id: &str) -> Option<String> {
        self.sessions.session_model_id(session_id)
    }

    pub(crate) async fn set_session_model_id(
        &self,
        session_id: &str,
        model_id: Option<String>,
    ) -> Result<()> {
        self.sessions.set_session_model_id(session_id, model_id).await
    }

    pub(crate) async fn set_thinking_tier(
        &self,
        session_id: &str,
        tier: crate::platform_knobs::ThinkingTier,
    ) -> Result<()> {
        self.sessions.set_thinking_tier(session_id, tier).await
    }

    pub(crate) async fn set_context_mode(
        &self,
        session_id: &str,
        mode: crate::platform_knobs::ContextMode,
    ) -> Result<()> {
        self.sessions.set_context_mode(session_id, mode).await
    }

    /// Blocking twin of [`Self::set_context_mode`]. Only for callers that
    /// provably run outside Tokio (`CliConversation::open` before `block_on`).
    pub(crate) fn set_context_mode_sync(
        &self,
        session_id: &str,
        mode: crate::platform_knobs::ContextMode,
    ) -> Result<()> {
        self.sessions.set_context_mode_sync(session_id, mode)
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

    /// Session -> agent identity with the bot product layer first. A bot
    /// session's agent id is the bot id; normal sessions still validate the
    /// settings profile.
    pub(crate) fn resolve_primary_agent_for_session(
        &self,
        runtime: &crate::runtime::RuntimeHandle,
        session_id: &str,
        default_primary: &str,
    ) -> Result<String> {
        if let Some(owner) = self.bot_owner_for_session(session_id) {
            return Ok(owner.bot_id);
        }
        self.resolve_primary_agent(session_id, default_primary, &runtime.resolved)
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

    // Async twins: `session/list` runs on a Tokio worker and must not park it on
    // SQLite, so it takes these instead of the `_blocking` forms above.
    pub(crate) async fn list_folders_async(&self) -> Result<Vec<SessionFolderRow>> {
        self.sessions.data().list_folders().await
    }

    pub(crate) async fn list_sessions_async(&self) -> Result<Vec<SessionListRow>> {
        self.sessions.data().list_sessions().await
    }

    pub(crate) async fn has_conversation_rows_async(&self, session_id: &str) -> Result<bool> {
        self.sessions.data().has_conversation_rows(session_id).await
    }

    pub(crate) async fn set_session_title(
        &self,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<()> {
        self.sessions.data().set_session_title(session_id, title).await
    }

    pub(crate) async fn set_session_folder(
        &self,
        session_id: &str,
        folder_id: Option<&str>,
    ) -> Result<()> {
        self.sessions
            .data()
            .set_session_folder(session_id, folder_id)
            .await
    }

    pub(crate) async fn create_session_folder(&self, name: &str, bot: bool) -> Result<String> {
        let prefix = if bot {
            crate::bot::BOT_FOLDER_PREFIX
        } else {
            ""
        };
        self.sessions.data().create_session_folder(name, prefix).await
    }

    pub(crate) async fn rename_session_folder(&self, id: &str, name: &str) -> Result<()> {
        self.sessions.data().rename_session_folder(id, name).await
    }

    pub(crate) async fn delete_session_folder(&self, id: &str) -> Result<()> {
        self.sessions.data().delete_session_folder(id).await
    }

    pub(crate) async fn reorder_session_folders(&self, ids: &[String]) -> Result<()> {
        self.sessions.data().reorder_session_folders(ids).await
    }

    // ── Session actions ───────────────────────────────────────────────────

    pub(crate) async fn ensure_entry(&self, session_id: &str) -> Result<()> {
        self.sessions.ensure_entry(session_id).await
    }

    pub(crate) async fn open_child_session_with_responsibility(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: &str,
        parent_call_id: &str,
        responsibility: &str,
    ) -> Result<String> {
        self.sessions
            .open_child_session_with_responsibility(
                project,
                agent_id,
                model_id,
                parent_session_id,
                parent_call_id,
                responsibility,
            )
            .await
    }

    pub(crate) async fn remove_session(&self, session_id: &str) -> Result<Vec<String>> {
        if let Some(owner) = self.bot_owner_for_session(session_id) {
            return Err(crate::types::LitecodeError::BotOwnedSession(owner.bot_id));
        }
        self.sessions.remove_session(session_id).await
    }

    /// Public session delete: bot-owned sessions are refused here, not only in
    /// the websocket controller. The bot lifecycle uses
    /// [`Self::delete_bot_session_tree`].
    pub async fn delete_session_tree<C: RemovedSessionCleanup + ?Sized>(
        &self,
        cleanup: &C,
        session_id: &str,
    ) -> std::result::Result<DeletedSessions, DeleteSessionError> {
        if let Some(owner) = self.bot_owner_for_session(session_id) {
            return Err(DeleteSessionError::BotOwned {
                bot_id: owner.bot_id,
                bot_name: owner.name,
            });
        }
        crate::session::delete::delete_session_tree(&self.sessions, cleanup, session_id).await
    }

    /// Bot lifecycle delete. Only `BotService::delete` may use this.
    pub(crate) async fn delete_bot_session_tree<C: RemovedSessionCleanup + ?Sized>(
        &self,
        cleanup: &C,
        session_id: &str,
    ) -> std::result::Result<DeletedSessions, DeleteSessionError> {
        crate::session::delete::delete_session_tree_for_bot(&self.sessions, cleanup, session_id)
            .await
    }
}
