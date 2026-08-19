use litecode::authority::responses::{ResponseStreamEvent, ResponseTextDeltaEvent};
use litecode::client_protocol::project;
use litecode::client_protocol::protocol::{
    TurnEndReason, TurnTokenStats, WireEvent, WireTurnPhase, methods,
};
use litecode::runtime::observer::{InternalEvent, TurnPhase};
use litecode::types::{Item, user_text};

fn sample_snapshot() -> litecode::client_protocol::protocol::SessionSnapshot {
    let turn = Some(litecode::client_protocol::protocol::TurnSnapshot {
        turn_id: "t1".into(),
        phase: WireTurnPhase::Streaming,
        step: 1,
        step_max: 3,
        started_at_ms: 1,
        awaiting_permission: false,
    });
    let binding = litecode::client_protocol::protocol::SessionBindingProjection {
        agent_id: "default".into(),
        model_id: Some("m".into()),
        api_model_id: "m-api".into(),
        label: "M".into(),
        context_window: 0,
        thinking_tier: "medium".into(),
        context_mode: "standard".into(),
    };
    project::buffer_snapshot("s1", "/p", &binding, 0, 0, 0, turn, None, None, 0, false)
}

#[test]
fn compact_lifecycle_started_projects_unified_wire() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::CompactionLifecycle {
            trigger: litecode::runtime::observer::CompactionTrigger::Manual,
            stage: litecode::runtime::observer::CompactionStage::Started,
            operation_id: Some("op-1".into()),
            fail_kind: None,
            error: None,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "session/compact_lifecycle"));
    assert_eq!(msg["params"]["operation_id"], "op-1");
    assert_eq!(msg["params"]["trigger"], "manual");
    assert_eq!(msg["params"]["stage"], "started");
    assert_eq!(msg["params"]["snapshot"]["compacting"], true);
}

#[test]
fn compact_lifecycle_auto_started_does_not_lock_session() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::CompactionLifecycle {
            trigger: litecode::runtime::observer::CompactionTrigger::Auto,
            stage: litecode::runtime::observer::CompactionStage::Started,
            operation_id: None,
            fail_kind: None,
            error: None,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "session/compact_lifecycle"));
    assert_eq!(msg["params"]["trigger"], "auto");
    assert_eq!(msg["params"]["snapshot"]["compacting"], false);
}

fn method_is(msg: &serde_json::Value, expected: &str) -> bool {
    msg.get("method").and_then(|m| m.as_str()) == Some(expected)
}

fn sample_stream_event() -> ResponseStreamEvent {
    ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
        sequence_number: 1,
        item_id: "msg_1".into(),
        output_index: 0,
        content_index: 0,
        delta: "hi".into(),
        logprobs: None,
    })
}

#[test]
fn stream_event_projects_to_turn_event() {
    let snap = sample_snapshot();
    let msg = project::project(&InternalEvent::StreamEvent(sample_stream_event()), &snap).unwrap();
    assert!(method_is(&msg, "agent/turn_event"));
    let params = &msg["params"];
    assert_eq!(params["session_id"], "s1");
    assert_eq!(params["turn_id"], "t1");
    let event: WireEvent = serde_json::from_value(params["event"].clone()).unwrap();
    match event {
        WireEvent::StreamEvent { event } => match event {
            ResponseStreamEvent::ResponseOutputTextDelta(e) => assert_eq!(e.delta, "hi"),
            other => panic!("unexpected stream event: {:?}", other),
        },
        other => panic!("unexpected wire event: {:?}", other),
    }
}

#[test]
fn buffer_item_projects_to_buffer_item_notification() {
    let snap = sample_snapshot();
    let item = user_text("hello");
    let msg = project::project(
        &InternalEvent::BufferItem {
            buffer_index: 3,
            item: item.clone(),
            kind: Some("detail".into()),
            child_session_id: None,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, methods::BUFFER_ITEM));
    assert_eq!(msg["params"]["session_id"], "s1");
    assert_eq!(msg["params"]["buffer_index"], 3);
    assert_eq!(msg["params"]["kind"], "detail");
    assert!(msg["params"].get("child_session_id").is_none());
    let got: Item = serde_json::from_value(msg["params"]["item"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        serde_json::to_value(&item).unwrap()
    );
}

#[test]
fn buffer_item_includes_child_session_id_when_set() {
    let snap = sample_snapshot();
    let item = user_text("hello");
    let msg = project::project(
        &InternalEvent::BufferItem {
            buffer_index: 1,
            item,
            kind: None,
            child_session_id: Some("child-1".into()),
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, methods::BUFFER_ITEM));
    assert_eq!(msg["params"]["child_session_id"], "child-1");
}

#[test]
fn buffer_item_compact_checkpoint_kind_projects() {
    let snap = sample_snapshot();
    let item = user_text("rolled-up");
    let msg = project::project(
        &InternalEvent::BufferItem {
            buffer_index: 3,
            item: item.clone(),
            kind: Some("compact_checkpoint".into()),
            child_session_id: None,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, methods::BUFFER_ITEM));
    assert_eq!(msg["params"]["buffer_index"], 3);
    assert_eq!(msg["params"]["kind"], "compact_checkpoint");
    let got: Item = serde_json::from_value(msg["params"]["item"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        serde_json::to_value(&item).unwrap()
    );
}

#[test]
fn subagent_bound_projects_to_agent_subagent_bound() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::SubagentBound {
            call_id: "call_1".into(),
            child_session_id: "child-xyz".into(),
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, methods::AGENT_SUBAGENT_BOUND));
    assert_eq!(msg["params"]["session_id"], "s1");
    assert_eq!(msg["params"]["call_id"], "call_1");
    assert_eq!(msg["params"]["child_session_id"], "child-xyz");
}

#[test]
fn bash_jobs_projects_to_bash_jobs_notification() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::BashJobs {
            snapshot: litecode::terminal::BashJobsSnapshot {
                jobs: vec![litecode::terminal::BashJobWire {
                    id: "bg_a".into(),
                    call_id: "call_a".into(),
                    command_preview: "sleep 8".into(),
                    output_file: ".litecode/bash/bg_a.output".into(),
                    started_at_ms: 42,
                }],
                waits: vec![litecode::terminal::BashWaitWire {
                    call_id: "wait_1".into(),
                    watching_id: Some("bg_a".into()),
                    started_at_ms: 40,
                    deadline_ms: Some(5000),
                }],
            },
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, methods::BASH_JOBS));
    assert_eq!(msg["params"]["session_id"], "s1");
    assert_eq!(msg["params"]["jobs"][0]["id"], "bg_a");
    assert_eq!(msg["params"]["jobs"][0]["call_id"], "call_a");
    assert_eq!(msg["params"]["waits"][0]["call_id"], "wait_1");
    assert!(msg["params"].get("turn_id").is_none());
}

#[test]
fn session_snapshot_includes_bash_when_set() {
    let mut snap = sample_snapshot();
    snap.bash = Some(litecode::terminal::BashJobsSnapshot {
        jobs: vec![litecode::terminal::BashJobWire {
            id: "bg_a".into(),
            call_id: "call_a".into(),
            command_preview: "sleep".into(),
            output_file: ".litecode/bash/bg_a.output".into(),
            started_at_ms: 1,
        }],
        waits: vec![],
    });
    let msg = project::session_snapshot(snap);
    assert!(method_is(&msg, methods::SESSION_SNAPSHOT));
    assert_eq!(msg["params"]["bash"]["jobs"][0]["id"], "bg_a");
}

#[test]
fn session_snapshot_includes_todos_when_set() {
    let mut snap = sample_snapshot();
    snap.todos = vec![litecode::session::task_state::TodoItem {
        id: "t1".into(),
        content: "ship".into(),
        status: litecode::session::task_state::TodoStatus::InProgress,
        priority: None,
    }];
    let msg = project::session_snapshot(snap);
    assert!(method_is(&msg, methods::SESSION_SNAPSHOT));
    assert_eq!(msg["params"]["todos"][0]["id"], "t1");
    assert_eq!(msg["params"]["todos"][0]["content"], "ship");
    assert_eq!(msg["params"]["todos"][0]["status"], "in_progress");
}

#[test]
fn turn_started_projects_to_turn_started_notification() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::TurnStarted {
            turn_id: "t1".into(),
            input: "go".into(),
            step_max: 3,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "agent/turn_started"));
    assert_eq!(msg["params"]["session_id"], "s1");
}

#[test]
fn turn_completed_projects_to_turn_finished() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::TurnCompleted {
            turn_id: "t1".into(),
            final_text: Some("done".into()),
            reason: TurnEndReason::Completed,
            turn_token_stats: TurnTokenStats::default(),
            committed_start: 0,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "agent/turn_finished"));
    assert_eq!(msg["params"]["session_id"], "s1");
}

#[test]
fn permission_ask_projects_to_permission_request_with_session_id() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::PermissionAsk {
            session_id: "s1".into(),
            turn_id: "t1".into(),
            request_id: "r1".into(),
            tool: "bash".into(),
            rule_id: "default".into(),
            summary: "Run bash command".into(),
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "agent/permission_request"));
    assert_eq!(msg["params"]["session_id"], "s1");
}

#[test]
fn phase_changed_projects_to_turn_event() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::PhaseChanged {
            phase: TurnPhase::CallingLlm,
            step: 2,
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "agent/turn_event"));
    let event: WireEvent = serde_json::from_value(msg["params"]["event"].clone()).unwrap();
    match event {
        WireEvent::PhaseChanged { phase, step } => {
            assert_eq!(phase, WireTurnPhase::CallingLlm);
            assert_eq!(step, 2);
        }
        other => panic!("unexpected event: {:?}", other),
    }
}

#[test]
fn workspace_changed_projects_to_workspace_changed() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::WorkspaceChanged {
            paths: vec!["a.txt".into()],
            kind: "modified".into(),
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "workspace/changed"));
}

#[test]
fn snapshot_notice_projects_to_turn_event() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::SnapshotNotice {
            level: "warn".into(),
            message: "track failed".into(),
        },
        &snap,
    )
    .unwrap();
    assert!(method_is(&msg, "agent/turn_event"));
    let params = msg.get("params").expect("params");
    let event = params.get("event").expect("event");
    assert_eq!(
        event.get("type").and_then(|v| v.as_str()),
        Some("snapshot_notice")
    );
    assert_eq!(
        event.get("message").and_then(|v| v.as_str()),
        Some("track failed")
    );
}

#[test]
fn file_revert_updated_projects_session_snapshot() {
    let snap = sample_snapshot();
    let msg =
        project::project(&InternalEvent::FileRevertUpdated { max_k: Some(2) }, &snap).unwrap();
    assert!(method_is(&msg, "session/snapshot"));
}
