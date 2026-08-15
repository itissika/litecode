use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::permission::{AskOutcome, PermissionSink, blocking_wait_oneshot_cancellable};

pub struct PendingPermission {
    pub session_id: String,
    pub agent_name: String,
    pub turn_id: String,
    pub request_id: String,
    pub tool: String,
    pub rule_id: String,
    pub summary: String,
    pub reply_tx: oneshot::Sender<AskOutcome>,
}

pub struct WsPermissionBridge {
    session_id: String,
    turn_id: String,
    agent_name: String,
    tx: mpsc::UnboundedSender<PendingPermission>,
}

impl WsPermissionBridge {
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        agent_name: impl Into<String>,
        tx: mpsc::UnboundedSender<PendingPermission>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            agent_name: agent_name.into(),
            tx,
        }
    }
}

impl PermissionSink for WsPermissionBridge {
    fn ask_permission(
        &self,
        tool: &str,
        rule_id: &str,
        summary: &str,
        cancel: &CancellationToken,
    ) -> AskOutcome {
        tracing::info!(tool = %tool, rule_id = %rule_id, "permission waiting for ui");
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .tx
            .send(PendingPermission {
                session_id: self.session_id.clone(),
                agent_name: self.agent_name.clone(),
                turn_id: self.turn_id.clone(),
                request_id: Uuid::new_v4().to_string(),
                tool: tool.to_string(),
                rule_id: rule_id.to_string(),
                summary: summary.to_string(),
                reply_tx,
            })
            .is_err()
        {
            tracing::warn!(tool = %tool, "no active websocket for permission");
            return AskOutcome::Deny;
        }
        let reply = blocking_wait_oneshot_cancellable(reply_rx, Some(cancel.clone()))
            .unwrap_or(AskOutcome::Aborted);
        tracing::info!(
            tool = %tool,
            rule_id = %rule_id,
            ?reply,
            "permission ui replied"
        );
        reply
    }
}

pub fn ws_permission_sink(
    session_id: &str,
    turn_id: &str,
    agent_name: &str,
    perm_tx: &mpsc::UnboundedSender<PendingPermission>,
) -> std::sync::Arc<dyn PermissionSink> {
    std::sync::Arc::new(WsPermissionBridge::new(
        session_id,
        turn_id,
        agent_name,
        perm_tx.clone(),
    ))
}
