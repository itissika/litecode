use litecode::client_protocol::permission_bridge::{PendingPermission, WsPermissionBridge};
use litecode::client_protocol::protocol::PermissionRequest;
use litecode::permission::{self, PermissionSink};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[test]
fn server_hello_notification_serializes_correctly() {
    use litecode::client_protocol::project;
    let msg = project::server_hello(
        "0.1.0".into(),
        "dev".into(),
        "sess".into(),
        "/proj".into(),
        "ws-id".into(),
        0,
        "default".into(),
        vec![litecode::client_protocol::protocol::PrimaryAgentInfo {
            id: "default".into(),
            description: String::new(),
        }],
        "openai".into(),
        vec![],
    );
    let json = serde_json::to_string(&msg).expect("json");
    assert!(json.contains("server/hello"));
    assert!(json.contains("workspace_id"));
    assert!(json.contains("jsonrpc"));
    assert!(json.contains("2.0"));
}

#[tokio::test]
async fn permission_channel_round_trip() {
    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel::<PendingPermission>();
    let (reply_tx, reply_rx) = oneshot::channel();

    perm_tx
        .send(PendingPermission {
            session_id: "s1".into(),
            agent_name: "default".into(),
            turn_id: "t1".into(),
            request_id: "r1".into(),
            tool: "bash".into(),
            rule_id: "default".into(),
            summary: "ls".into(),
            reply_tx,
        })
        .expect("send");

    let perm = perm_rx.recv().await.expect("recv");
    assert_eq!(perm.tool, "bash");
    perm.reply_tx
        .send(permission::AskOutcome::Allow { always: false })
        .ok();

    let reply = permission::blocking_wait_oneshot(reply_rx);
    assert_eq!(reply, Some(permission::AskOutcome::Allow { always: false }));
}

#[test]
fn permission_request_serializes_request_id() {
    let req = PermissionRequest {
        session_id: "s1".into(),
        turn_id: "turn-1".into(),
        request_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        tool: "bash".into(),
        rule_id: "default".into(),
        summary: "ls".into(),
    };
    let json = serde_json::to_string(&req).expect("json");
    assert!(json.contains("request_id"));
    assert!(json.contains("550e8400-e29b-41d4-a716-446655440000"));
}

#[tokio::test]
async fn ws_permission_bridge_assigns_uuid_request_id() {
    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel::<PendingPermission>();
    let sink: Arc<dyn PermissionSink> =
        Arc::new(WsPermissionBridge::new("s1", "turn-1", "default", perm_tx));

    let sink2 = sink.clone();
    let ask = tokio::task::spawn_blocking(move || {
        sink2.ask_permission(
            "bash",
            "default",
            "ls",
            &tokio_util::sync::CancellationToken::new(),
        )
    });

    let perm = tokio::time::timeout(std::time::Duration::from_secs(2), perm_rx.recv())
        .await
        .expect("timeout")
        .expect("recv");
    assert_eq!(perm.turn_id, "turn-1");
    assert_eq!(perm.agent_name, "default");
    assert_eq!(perm.session_id, "s1");
    assert_eq!(perm.tool, "bash");
    assert!(
        uuid::Uuid::parse_str(&perm.request_id).is_ok(),
        "request_id must be a valid UUID, got {}",
        perm.request_id
    );
    perm.reply_tx
        .send(permission::AskOutcome::Allow { always: false })
        .ok();
    let reply = ask.await.expect("join");
    assert_eq!(reply, permission::AskOutcome::Allow { always: false });
}

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
