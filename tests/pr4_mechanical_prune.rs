//! PR-4: deterministic mechanical pruning before summarization.
//!
//! A prune runs only inside a compaction trigger, lands as replay-safe surface
//! replacements, is remeasured with the unified ContextMeter, and skips the
//! summary when it already relieved pressure.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::SessionDataFixture;
use common::recording_provider::RecordingProvider;
use litecode::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, InputContent, OutputStatus,
    ReasoningItem,
};
use litecode::client_protocol::controller::SessionController;
use litecode::config::global_db;
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard, WorkspaceState};
use litecode::context_pipeline::{
    BudgetPolicy, CompactPolicy, Context, ContextEnvelope, ContextPipeline, MechanicalPruneConfig,
    MechanicalPrunePlan, ProviderPromptBaseline, TOOL_RESULT_PRUNE_HEAD_CHARS,
    TOOL_RESULT_PRUNE_MARKER, TOOL_RESULT_PRUNE_TAIL_CHARS, TOOL_RESULT_PRUNE_THRESHOLD_CHARS,
    measure_request_context, prune_min_saved_tokens,
};
use litecode::engines::WorkspaceEngines;
use litecode::llm::LlmProvider;
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::session::{
    SessionManager, SurfaceOp, derive_messages, derive_transcript_items, fold_surface,
};
use litecode::types::{Item, assistant_text, item_text_preview, user_message, user_text};
use tokio_util::sync::CancellationToken;

fn call(call_id: &str) -> Item {
    Item::FunctionCall(FunctionToolCall {
        call_id: call_id.into(),
        name: "read".into(),
        arguments: "{}".into(),
        id: Some(format!("call-item-{call_id}")),
        status: None,
        namespace: None,
    })
}

fn output(call_id: &str, text: String) -> Item {
    Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: call_id.into(),
        output: FunctionCallOutput::Text(text),
        id: Some(format!("output-{call_id}")),
        status: Some(OutputStatus::Completed),
    })
}

fn output_text(item: &Item) -> &str {
    let Item::FunctionCallOutput(output) = item else {
        panic!("expected function_call_output, got {item:?}");
    };
    let FunctionCallOutput::Text(text) = &output.output else {
        panic!("expected text output, got {:?}", output.output);
    };
    text
}

fn signed_reasoning() -> Item {
    Item::Reasoning(ReasoningItem {
        id: Some("rs_pr4_media".into()),
        summary: vec![],
        content: None,
        encrypted_content: Some("signed-before-prune".into()),
        status: None,
    })
}

fn controller_with_system_prompt(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
    provider: Arc<dyn LlmProvider>,
    system_prompt: &str,
) -> SessionController {
    let db = workspace_root.join("global-litecode.db");
    let mut baseline = common::test_resolved("default", &[]).global().clone();
    baseline
        .agents
        .get_mut("default")
        .expect("default agent")
        .system_prompt = system_prompt.to_string();
    common::insert_test_llm_registry(&mut baseline, "test-key");
    common::seed_test_catalog(&db, "http://127.0.0.1:9", 100_000);
    global_db::import_into(&db, &baseline).expect("seed global db");

    let guard = Arc::new(TurnGuard::new());
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(&db, guard);
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let revision = writer.revision_handle();
    let settings = writer.load_settings().expect("load settings");
    let workspace = WorkspaceState::new(workspace_root);
    let resolved = ConfigManager::resolve(settings, workspace.clone(), common::catalog_for_db(&db));
    let workspace_engines = Arc::new(WorkspaceEngines::new());
    let ide = litecode::ide_base::IdeBaseHandle::open(
        workspace.workspace_root.clone(),
        Arc::clone(&workspace_engines),
    )
    .expect("ide");
    let runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        workspace_engines,
        ide,
        revision,
        &db,
    )
    .with_test_llm_override(provider);
    SessionController::with_turn_guard(runtime, None, sessions).expect("controller")
}

#[tokio::test(flavor = "current_thread")]
async fn prune_relieves_pressure_skips_summary_and_replays_from_source_seq() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    let mut seed = vec![user_text("start")];
    for index in 0..6 {
        let call_id = format!("call-{index}");
        seed.push(call(&call_id));
        // ~12k chars / ~2.4k cl100k tokens each: six results comfortably clear
        // the 4,096-token prune floor and the small-window pressure line.
        seed.push(output(&call_id, "word ".repeat(2_400)));
    }
    sessions.insert_detail_rows(&sid, &seed).expect("seed");

    let before_rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("before working set");
    let provider = Arc::new(RecordingProvider::new(vec![]));
    let mut request =
        common::test_compaction_request(provider.clone(), "main instructions", vec![], &sid);
    common::set_compaction_request_capacity(&mut request, 12_000, 1_024);

    let envelope = ContextEnvelope::from_compaction_request(&request);
    let baseline = ProviderPromptBaseline::default();
    let before_meter = measure_request_context(
        &sessions,
        &sid,
        &envelope,
        &baseline,
        &before_rows,
        request.target.as_ref(),
        &sessions.data_root_path(),
    );
    assert!(
        before_meter.should_compact(),
        "fixture must trigger pressure before pruning: {before_meter:?}"
    );
    let plan = MechanicalPrunePlan::build(&before_rows, MechanicalPruneConfig::default());
    assert!(
        plan.meets_min_savings(request.context_window),
        "fixture must clear the prune savings floor: {plan:?}"
    );

    let budget = BudgetPolicy::new(12_000).with_keep_recent_tokens(1);
    let compacted = CompactPolicy::compact_now(
        &budget,
        &sessions,
        &sid,
        &request,
        1_024,
        &CancellationToken::new(),
        Some("pr4-prune-relief"),
        &sessions.data_root_path(),
    )
    .await
    .expect("manual compaction trigger");
    assert!(
        compacted,
        "a committed prune counts as a successful compaction step"
    );
    assert!(
        provider.captured().is_empty(),
        "prune relieved pressure, so the summarizer must not be called"
    );

    let events = sessions.data().events_blocking(&sid).expect("events");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event_type, litecode::session::EventType::Compacted)),
        "skipping summary must not write a checkpoint"
    );
    let replacement_events: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.surface_op, Some(SurfaceOp::Replace { .. })))
        .collect();
    assert_eq!(
        replacement_events.len(),
        6,
        "one replacement per tool result"
    );
    for event in &replacement_events {
        assert_eq!(event.event_type.as_str(), "item/tool_result");
        let sources = event
            .source_seqs
            .as_ref()
            .expect("replacement cites sources");
        assert_eq!(sources.len(), 1);
        let source_seq = sources[0];
        assert!(source_seq < event.seq);
        let original = events
            .iter()
            .find(|candidate| candidate.seq == source_seq)
            .expect("original append-only row survives");
        let original_item: Item =
            serde_json::from_value(original.data.clone()).expect("original item");
        assert_eq!(
            output_text(&original_item).chars().count(),
            12_000,
            "the original full tool result remains in the log"
        );
        let replacement_item: Item =
            serde_json::from_value(event.data.clone()).expect("replacement item");
        assert_eq!(
            output_text(&replacement_item).chars().count(),
            TOOL_RESULT_PRUNE_HEAD_CHARS
                + TOOL_RESULT_PRUNE_MARKER.chars().count()
                + TOOL_RESULT_PRUNE_TAIL_CHARS
        );
        assert!(output_text(&replacement_item).contains(TOOL_RESULT_PRUNE_MARKER));
    }

    let after_rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("after working set");
    let calls: Vec<_> = after_rows
        .iter()
        .filter_map(|row| row.item())
        .filter_map(|item| match item {
            Item::FunctionCall(call) => Some(call.call_id.clone()),
            _ => None,
        })
        .collect();
    let outputs: Vec<_> = after_rows
        .iter()
        .filter_map(|row| row.item())
        .filter_map(|item| match item {
            Item::FunctionCallOutput(output) => Some(output),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 6, "function calls must still be present");
    assert_eq!(outputs.len(), 6, "function outputs must still be present");
    for output in &outputs {
        assert!(
            calls.contains(&output.call_id),
            "call/output pair stays joined"
        );
        let expected_id = format!("output-{}", output.call_id);
        assert_eq!(output.id.as_deref(), Some(expected_id.as_str()));
        assert_eq!(output.status, Some(OutputStatus::Completed));
    }

    // Surface fold + Item projection must show exactly the durable replacement,
    // not the original full row.
    let surface = fold_surface(&events).expect("surface fold");
    assert_eq!(surface.nodes.len(), after_rows.len());
    let history = derive_transcript_items(&events).expect("append-origin history");
    assert!(
        history.iter().any(|item| matches!(
            item,
            Item::FunctionCallOutput(output)
                if matches!(&output.output, FunctionCallOutput::Text(text) if text.chars().count() == 12_000)
        )),
        "the original full result must remain visible in the append-only history"
    );
    let messages = derive_messages(&events).expect("replayed model messages");
    let replayed_outputs: Vec<_> = messages
        .iter()
        .filter_map(|item| match item {
            Item::FunctionCallOutput(output) => Some(output),
            _ => None,
        })
        .collect();
    assert_eq!(replayed_outputs.len(), 6);
    for output in replayed_outputs {
        let FunctionCallOutput::Text(text) = &output.output else {
            panic!("replayed output must be pruned text");
        };
        assert!(text.contains(TOOL_RESULT_PRUNE_MARKER));
        assert!(text.chars().count() <= TOOL_RESULT_PRUNE_THRESHOLD_CHARS);
    }

    let second = MechanicalPrunePlan::build(&after_rows, MechanicalPruneConfig::default());
    assert!(
        second.is_empty(),
        "a second prune pass over the durable replacement must be a no-op: {second:?}"
    );

    let after_envelope = ContextEnvelope::from_compaction_request(&request);
    let after_meter = measure_request_context(
        &sessions,
        &sid,
        &after_envelope,
        &baseline,
        &after_rows,
        request.target.as_ref(),
        &sessions.data_root_path(),
    );
    assert!(
        !after_meter.should_compact(),
        "pruned surface must remeasure below the pressure line: {after_meter:?}"
    );
    let replacement_preview = after_rows
        .iter()
        .filter_map(|row| row.item())
        .map(item_text_preview)
        .find(|text| text.contains(TOOL_RESULT_PRUNE_MARKER))
        .expect("replacement preview");
    assert!(replacement_preview.starts_with(&"word ".repeat(10)));
}

#[tokio::test(flavor = "current_thread")]
async fn low_savings_prune_writes_no_surface_and_summary_still_runs() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    let mut seed = vec![user_text("start")];
    for index in 0..4 {
        let call_id = format!("low-call-{index}");
        seed.push(call(&call_id));
        // 8,200 chars is over the 8,192 threshold, but four of these results
        // together save far less than the 4,096-token cache-bust floor.
        seed.push(output(&call_id, "word ".repeat(1_640)));
    }
    sessions.insert_detail_rows(&sid, &seed).expect("seed");

    let before_rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("before working set");
    let plan = MechanicalPrunePlan::build(&before_rows, MechanicalPruneConfig::default());
    assert_eq!(plan.replacements.len(), 4);
    assert!(
        plan.estimated_saved_tokens < prune_min_saved_tokens(7_000),
        "fixture must stay below the savings floor: {plan:?}"
    );

    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![vec![common::assistant_text_item(
            "## summary\nlow-yield fixture",
            "msg_low_yield",
        )]],
    ));
    let captured = Arc::clone(&provider);
    let mut request = common::test_compaction_request(provider, "main instructions", vec![], &sid);
    common::set_compaction_request_capacity(&mut request, 7_000, 512);

    let envelope = ContextEnvelope::from_compaction_request(&request);
    let baseline = ProviderPromptBaseline::default();
    let meter = measure_request_context(
        &sessions,
        &sid,
        &envelope,
        &baseline,
        &before_rows,
        request.target.as_ref(),
        &sessions.data_root_path(),
    );
    assert!(
        meter.should_compact(),
        "fixture must trigger pressure so the prune gate is exercised: {meter:?}"
    );

    let budget = BudgetPolicy::new(7_000).with_keep_recent_tokens(1);
    let mut working_rows = before_rows.clone();
    let did = CompactPolicy
        .compact_if_needed(
            &budget,
            &sessions,
            &sid,
            &request,
            512,
            &baseline,
            &mut working_rows,
            1,
            &CancellationToken::new(),
            &sessions.data_root_path(),
        )
        .await
        .expect("auto compaction path");
    assert!(did, "summary must still run after a below-floor prune");
    assert_eq!(captured.captured().len(), 1, "one main-model summary call");

    let events = sessions.data().events_blocking(&sid).expect("events");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.surface_op, Some(SurfaceOp::Replace { .. }))),
        "a below-floor plan must not commit a surface replacement"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event_type, litecode::session::EventType::Compacted)),
        "the normal summary path still commits its checkpoint"
    );
}

#[test]
fn ordinary_view_build_never_prunes_before_a_compaction_trigger() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    sessions
        .insert_detail_rows(
            &sid,
            &[
                user_text("start"),
                call("call-view"),
                output("call-view", "word ".repeat(2_400)),
            ],
        )
        .expect("seed");

    let provider = Arc::new(RecordingProvider::new(vec![]));
    let request = common::test_compaction_request(provider, "main instructions", vec![], &sid);
    let pipeline = ContextPipeline::new(
        128_000,
        Context {
            cwd: std::path::PathBuf::from("/proj"),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(
                std::path::Path::new("/proj"),
            ),
            agents_md: None,
            claude_md: None,
        },
        sessions.data_root_path(),
    );
    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, request.target.as_ref())
        .expect("ordinary view build");
    let prepared = pipeline.take_prepared_view().expect("prepared view");
    let full_output = prepared
        .items
        .iter()
        .find_map(|item| match item {
            Item::FunctionCallOutput(output) => Some(output),
            _ => None,
        })
        .expect("tool output in the ordinary view");
    let FunctionCallOutput::Text(text) = &full_output.output else {
        panic!("expected full text output");
    };
    assert_eq!(
        text.chars().count(),
        12_000,
        "ordinary turns must not temporarily prune the model view"
    );

    let events = sessions.data().events_blocking(&sid).expect("events");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.surface_op, Some(SurfaceOp::Replace { .. }))),
        "view building is read-only; prune belongs to the compaction trigger"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn stale_provider_bill_cannot_hide_prune_savings_after_cold_restart() {
    use litecode::config::TurnGuard;
    use litecode::session::store::ProviderUsage;
    use litecode::session::{SessionData, WorkspaceWriteLease};
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("workspace lease");
    {
        let data = SessionData::open(&lease, &db).expect("open session db");
        let sessions = Arc::new(SessionManager::from_data(Arc::new(TurnGuard::new()), data));
        let sid = sessions
            .data()
            .create_session("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF))
            .expect("create session");

        let mut seed = vec![user_text("start")];
        for index in 0..6 {
            let call_id = format!("bill-call-{index}");
            seed.push(call(&call_id));
            seed.push(output(&call_id, "word ".repeat(2_400)));
        }
        sessions.insert_detail_rows(&sid, &seed).expect("seed");
        let header = sessions
            .append_request_origin(&sid, &serde_json::json!({"schema": 1, "turn": "bill"}))
            .expect("request header");
        sessions
            .record_provider_usage(
                &sid,
                header,
                &ProviderUsage {
                    prompt_tokens: 12_000,
                    completion_tokens: 1,
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 12_000,
                },
            )
            .expect("record persisted bill");

        let provider = Arc::new(RecordingProvider::new(vec![]));
        let mut request =
            common::test_compaction_request(provider, "main instructions", vec![], &sid);
        common::set_compaction_request_capacity(&mut request, 12_000, 1_024);

        let baseline = ProviderPromptBaseline::default();
        let pipeline = ContextPipeline::new(
            12_000,
            Context {
                cwd: std::path::PathBuf::from("/proj"),
                workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(
                    std::path::Path::new("/proj"),
                ),
                agents_md: None,
                claude_md: None,
            },
            sessions.data_root_path(),
        );
        pipeline.begin_turn(&sessions, &sid).expect("begin turn");
        let compacted = pipeline
            .compact_step(
                &sessions,
                &sid,
                &request,
                1_024,
                &baseline,
                1,
                &CancellationToken::new(),
            )
            .await
            .expect("auto compact step");
        assert!(compacted, "prune must count as the compaction-stage result");
        assert!(
            baseline.is_invalidated(),
            "live baseline must be invalidated"
        );

        let meta = sessions.data().meta_blocking(&sid).expect("session meta");
        assert!(
            meta.surface_rewrite_seq > header,
            "ReplaceSurface must durably advance the rewrite watermark: meta={:?} header={header}",
            meta.surface_rewrite_seq
        );
        drop(pipeline);
        drop(sessions);
    }

    // Writer shutdown + new SessionData is the cold-start path: no live bill,
    // no baseline invalidated flag, only the persisted meta/rollup.
    let data = SessionData::open(&lease, &db).expect("reopen session db");
    let sessions = Arc::new(SessionManager::from_data(Arc::new(TurnGuard::new()), data));
    let sid = sessions
        .data()
        .list_sessions_blocking()
        .expect("list sessions")
        .first()
        .expect("persisted session")
        .id
        .clone();
    let meta = sessions.data().meta_blocking(&sid).expect("cold meta");
    let provider = Arc::new(RecordingProvider::new(vec![]));
    let mut request = common::test_compaction_request(provider, "main instructions", vec![], &sid);
    common::set_compaction_request_capacity(&mut request, 12_000, 1_024);
    let pipeline = ContextPipeline::new(
        12_000,
        Context {
            cwd: std::path::PathBuf::from("/proj"),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(
                std::path::Path::new("/proj"),
            ),
            agents_md: None,
            claude_md: None,
        },
        sessions.data_root_path(),
    );
    let fresh_baseline = ProviderPromptBaseline::default();
    pipeline
        .begin_turn(&sessions, &sid)
        .expect("cold begin turn");
    pipeline
        .build_view(&sessions, &sid, &fresh_baseline, request.target.as_ref())
        .expect("cold view after prune");
    let prepared = pipeline.take_prepared_view().expect("prepared view");
    assert_eq!(
        prepared.context_meter.source,
        litecode::context_pipeline::ContextMeterSource::LocalEnvelope,
        "bill latest_seq <= surface_rewrite_seq ({}) must fall back local, not double count",
        meta.surface_rewrite_seq
    );
    assert!(
        !prepared.context_meter.should_compact(),
        "cold view after prune must be below pressure: {:?}",
        prepared.context_meter
    );
    assert!(
        prepared.context_meter.used_tokens < 12_000,
        "stale full-text bill must not survive the rewrite: {:?}",
        prepared.context_meter
    );
}

fn media_count(item: &Item) -> usize {
    match item {
        Item::Message(litecode::authority::responses::MessageItem::Input(message)) => message
            .content
            .iter()
            .filter(|part| {
                matches!(
                    part,
                    InputContent::InputImage(_) | InputContent::InputFile(_)
                )
            })
            .count(),
        Item::FunctionCallOutput(output) => match &output.output {
            FunctionCallOutput::Content(parts) => parts
                .iter()
                .filter(|part| {
                    matches!(
                        part,
                        InputContent::InputImage(_) | InputContent::InputFile(_)
                    )
                })
                .count(),
            FunctionCallOutput::Text(_) => 0,
        },
        _ => 0,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn historical_user_media_is_replaced_by_stable_text_in_the_surface() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    let urls: Vec<String> = (0..6)
        .map(|index| format!("litecode-media:old-{index}.png"))
        .collect();
    let media_user = user_message("look at these", &urls);
    let mut seed = vec![
        media_user,
        signed_reasoning(),
        assistant_text("done"),
        user_text("next"),
    ];
    // Historical media alone is already stripped by the ephemeral show-once
    // projection, so pair it with worthwhile tool-result text to exercise the
    // durable media replacement inside a real prune batch.
    for index in 0..4 {
        let call_id = format!("media-batch-{index}");
        seed.push(call(&call_id));
        seed.push(output(&call_id, "word ".repeat(2_400)));
    }
    sessions
        .insert_detail_rows(&sid, &seed)
        .expect("seed media history");
    let media_seq = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("working set")
        .first()
        .and_then(|row| row.log_seq)
        .expect("media user seq");

    let provider = Arc::new(RecordingProvider::new(vec![]));
    let mut request = common::test_compaction_request(provider, "main instructions", vec![], &sid);
    common::set_compaction_request_capacity(&mut request, 12_000, 1_024);

    let pipeline = ContextPipeline::new(
        12_000,
        Context {
            cwd: std::path::PathBuf::from("/proj"),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(
                std::path::Path::new("/proj"),
            ),
            agents_md: None,
            claude_md: None,
        },
        sessions.data_root_path(),
    );
    pipeline.begin_turn(&sessions, &sid).expect("begin turn");
    let baseline = ProviderPromptBaseline::default();
    let compacted = pipeline
        .compact_step(
            &sessions,
            &sid,
            &request,
            1_024,
            &baseline,
            1,
            &CancellationToken::new(),
        )
        .await
        .expect("auto prune trigger");
    assert!(compacted, "worthwhile media prune must run on the trigger");

    let events = sessions.data().events_blocking(&sid).expect("events");
    let replacement = events
        .iter()
        .find(|event| {
            matches!(event.event_type, litecode::session::EventType::ItemUser)
                && matches!(event.surface_op, Some(SurfaceOp::Replace { .. }))
        })
        .expect("media replacement event");
    assert_eq!(
        replacement.source_seqs.as_deref(),
        Some([media_seq].as_slice())
    );
    let replacement_item: Item =
        serde_json::from_value(replacement.data.clone()).expect("replacement item");
    assert_eq!(media_count(&replacement_item), 0);
    assert!(
        item_text_preview(&replacement_item)
            .contains("[image already viewed; removed from context]")
    );
    let original = events
        .iter()
        .find(|event| event.seq == media_seq)
        .expect("original media row");
    let original_item: Item = serde_json::from_value(original.data.clone()).expect("original item");
    assert_eq!(
        media_count(&original_item),
        6,
        "original event is untouched"
    );

    let after_rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("after media prune");
    let replacement_row = after_rows
        .iter()
        .find(|row| row.log_seq == Some(replacement.seq))
        .expect("durable media replacement row");
    assert_eq!(media_count(replacement_row.item().unwrap()), 0);
    assert!(
        replacement_row.replacement_media,
        "media replacement provenance must survive the cold reader"
    );
    pipeline
        .build_view(&sessions, &sid, &baseline, request.target.as_ref())
        .expect("view after media prune");
    let prepared = pipeline.take_prepared_view().expect("prepared view");
    let reasoning = prepared
        .items
        .iter()
        .position(|item| matches!(item, Item::Reasoning(_)))
        .expect("signed reasoning in prepared view");
    assert!(
        prepared.rebased[reasoning],
        "media replacement must rebuild the media-rebase signature: {:?}",
        prepared.rebased
    );
    assert!(
        MechanicalPrunePlan::build(&after_rows, MechanicalPruneConfig::default()).is_empty(),
        "media placeholders must not be rewritten again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_controller_admits_prune_without_keep_recent_cut() {
    use litecode::runtime::observer::{CompactionStage, CompactionTrigger, InternalEvent};

    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    let mut seed = vec![user_text("start")];
    for index in 0..4 {
        let call_id = format!("controller-call-{index}");
        seed.push(call(&call_id));
        seed.push(output(&call_id, "word ".repeat(2_400)));
    }
    sessions.insert_detail_rows(&sid, &seed).expect("seed");

    let rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("working set");
    let transcript = litecode::session::project_items(&rows);
    let budget = BudgetPolicy::new(100_000);
    assert!(
        !CompactPolicy::can_compact(&budget, &transcript),
        "fixture must have no keep-recent summary cut"
    );
    let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default());
    assert!(
        plan.meets_min_savings(100_000),
        "fixture prune must clear the manual fallback floor: {plan:?}"
    );

    let provider = Arc::new(RecordingProvider::new(vec![]));
    let captured = Arc::clone(&provider);
    let mut ctrl = controller_with_system_prompt(
        Arc::clone(&sessions),
        fixture.dir.path(),
        provider,
        &"word ".repeat(30_000),
    );
    ctrl.subscribe_checked(&sid).await.expect("subscribe");
    let snapshot = ctrl.snapshot_for(&sid).expect("snapshot");
    assert!(
        snapshot.context_meter.manual_eligible(),
        "large system prompt must make the manual 30% gate eligible: {:?}",
        snapshot.context_meter
    );
    assert_eq!(
        snapshot.compact_eligible,
        snapshot.context_meter.manual_eligible(),
        "manual eligibility must stay the shared product gate"
    );

    let mut rx = sessions
        .subscribe(&sid)
        .expect("subscribe compaction lifecycle");
    let operation_id = ctrl
        .start_manual_compact(&sid)
        .await
        .expect("controller must admit prune-only manual compaction");

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut started_telemetry = None;
    let mut succeeded_telemetry = None;
    while std::time::Instant::now() < deadline && succeeded_telemetry.is_none() {
        if let Ok(envelope) = rx.try_recv()
            && let InternalEvent::CompactionLifecycle {
                trigger: CompactionTrigger::Manual,
                stage,
                operation_id: event_operation,
                telemetry,
                ..
            } = envelope.event
        {
            if event_operation.as_deref() != Some(operation_id.as_str()) {
                continue;
            }
            match stage {
                CompactionStage::Succeeded => succeeded_telemetry = Some(telemetry),
                CompactionStage::Failed => panic!("manual prune-only compaction failed"),
                CompactionStage::Started => started_telemetry = Some(telemetry),
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let started = started_telemetry.expect("manual prune-only Started lifecycle");
    let succeeded = succeeded_telemetry.expect("manual prune-only Succeeded lifecycle");
    for (stage, telemetry) in [("Started", started), ("Succeeded", succeeded)] {
        assert!(
            telemetry.tokens_before.is_some()
                && telemetry.tokens_pruned.is_some()
                && telemetry.tokens_after.is_some(),
            "prune-only {stage} must carry measured token fields: {telemetry:?}"
        );
        assert_eq!(
            telemetry.summary_attempts,
            Some(0),
            "prune-only {stage} must report zero summary attempts"
        );
        assert_eq!(
            telemetry.overflow_retry,
            Some(0),
            "prune-only {stage} must report zero overflow retries"
        );
    }

    let events = sessions.data().events_blocking(&sid).expect("events");
    let replacements = events
        .iter()
        .filter(|event| matches!(event.surface_op, Some(SurfaceOp::Replace { .. })))
        .count();
    assert_eq!(replacements, 4, "manual path must commit the prune batch");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event_type, litecode::session::EventType::Compacted)),
        "prune-only manual path must skip summary checkpoint"
    );
    assert!(
        captured.captured().is_empty(),
        "prune-only manual path must not call the summarizer"
    );
}
