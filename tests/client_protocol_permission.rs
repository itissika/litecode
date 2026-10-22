use litecode::client_protocol::permission_bridge::{PendingPermission, WsPermissionBridge};
use litecode::client_protocol::protocol::PermissionRequest;
use litecode::permission::{self, AskKind, AskReply, PermissionSink};
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::test]
async fn ws_permission_bridge_abort_on_cancel_is_not_deny() {
    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel::<PendingPermission>();
    let sink: Arc<dyn PermissionSink> =
        Arc::new(WsPermissionBridge::new("s1", "turn-1", "default", perm_tx));
    let cancel = tokio_util::sync::CancellationToken::new();
    let sink2 = sink.clone();
    let cancel2 = cancel.clone();
    let ask = tokio::task::spawn_blocking(move || {
        sink2.ask_permission("bash", "default", "ls", &cancel2)
    });

    let perm = tokio::time::timeout(std::time::Duration::from_secs(2), perm_rx.recv())
        .await
        .expect("timeout")
        .expect("recv");
    assert_eq!(perm.tool, "bash");
    cancel.cancel();
    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), ask)
        .await
        .expect("ask should unblock")
        .expect("join");
    assert_eq!(reply, permission::AskOutcome::Aborted);
}

#[tokio::test]
async fn ws_bridge_approval_round_trip_with_free_text() {
    use litecode::permission::AskPrompt;

    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel::<PendingPermission>();
    let sink: Arc<dyn PermissionSink> = Arc::new(WsPermissionBridge::new(
        "s-appr", "t-appr", "default", perm_tx,
    ));

    let sink2 = sink.clone();
    let ask = tokio::task::spawn_blocking(move || {
        sink2.ask(
            &AskPrompt::approval("plan", "default", "Write plan for X"),
            &tokio_util::sync::CancellationToken::new(),
        )
    });

    let perm = tokio::time::timeout(std::time::Duration::from_secs(2), perm_rx.recv())
        .await
        .expect("timeout")
        .expect("recv");
    assert_eq!(perm.kind, AskKind::Approval);
    assert!(perm.free_text, "approval prompts request free_text");
    assert!(perm.questions.is_empty());

    let wire_req = PermissionRequest {
        session_id: perm.session_id.clone(),
        turn_id: perm.turn_id.clone(),
        request_id: perm.request_id.clone(),
        tool: perm.tool.clone(),
        rule_id: perm.rule_id.clone(),
        summary: perm.summary.clone(),
        kind: perm.kind,
        free_text: perm.free_text,
        options: Vec::new(),
        multi_select: false,
        questions: Vec::new(),
    };
    let req_json = serde_json::to_value(&wire_req).expect("ser");
    assert_eq!(req_json["kind"], "approval");
    assert_eq!(req_json["free_text"], true);
    assert!(
        req_json.get("questions").is_none()
            || req_json["questions"]
                .as_array()
                .is_none_or(|a| a.is_empty()),
        "approval omits empty questions on wire when skip_serializing_if"
    );

    // Reject with opinion.
    perm.reply_tx
        .send(AskReply::from_grant(
            false,
            false,
            Some("  too broad  ".into()),
            Vec::new(),
            Default::default(),
        ))
        .ok();
    let reply = ask.await.expect("join");
    assert_eq!(reply.outcome, permission::AskOutcome::Deny);
    assert_eq!(reply.free_text.as_deref(), Some("too broad"));
}
