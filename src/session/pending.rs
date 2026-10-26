//! WP-186: pending user-gate asks (ask_user | permission | approval).
//!
//! B: this is a live-turn sidecar, not cross-process truth. A row is written
//! only while the owning turn is live in this process; the writer clears all
//! rows at startup and the manager clears a turn's rows when it ends. It
//! survives a disconnect (same process) so seal repair does not close a call
//! whose `ask_user` may still be answered after reconnect.

use serde::{Deserialize, Serialize};

use crate::permission::{AskKind, AskOption, AskQuestion};

/// One durable pending permission. Wire snapshot projects the fields that
/// match `agent/permission_request` 1:1; `call_id` stays server-side for seal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPermissionRecord {
    pub session_id: String,
    pub turn_id: String,
    pub request_id: String,
    pub tool: String,
    pub rule_id: String,
    pub summary: String,
    /// Required on write (WP-186). Never rely on AskKind's serde default here.
    pub kind: AskKind,
    #[serde(default)]
    pub free_text: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<AskQuestion>,
    /// Matching transcript `function_call.call_id` when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

impl PendingPermissionRecord {
    /// Tool names that are always permission-class. Used only as a conservative
    /// fallback when the sidecar lookup itself fails.
    pub fn is_permission_class_tool_name(name: &str) -> bool {
        matches!(name, "ask_user" | "ask-user")
    }

    pub fn is_permission_class_kind(kind: AskKind) -> bool {
        matches!(
            kind,
            AskKind::Permission | AskKind::Approval | AskKind::AskUser
        )
    }
}
