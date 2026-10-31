use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use super::{AskKind, AskOption, AskQuestion, AskReply};

/// One human Ask on its way to whichever connection currently owns the UI.
///
/// Kept outside the WebSocket adapter so the session record can hold a
/// connection-independent route without depending on client_protocol.
pub struct PendingPermission {
    pub session_id: String,
    pub agent_name: String,
    pub turn_id: String,
    pub request_id: String,
    pub tool: String,
    pub rule_id: String,
    pub summary: String,
    pub kind: AskKind,
    pub free_text: bool,
    pub options: Vec<AskOption>,
    pub multi_select: bool,
    pub questions: Vec<AskQuestion>,
    pub reply_tx: oneshot::Sender<AskReply>,
}

struct BoundRoute {
    owner: String,
    tx: mpsc::UnboundedSender<PendingPermission>,
}

/// Session-level permission route.
///
/// A running turn keeps one bridge for its whole life. The bridge must not
/// capture a connection's sender forever: after a reconnect the new connection
/// rebinds this route, and the next Ask from the same running turn reaches it.
#[derive(Clone, Default)]
pub struct PermissionRoute {
    inner: Arc<Mutex<Option<BoundRoute>>>,
}

impl PermissionRoute {
    pub fn bind(&self, owner: impl Into<String>, tx: mpsc::UnboundedSender<PendingPermission>) {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(BoundRoute {
            owner: owner.into(),
            tx,
        });
    }

    /// Clear only this owner's binding. A stale connection must not steal the
    /// route from a newer connection that has already rebound it.
    pub fn unbind(&self, owner: &str) {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|bound| bound.owner == owner) {
            *slot = None;
        }
    }

    /// Send one Ask to the currently bound connection. A closed receiver means
    /// the connection is gone; the caller maps that to Aborted, never Deny.
    pub fn send(&self, prompt: PendingPermission) -> bool {
        let tx = {
            let slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            slot.as_ref().map(|bound| bound.tx.clone())
        };
        let Some(tx) = tx else {
            return false;
        };
        tx.send(prompt).is_ok()
    }
}

impl From<mpsc::UnboundedSender<PendingPermission>> for PermissionRoute {
    fn from(tx: mpsc::UnboundedSender<PendingPermission>) -> Self {
        let route = Self::default();
        route.bind("direct", tx);
        route
    }
}
