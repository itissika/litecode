use litecode::client_protocol::project;
use litecode::client_protocol::protocol::{TurnEndReason, TurnTokenStats, WireTurnPhase};
use litecode::runtime::observer::{InternalEvent, TurnErrorKind};

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
        max_output_tokens: 0,
        thinking_tier: "medium".into(),
        context_mode: "standard".into(),
    };
    project::buffer_snapshot(
        "s1",
        "/p",
        &binding,
        -1,
        0,
        0,
        turn,
        None,
        None,
        &litecode::context_pipeline::ContextMeter::default(),
        &litecode::context_pipeline::ContextEnvelope::default(),
        Default::default(),
    )
}

#[test]
fn auto_compaction_is_visible_in_snapshot_with_one_revisioned_state() {
    use litecode::client_protocol::protocol::SessionBindingProjection;
    use litecode::runtime::observer::CompactionTrigger;

    let manager = litecode::session::SessionManager::new_for_test(
        std::sync::Arc::new(litecode::config::TurnGuard::default()),
        String::new(),
    );
    let sid = manager.open_session_sync("", "default", None).unwrap();
    let binding = SessionBindingProjection {
        agent_id: "default".into(),
        model_id: Some("m".into()),
        api_model_id: "m-api".into(),
        label: "M".into(),
        context_window: 0,
        max_output_tokens: 0,
        thinking_tier: "medium".into(),
        context_mode: "standard".into(),
    };

    let (revision, id) =
        manager.begin_compaction_blocking(&sid, None, CompactionTrigger::Auto, Some(41));
    let snapshot = project::buffer_snapshot(
        &sid,
        "/p",
        &binding,
        -1,
        0,
        0,
        None,
        None,
        None,
        &litecode::context_pipeline::ContextMeter::default(),
        &litecode::context_pipeline::ContextEnvelope::default(),
        project::compaction_state_wire(&manager.compaction_state_blocking(&sid)),
    );
    assert!(snapshot.compacting, "legacy mirror must stay in sync");
    assert!(snapshot.compaction.active);
    assert_eq!(snapshot.compaction.revision, revision);
    assert_eq!(
        snapshot.compaction.operation_id.as_deref(),
        Some(id.as_str())
    );
    assert_eq!(snapshot.compaction.trigger, Some(CompactionTrigger::Auto));
    assert_eq!(snapshot.compaction.at, Some(41));

    manager.end_compaction_blocking(&sid, None, CompactionTrigger::Auto);
    let cleared = project::compaction_state_wire(&manager.compaction_state_blocking(&sid));
    assert!(!cleared.active);
    assert!(cleared.revision > revision);
}

#[test]
fn turn_finished_marks_a_failed_reconnect_retryable() {
    use litecode::llm::reconnect::{LlmReconnect, LlmReconnectPhase};

    let mut snap = sample_snapshot();
    snap.llm_reconnect = Some(LlmReconnect {
        phase: LlmReconnectPhase::Failed,
        attempt: 6,
        max_attempts: 6,
        delay_ms: 0,
    });
    let msg = project::project(
        &InternalEvent::TurnCompleted {
            turn_id: "t1".into(),
            final_text: Some("opening stream failed".into()),
            reason: TurnEndReason::Error,
            turn_token_stats: TurnTokenStats::default(),
            committed_next_seq: 0,
            error_kind: None,
            cancelled_by: None,
        },
        &snap,
    )
    .unwrap();
    assert_eq!(msg["params"]["error"]["code"], "llm_http");
    assert_eq!(msg["params"]["error"]["retryable"], true);
    assert_eq!(
        msg["params"]["snapshot"]["llm_reconnect"]["phase"],
        "failed"
    );
    assert!(
        msg["params"]["snapshot"]["llm_reconnect"]
            .get("delay_ms")
            .is_none()
    );

    snap.llm_reconnect = None;
    let plain = project::project(
        &InternalEvent::TurnCompleted {
            turn_id: "t1".into(),
            final_text: Some("boom".into()),
            reason: TurnEndReason::Error,
            turn_token_stats: TurnTokenStats::default(),
            committed_next_seq: 0,
            error_kind: None,
            cancelled_by: None,
        },
        &snap,
    )
    .unwrap();
    assert_eq!(plain["params"]["error"]["code"], "internal");
    assert_eq!(plain["params"]["error"]["retryable"], false);
}

/// WP-024 G3 (protocol half) / WP-025 / WP-069: auto compact failure ends the
/// turn with typed error_kind=CompactionFailed; client sees compaction_failed,
/// not retryable (no final_text string match).
#[test]
fn turn_finished_auto_compact_failure_is_compaction_failed() {
    let snap = sample_snapshot();
    let msg = project::project(
        &InternalEvent::TurnCompleted {
            turn_id: "t1".into(),
            final_text: Some("compaction blew up".into()),
            reason: TurnEndReason::Error,
            turn_token_stats: TurnTokenStats::default(),
            committed_next_seq: 0,
            error_kind: Some(TurnErrorKind::CompactionFailed),
            cancelled_by: None,
        },
        &snap,
    )
    .unwrap();
    assert_eq!(msg["params"]["error"]["code"], "compaction_failed");
    assert_eq!(msg["params"]["error"]["retryable"], false);
    assert_eq!(msg["params"]["error"]["message"], "compaction blew up");
}
