use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use crate::permission::PendingPermission;
use crate::permission::{
    AskOutcome, AskPrompt, AskReply, PermissionRoute, PermissionSink,
    blocking_wait_oneshot_cancellable,
};

pub struct WsPermissionBridge {
    session_id: String,
    turn_id: String,
    agent_name: String,
    route: PermissionRoute,
}

impl WsPermissionBridge {
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        agent_name: impl Into<String>,
        tx: mpsc::UnboundedSender<PendingPermission>,
    ) -> Self {
        Self::with_route(session_id, turn_id, agent_name, PermissionRoute::from(tx))
    }

    /// The bridge keeps session/turn identity and follows the connection that
    /// currently owns this session's permission route.
    pub fn with_route(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        agent_name: impl Into<String>,
        route: PermissionRoute,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            agent_name: agent_name.into(),
            route,
        }
    }
}

impl PermissionSink for WsPermissionBridge {
    fn ask(&self, prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
        let questions = prompt.effective_questions();
        tracing::info!(
            tool = %prompt.tool,
            rule_id = %prompt.rule_id,
            kind = ?prompt.kind,
            free_text = prompt.free_text,
            options = prompt.options.len(),
            multi_select = prompt.multi_select,
            questions = questions.len(),
            "permission waiting for ui"
        );
        let (reply_tx, reply_rx) = oneshot::channel();
        if !self.route.send(PendingPermission {
            session_id: self.session_id.clone(),
            agent_name: self.agent_name.clone(),
            turn_id: self.turn_id.clone(),
            request_id: Uuid::new_v4().to_string(),
            tool: prompt.tool.to_string(),
            rule_id: prompt.rule_id.to_string(),
            summary: prompt.summary.to_string(),
            kind: prompt.kind,
            free_text: prompt.free_text,
            options: prompt.options.to_vec(),
            multi_select: prompt.multi_select,
            questions,
            reply_tx,
        }) {
            // WP-186 R5: no WS must not look like user Skip (Deny).
            tracing::warn!(tool = %prompt.tool, "no active websocket for permission");
            return AskReply::from_outcome(AskOutcome::Aborted);
        }
        let reply = blocking_wait_oneshot_cancellable(reply_rx, Some(cancel.clone()))
            .unwrap_or_else(|| AskReply::from_outcome(AskOutcome::Aborted));
        tracing::info!(
            tool = %prompt.tool,
            rule_id = %prompt.rule_id,
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
    route: PermissionRoute,
) -> std::sync::Arc<dyn PermissionSink> {
    std::sync::Arc::new(WsPermissionBridge::with_route(
        session_id, turn_id, agent_name, route,
    ))
}
