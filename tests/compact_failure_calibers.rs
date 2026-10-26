//! WP-014 / ISSUE-027: Andy's three compact-failure calibers.
//!
//! 1. auto compact Err → turn stops (see also `agent_loop::auto_compact_fail_stops_turn_without_model_call`)
//! 2. compact failure restores transcript snapshot
//! 3. manual compact failure releases Compact lease so the user can retry

mod common;

use std::sync::Arc;

use litecode::config::workspace::set_runtime_paths;
use litecode::config::{TurnGuard, WorkspaceState};
use litecode::context_pipeline::{
    BudgetPolicy, CompactPolicy, ContextPipeline, ProviderPromptBaseline,
};
use litecode::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, InternalEvent,
};
use litecode::session::manager::{SessionManager, SessionOperationKind};
use litecode::session::{WorkingRow, project_items};
use litecode::types::{LitecodeError, item_text_preview, user_text};
use tokio_util::sync::CancellationToken;

use common::scripted_provider::ScriptedProvider;

fn light_workspace(dir: &std::path::Path) -> WorkspaceState {
    let litecode_dir = dir.join(".litecode");
    std::fs::create_dir_all(litecode_dir.join("logs")).expect("logs dir");
    std::fs::create_dir_all(litecode_dir.join("plan")).expect("plan dir");
    let sessions_db = litecode_dir.join("sessions.db");
    if !sessions_db.exists() {
        std::fs::File::create(&sessions_db).expect("sessions.db");
    }
    let workspace_id = "test-workspace-id".to_string();
    WorkspaceState {
        workspace_root: dir.to_path_buf(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        agents_md: String::new(),
        paths: litecode::config::WorkspacePaths::for_workspace(dir, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
        workspace_custom_suites: Default::default(),
    }
}

fn setup_workspace_and_session(
    dir: &std::path::Path,
    agent: &str,
) -> (String, Arc<SessionManager>) {
    let ws = light_workspace(dir);
    set_runtime_paths(ws.paths.clone());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path,
    ));
    let sid = sessions
        .open_session_sync("/proj", agent, Some(common::TEST_PRIMARY_MODEL_REF))
        .expect("open");
    (sid, sessions)
}

fn row_previews(rows: &[WorkingRow]) -> Vec<String> {
    rows.iter()
        .map(|r| item_text_preview(&r.payload.model_item()))
        .collect()
}

/// Caliber 2: empty-summary compact failure restores the in-memory working-set
/// snapshot and leaves durable transcript / Compacted events untouched.
#[tokio::test(flavor = "current_thread")]
async fn compact_failure_restores_transcript_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_workspace_and_session(dir.path(), "default");

    let seed: Vec<_> = (0..12)
        .map(|i| user_text(format!("restore-seed-{i}")))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let before = sessions.data().working_set_blocking(&sid).unwrap();
    let before_previews = row_previews(&before);
    let before_len = before.len();
    assert!(before_len >= 12, "fixture must seed a real prefix");

    // Tiny keep window so keep-recent finds a cut; empty summary → CompactionFailed.
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    assert!(
        CompactPolicy::can_compact(&budget, &project_items(&before)),
        "fixture must be compactable so we hit the LLM/fail path, not NothingToCompact"
    );

    let provider = ScriptedProvider::with_texts(&["", ""]);
    let err = CompactPolicy::compact_now(
        &budget,
        &sessions,
        &sid,
        &common::test_compaction_request(Arc::new(provider), "system", vec![], &sid),
        1024,
        &CancellationToken::new(),
        Some("op-restore"),
        &sessions.data_root_path(),
    )
    .await
    .expect_err("empty summary must fail compact");
    assert!(
        err.is_compaction_failure(),
        "expected CompactionFailure, got {err:?}"
    );

    let after = sessions.data().working_set_blocking(&sid).unwrap();
    assert_eq!(
        row_previews(&after),
        before_previews,
        "failure must restore the transcript snapshot (working set unchanged)"
    );
    assert_eq!(after.len(), before_len);

    let events = sessions.data().events_blocking(&sid).unwrap();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.event_type, litecode::session::EventType::Compacted)),
        "failed compact must not persist a Compacted event"
    );
}

/// Caliber 2 (auto path): `compact_step` Err leaves the staged working set intact.
#[tokio::test(flavor = "current_thread")]
async fn auto_compact_failure_restores_pipeline_working_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_workspace_and_session(dir.path(), "default");

    let seed: Vec<_> = (0..30)
        .map(|i| user_text(format!("auto-restore-{i}")))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let workspace_id = litecode::config::peek_workspace_id(dir.path())
        .unwrap_or_else(|| "test-workspace-id".into());
    let paths = litecode::config::WorkspacePaths::for_workspace(dir.path(), &workspace_id);
    let ctx = litecode::context_pipeline::Context {
        cwd: dir.path().to_path_buf(),
        workspace_paths: paths,
        agents_md: None,
        claude_md: None,
    };
    let pipeline =
        ContextPipeline::new(10_000, ctx, dir.path().to_path_buf()).with_keep_recent_tokens(1);

    let mut turn = pipeline.begin_turn(&sessions, &sid).unwrap();
    let before = row_previews(&turn);
    pipeline.stage_working(turn.clone());

    let provider = ScriptedProvider::with_texts(&["", ""]);
    let prompt_baseline = ProviderPromptBaseline::default();
    // Force auto pressure on the same 10_000-token request window.
    prompt_baseline.record(8_500, u64::MAX);
    let mut request = common::test_compaction_request(Arc::new(provider), "system", vec![], &sid);
    let max_output_tokens = request.max_output_tokens;
    common::set_compaction_request_capacity(
        &mut request,
        pipeline.context_window(),
        max_output_tokens,
    );

    let err = pipeline
        .compact_step(
            &sessions,
            &sid,
            &request,
            1024,
            &prompt_baseline,
            1,
            &CancellationToken::new(),
        )
        .await
        .expect_err("auto compact empty summary must Err");
    assert!(
        err.is_compaction_failure(),
        "expected CompactionFailure, got {err:?}"
    );

    // compact_step returns Err before restaging — staged set must still be the snapshot.
    let staged = pipeline.working_set();
    assert_eq!(
        row_previews(&staged),
        before,
        "auto compact failure must not replace the staged working set"
    );

    turn = pipeline.begin_turn(&sessions, &sid).unwrap();
    assert_eq!(
        row_previews(&turn),
        before,
        "durable fold after failed auto compact must match the pre-compact snapshot"
    );
}

/// Caliber 3: mirrors `start_manual_compact` — lease → compact_now → drop(lease).
/// After Failed, a new Compact lease must be acquirable (user retry, not agent/retry).
#[tokio::test(flavor = "current_thread")]
async fn manual_compact_failure_releases_lease_for_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_workspace_and_session(dir.path(), "default");

    let seed: Vec<_> = (0..12)
        .map(|i| user_text(format!("manual-retry-{i}")))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let mut rx = sessions
        .subscribe(&sid)
        .expect("subscribe for Failed lifecycle");

    let lease = sessions
        .try_begin_operation(&sid, SessionOperationKind::Compact)
        .expect("first Compact lease");
    let op_id = lease.operation_id().to_string();
    assert!(sessions.is_compacting_blocking(&sid));
    assert!(sessions.is_session_busy_blocking(&sid));

    // Second acquire must fail while the lease is held (same gate start_manual_compact uses).
    assert!(
        matches!(
            sessions.try_begin_operation(&sid, SessionOperationKind::Compact),
            Err(LitecodeError::AgentAlreadyRunning)
        ),
        "overlapping Compact lease must be refused"
    );

    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    let provider = ScriptedProvider::with_texts(&["", ""]);
    let err = CompactPolicy::compact_now(
        &budget,
        &sessions,
        &sid,
        &common::test_compaction_request(Arc::new(provider), "system", vec![], &sid),
        1024,
        &CancellationToken::new(),
        Some(&op_id),
        &sessions.data_root_path(),
    )
    .await
    .expect_err("manual compact empty summary must fail");
    assert!(err.is_compaction_failure());

    // Drain lifecycle: Started then Failed (same events start_manual_compact's spawn emits).
    let mut saw_failed = false;
    while let Ok(env) = rx.try_recv() {
        if let InternalEvent::CompactionLifecycle {
            trigger: CompactionTrigger::Manual,
            stage: CompactionStage::Failed,
            fail_kind,
            operation_id,
            ..
        } = env.event
        {
            assert_eq!(fail_kind, Some(CompactionFailKind::SummaryFinish));
            assert_eq!(operation_id.as_deref(), Some(op_id.as_str()));
            saw_failed = true;
        }
    }
    assert!(
        saw_failed,
        "manual compact failure must emit CompactionStage::Failed"
    );

    // session_lifecycle::start_manual_compact drops the lease after compact_now returns.
    drop(lease);
    assert!(!sessions.is_compacting_blocking(&sid));
    assert!(!sessions.is_session_busy_blocking(&sid));

    let retry = sessions
        .try_begin_operation(&sid, SessionOperationKind::Compact)
        .expect("after Failed + lease drop, user must be able to start_manual_compact again");
    assert!(sessions.is_compacting_blocking(&sid));
    drop(retry);
}

// ---- WP-025 R4 (Runtime turn-stop side) ----
//
// Reconnect is absorbed inside `call_model` (provider stream retry). At the
// agent-loop seam the observable post-reconnect state is: step 1's
// `call_model` returned items (a tool call) after the reconnect. These tests
// pin what happens to the *next* step.

use common::fake_deps::{FakeAgentDeps, function_call_item};

/// After a reconnected step 1 (tool call executed), step 2's auto compact
/// fails → TurnOutcome::Error(CompactionFailed) and no further `call_model`.
#[tokio::test(flavor = "current_thread")]
async fn reconnect_then_auto_compact_failure_stops_turn() {
    let mut deps = FakeAgentDeps::with_responses(vec![
        vec![function_call_item("call_r1", "read", "{}", "fc_r1")],
        vec![common::fake_deps::assistant_text_item(
            "must-not-run",
            "msg_r2",
        )],
    ]);
    deps.compact_fail_from_step = Some(2);

    let outcome = litecode::agent::run(&mut deps).await;
    match outcome {
        litecode::agent::TurnOutcome::Error(err) => assert!(
            matches!(err, LitecodeError::CompactionFailed),
            "expected CompactionFailed, got {err:?}"
        ),
        other => panic!("expected Error(CompactionFailed), got {other:?}"),
    }
    assert_eq!(
        deps.model_calls(),
        1,
        "no call_model after auto compact fails"
    );
    assert_eq!(
        deps.compact_calls.get(),
        2,
        "compact ran at step 1 (ok) and step 2 (fail)"
    );
    assert_eq!(
        deps.execute_calls.get(),
        1,
        "step-1 tool call executed once"
    );
    let landed = deps.landed.borrow();
    assert!(
        !landed
            .iter()
            .any(|i| item_text_preview(i) == "must-not-run"),
        "step-2 model output must never land: {landed:?}"
    );
}
