//! WP-186: durable pending user-gate asks (ask_user | permission | approval).
//!
//! Transcript keeps the intentional unsealed `item/tool_call`; this sidecar
//! holds enough metadata to re-hydrate `pending_permissions` on snapshot/attach
//! and to divert startup seal away from permission-class tails.

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
    /// Tool names that are always permission-class even without a sidecar row
    /// (R4 name-fallback for old DBs).
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
