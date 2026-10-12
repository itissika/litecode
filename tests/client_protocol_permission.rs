use litecode::client_protocol::permission_bridge::{PendingPermission, WsPermissionBridge};
use litecode::client_protocol::protocol::PermissionRequest;
use litecode::permission::{self, AskKind, AskReply, PermissionSink};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[test]
fn server_hello_notification_serializes_correctly() {
    use litecode::client_protocol::project;
    use litecode::client_protocol::protocol::{ModelInfo, PrimaryAgentInfo};
    let msg = project::server_hello(
        "0.1.0".into(),
        "dev".into(),
        "sess".into(),
        "/proj".into(),
        "ws-id".into(),
        0,
        "default".into(),
        vec![PrimaryAgentInfo {
            id: "default".into(),
            description: String::new(),
        }],
        vec![ModelInfo {
            id: "test/test-primary-model".into(),
            api_model_id: "test-primary-model".into(),
            label: "Test Primary".into(),
            context_window: 128_000,
            provider_id: "test".into(),
            modalities: vec!["text".into()],
        }],
    );
    let json = serde_json::to_string(&msg).expect("json");
    assert!(json.contains("server/hello"));
    assert!(json.contains("workspace_id"));
    assert!(json.contains("jsonrpc"));
    assert!(json.contains("2.0"));
    assert!(
        !json.contains("llm_ecosystem"),
        "server/hello must no longer carry the removed llm_ecosystem field: {json}"
    );
    let hello: litecode::client_protocol::protocol::ServerHello =
        serde_json::from_value(msg["params"].clone()).expect("hello params deserialize");
    assert_eq!(
        hello.models.len(),
        1,
        "models carries the active-model projection"
    );
    assert_eq!(hello.models[0].id, "test/test-primary-model");
    assert_eq!(hello.models[0].api_model_id, "test-primary-model");
    assert_eq!(hello.models[0].provider_id, "test");
    assert_eq!(hello.models[0].context_window, 128_000);
    assert_eq!(hello.host_os, std::env::consts::OS);
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
            kind: AskKind::Permission,
            free_text: false,
            options: Vec::new(),
            multi_select: false,
            questions: Vec::new(),
            reply_tx,
        })
        .expect("send");

    let perm = perm_rx.recv().await.expect("recv");
    assert_eq!(perm.tool, "bash");
    perm.reply_tx
        .send(AskReply::from_outcome(permission::AskOutcome::Allow {
            always: false,
        }))
        .ok();

    let reply = permission::blocking_wait_oneshot(reply_rx);
    assert_eq!(
        reply.map(|r| r.outcome),
        Some(permission::AskOutcome::Allow { always: false })
    );
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
        kind: AskKind::Permission,
        free_text: false,
        options: Vec::new(),
        multi_select: false,
        questions: Vec::new(),
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
        .send(AskReply::from_outcome(permission::AskOutcome::Allow {
            always: false,
        }))
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

#[tokio::test]
async fn ws_bridge_ask_user_questions_round_trip_with_answers_map() {
    use litecode::permission::{AskAnswer, AskOption, AskPrompt, AskQuestion};
    use std::collections::HashMap;

    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel::<PendingPermission>();
    let sink: Arc<dyn PermissionSink> = Arc::new(WsPermissionBridge::new(
        "s-ask", "t-ask", "default", perm_tx,
    ));

    let questions = vec![
        AskQuestion {
            id: "q_lang".into(),
            prompt: "Which language?".into(),
            options: vec![
                AskOption {
                    id: "zh".into(),
                    label: "中文".into(),
                },
                AskOption {
                    id: "en".into(),
                    label: "English".into(),
                },
            ],
            multi_select: false,
            free_text: true,
        },
        AskQuestion {
            id: "q_scope".into(),
            prompt: "Scope?".into(),
            options: vec![
                AskOption {
                    id: "a".into(),
                    label: "A".into(),
                },
                AskOption {
                    id: "b".into(),
                    label: "B".into(),
                },
            ],
            multi_select: true,
            free_text: false,
        },
    ];

    let sink2 = sink.clone();
    let ask = tokio::task::spawn_blocking(move || {
        sink2.ask(
            &AskPrompt::ask_user("ask_user", "default", "Card title", &questions),
            &tokio_util::sync::CancellationToken::new(),
        )
    });

    let perm = tokio::time::timeout(std::time::Duration::from_secs(2), perm_rx.recv())
        .await
        .expect("timeout")
        .expect("recv");
    assert_eq!(perm.kind, AskKind::AskUser);
    assert_eq!(perm.tool, "ask_user");
    assert_eq!(perm.summary, "Card title");
    assert_eq!(perm.questions.len(), 2);
    assert_eq!(perm.questions[0].id, "q_lang");
    assert_eq!(perm.questions[1].multi_select, true);

    // Outbound wire shape (agent/permission_request params).
    let wire_req = PermissionRequest {
        session_id: perm.session_id.clone(),
        turn_id: perm.turn_id.clone(),
        request_id: perm.request_id.clone(),
        tool: perm.tool.clone(),
        rule_id: perm.rule_id.clone(),
        summary: perm.summary.clone(),
        kind: perm.kind,
        free_text: perm.free_text,
        options: perm.options.clone(),
        multi_select: perm.multi_select,
        questions: perm.questions.clone(),
    };
    let req_json = serde_json::to_value(&wire_req).expect("ser");
    assert_eq!(req_json["kind"], "ask_user");
    assert_eq!(req_json["questions"][0]["id"], "q_lang");
    assert_eq!(req_json["questions"][1]["options"][1]["id"], "b");
    let round: PermissionRequest = serde_json::from_value(req_json).expect("de");
    assert_eq!(round.questions.len(), 2);

    // Inbound agent/permission grant params (answers map) — mirror ws.rs PermissionParams.
    #[derive(serde::Deserialize)]
    struct PermissionParams {
        request_id: String,
        tool: String,
        approved: bool,
        #[serde(default)]
        always: bool,
        #[serde(default)]
        free_text: Option<String>,
        #[serde(default)]
        selected: Vec<String>,
        #[serde(default)]
        answers: HashMap<String, AskAnswer>,
    }
    let grant_raw = serde_json::json!({
        "request_id": perm.request_id,
        "tool": "ask_user",
        "approved": true,
        "always": false,
        "answers": {
            "q_lang": { "selected": ["zh"], "free_text": "  prefer zh  " },
            "q_scope": { "selected": ["a", "b"] }
        }
    });
    let params: PermissionParams = serde_json::from_value(grant_raw).expect("grant de");
    assert_eq!(params.request_id, perm.request_id);
    assert_eq!(params.tool, "ask_user");
    assert!(params.approved);
    assert!(!params.always);
    assert_eq!(params.answers["q_lang"].selected, vec!["zh"]);
    assert_eq!(params.answers["q_scope"].selected, vec!["a", "b"]);

    perm.reply_tx
        .send(AskReply::from_grant(
            params.approved,
            params.always,
            params.free_text,
            params.selected,
            params.answers,
        ))
        .ok();

    let reply = ask.await.expect("join");
    assert_eq!(
        reply.outcome,
        permission::AskOutcome::Allow { always: false }
    );
    assert_eq!(reply.answers["q_lang"].selected, vec!["zh".to_string()]);
    assert_eq!(
        reply.answers["q_lang"].free_text.as_deref(),
        Some("prefer zh"),
        "from_grant trims free_text"
    );
    assert_eq!(
        reply.answers["q_scope"].selected,
        vec!["a".to_string(), "b".to_string()]
    );
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

#[test]
fn ask_kind_wire_snake_case() {
    assert_eq!(
        serde_json::to_value(AskKind::AskUser).unwrap(),
        serde_json::json!("ask_user")
    );
    assert_eq!(
        serde_json::to_value(AskKind::Approval).unwrap(),
        serde_json::json!("approval")
    );
    assert_eq!(
        serde_json::to_value(AskKind::Permission).unwrap(),
        serde_json::json!("permission")
    );
}
