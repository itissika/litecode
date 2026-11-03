//! Business: permissions and pending interactions.
//!
//! Queued prompts and durable permission records. Permission routing (which
//! connection owns the sink) is process plumbing in [`super::support`].
//!
//! Candidate contract operations: `pending_messages_snapshot`,
//! `pending_permissions_snapshot`, `claim_pending_if_idle`,
//! `restore_pending_messages`, `upsert_pending_permission`,
//! `remove_pending_permission`.

use super::*;

impl SessionService {
    // ── Pending / permission facts ────────────────────────────────────────

    pub(crate) fn pending_messages_snapshot(&self, session_id: &str) -> Vec<PendingMessage> {
        self.sessions.pending_messages_snapshot(session_id)
    }

    pub(crate) fn pending_permissions_snapshot(
        &self,
        session_id: &str,
    ) -> Vec<PendingPermissionRecord> {
        self.sessions.pending_permissions_snapshot(session_id)
    }

    pub(crate) fn claim_pending_if_idle(&self, session_id: &str) -> Option<Vec<PendingMessage>> {
        self.sessions.claim_pending_if_idle(session_id)
    }

    pub(crate) fn restore_pending_messages(&self, session_id: &str, claimed: Vec<PendingMessage>) {
        self.sessions.restore_pending_messages(session_id, claimed);
    }

    pub(crate) fn remove_pending_message(&self, session_id: &str, id: &str) -> bool {
        self.sessions.remove_pending_message(session_id, id)
    }

    pub(crate) fn upsert_pending_permission(&self, rec: PendingPermissionRecord) -> bool {
        self.sessions.upsert_pending_permission(rec)
    }

    pub(crate) fn remove_pending_permission(&self, session_id: &str, request_id: &str) {
        self.sessions
            .remove_pending_permission(session_id, request_id);
    }

    pub(crate) fn clear_pending_permissions(&self, session_id: &str) {
        self.sessions.clear_pending_permissions(session_id);
    }

    pub(crate) fn clear_pending_permission_if_turn_dead(&self, request_id: &str) -> Option<String> {
        self.sessions
            .clear_pending_permission_if_turn_dead(request_id)
    }

    pub(crate) fn latest_unanswered_call_id_for_tool(
        &self,
        session_id: &str,
        tool_name: &str,
    ) -> Option<String> {
        self.sessions
            .latest_unanswered_call_id_for_tool(session_id, tool_name)
    }
}
