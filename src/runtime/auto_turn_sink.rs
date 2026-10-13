//! WP-090 K6: one place deciding which permission sink an auto-turn uses.

use std::sync::Arc;

use crate::permission::{PermissionSink, deny_permission_sink};
use crate::session::SessionManager;

/// Child sessions (any `parent_session_id`) always get the deny sink and never
/// read `last_permission_sink`. Primary sessions keep the last UI sink (Q1).
pub(crate) fn auto_turn_permission_sink(
    sessions: &SessionManager,
    session_id: &str,
) -> Arc<dyn PermissionSink> {
    let is_child = sessions
        .reader()
        .meta_blocking(session_id)
        .ok()
        .and_then(|meta| meta.parent_session_id)
        .is_some_and(|parent| !parent.is_empty());
    if is_child {
        return deny_permission_sink();
    }
    sessions
        .last_permission_sink(session_id)
        .unwrap_or_else(deny_permission_sink)
}
