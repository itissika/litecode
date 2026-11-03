//! Observation: session facts, history and usage reads.
//!
//! This is the business read surface. Projection buffers, cursor transport and
//! runtime measurements live in [`super::support`].
//!
//! Candidate contract operations: `meta`, `status`, `transcript_file`,
//! `latest_turn_result`, `meter`, `provider_usage_series`, `workspace_usage`.

use super::*;

impl SessionService {
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

    pub(crate) fn is_turn_running_blocking(&self, session_id: &str) -> bool {
        self.sessions.is_turn_running_blocking(session_id)
    }

    pub(crate) fn thinking_tier(
        &self,
        session_id: &str,
    ) -> Option<crate::platform_knobs::ThinkingTier> {
        self.sessions.thinking_tier(session_id)
    }

    pub(crate) fn context_mode(
        &self,
        session_id: &str,
    ) -> Option<crate::platform_knobs::ContextMode> {
        self.sessions.context_mode(session_id)
    }

    pub(crate) fn meter(
        &self,
        session_id: &str,
    ) -> Result<crate::session::data::sqlite::session::SessionContextMeter> {
        self.sessions.data().meter_blocking(session_id)
    }

    pub(crate) fn provider_usage_rollup(
        &self,
        session_id: &str,
    ) -> Result<crate::session::data::sqlite::session::ProviderUsageRollup> {
        self.sessions.provider_usage_rollup(session_id)
    }

    pub(crate) fn compaction_state(&self, session_id: &str) -> CompactionStateSnapshot {
        self.sessions.compaction_state_blocking(session_id)
    }

    pub(crate) fn child_session_id_for_call(
        &self,
        parent_session_id: &str,
        parent_call_id: &str,
    ) -> Option<String> {
        self.sessions
            .child_session_id_for_call(parent_session_id, parent_call_id)
    }

    pub(crate) fn find_function_call_event(
        &self,
        session_id: &str,
        call_id: &str,
    ) -> Option<SessionEvent> {
        self.sessions.find_function_call_event(session_id, call_id)
    }

    pub(crate) fn is_child_session(&self, session_id: &str) -> bool {
        self.sessions.is_child_session(session_id)
    }

    pub(crate) fn parent_session_id_of(&self, session_id: &str) -> Option<String> {
        self.sessions.parent_session_id_of(session_id)
    }

    pub(crate) async fn is_turn_running(&self, session_id: &str) -> bool {
        self.sessions.is_turn_running(session_id).await
    }

    pub(crate) fn is_session_busy_blocking(&self, session_id: &str) -> bool {
        self.sessions.is_session_busy_blocking(session_id)
    }

    // ── Workspace / usage observation ─────────────────────────────────────

    pub(crate) fn workspace_usage(&self) -> Result<WorkspaceUsage> {
        self.sessions.workspace_usage()
    }

    pub(crate) fn provider_usage_series(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<ProviderUsageSeries> {
        self.sessions.provider_usage_series(session_id, limit)
    }

    pub(crate) fn working_set_state(&self, session_id: &str) -> Result<WorkingSetState> {
        self.sessions.data().working_set_state_blocking(session_id)
    }
}
