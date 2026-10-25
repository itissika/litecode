//! PR-3: auto trigger, manual eligibility, and the UI snapshot consume one
//! ContextMeter over the same `project_model_view` request projection.

mod common;

use std::sync::Arc;

use common::SessionDataFixture;
use common::scripted_provider::ScriptedProvider;
use litecode::client_protocol::controller::Projection;
use litecode::client_protocol::protocol::SessionBindingProjection;
use litecode::context_pipeline::{
    Context, ContextEnvelope, ContextMeterSource, ContextPipeline, ProviderPromptBaseline,
    measure_persisted_context, measure_request_context, request_token_breakdown_for_rows,
};
use litecode::platform_knobs::ContextMode;
use litecode::provider_catalog::ResolvedModel;
use litecode::runtime::observer::{InternalEvent, TurnEndReason, TurnTokenStats};
use litecode::types::{Item, user_message, user_text};
use tokio_util::sync::CancellationToken;

fn test_context(dir: &std::path::Path) -> Context {
    Context {
        cwd: dir.to_path_buf(),
        workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(dir),
        agents_md: None,
        claude_md: None,
    }
}

fn test_model() -> Arc<ResolvedModel> {
    common::default_test_catalog()
        .model(common::TEST_PRIMARY_MODEL_REF)
        .expect("fixture catalog primary model")
        .clone()
}

fn model_with(window: usize, output: u32) -> Arc<ResolvedModel> {
    let mut model = (*test_model()).clone();
    model.context_window = window;
    model.context_window_max = window;
    model.max_output = output;
    Arc::new(model)
}

fn binding(context_window: usize, max_output_tokens: u32) -> SessionBindingProjection {
    SessionBindingProjection {
        agent_id: "default".into(),
        model_id: Some(common::TEST_PRIMARY_MODEL_REF.into()),
        api_model_id: "test-primary-model".into(),
        label: "Test".into(),
        context_window,
        max_output_tokens,
        thinking_tier: "medium".into(),
        context_mode: "standard".into(),
    }
}

fn provider_request(
    sid: &str,
    context_window: usize,
    max_output_tokens: u32,
) -> litecode::llm::CompactionRequestContext {
    let provider = Arc::new(
        ScriptedProvider::with_texts_legacy_accepting_compaction_summary(&["compact summary"]),
    );
    let mut request = common::test_compaction_request(provider, "system instructions", vec![], sid);
    common::set_compaction_request_capacity(&mut request, context_window, max_output_tokens);
    request
}

#[tokio::test(flavor = "current_thread")]
async fn auto_manual_and_ui_share_provider_plus_suffix_meter() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let model = model_with(10_000, 1_024);
    let data_root = fixture.dir.path();

    let seed: Vec<_> = (0..30)
        .map(|i| user_text(format!("covered-{i} {}", "word ".repeat(240))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let header = sessions
        .append_request_origin(
            &sid,
            &serde_json::json!({"schema": 1, "turn": "t", "step": 1}),
        )
        .unwrap();
    let usage = litecode::session::store::ProviderUsage {
        prompt_tokens: 8_000,
        completion_tokens: 10,
        cache_hit_tokens: 0,
        cache_miss_tokens: 8_000,
    };
    sessions
        .record_provider_usage(&sid, header, &usage)
        .unwrap();
    let suffix_item = user_text("uncovered suffix after the provider bill");
    let suffix_tokens = litecode::session::estimate::compute_token_estimate(&[suffix_item.clone()]);
    sessions.insert_detail_rows(&sid, &[suffix_item]).unwrap();

    let request = provider_request(&sid, 10_000, 1_024);
    let envelope = ContextEnvelope::from_compaction_request(&request);
    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    let baseline = ProviderPromptBaseline::default();
    baseline.record(8_000, header);

    let auto_meter = measure_request_context(
        &sessions, &sid, &envelope, &baseline, &rows, &model, data_root,
    );
    let manual_meter =
        measure_persisted_context(&sessions, &sid, &envelope, &model, data_root).unwrap();

    let pipeline = ContextPipeline::new(10_000, test_context(data_root), data_root.to_path_buf())
        .with_keep_recent_tokens(1);
    pipeline.freeze_request_envelope(envelope.clone());
    pipeline.stage_working(rows.clone());
    pipeline
        .build_view(&sessions, &sid, &baseline, &model)
        .expect("build actual request view");
    let prepared = pipeline.prepared_view().expect("prepared view");

    let mut proj = Projection::new_with_model(
        sid.clone(),
        Arc::clone(&sessions),
        10_000,
        Some(model.clone()),
    );
    proj.set_context_envelope(envelope.clone());
    let snap = proj.snapshot("/proj", &binding(10_000, 1_024));

    let breakdown = request_token_breakdown_for_rows(&rows, &envelope, &model, data_root);
    let classified = breakdown.system
        + breakdown.tool_schema
        + breakdown.prompt_overhead
        + breakdown.tool_call
        + breakdown.tool_output
        + breakdown.conversation;
    let local_accounted = envelope.fixed_prompt_tokens()
        + litecode::session::estimate::compute_token_estimate(&prepared.items);
    assert_eq!(classified, local_accounted);
    assert!(
        classified < 8_000 + suffix_tokens,
        "provider bill is authoritative; breakdown remains the local item mix"
    );

    let expected = 8_000 + suffix_tokens;
    for (label, meter) in [
        ("auto", &auto_meter),
        ("manual", &manual_meter),
        ("prepared", &prepared.context_meter),
        ("ui", &snap.context_meter),
    ] {
        assert_eq!(meter.used_tokens, expected, "{label} used_tokens");
        assert_eq!(
            meter.source,
            ContextMeterSource::ProviderPlusSuffix,
            "{label}"
        );
        assert_eq!(meter.prompt_limit, auto_meter.prompt_limit, "{label}");
        assert_eq!(meter.compact_at, auto_meter.compact_at, "{label}");
    }
    assert!(auto_meter.should_compact());
    assert!(auto_meter.manual_eligible());
    assert_eq!(snap.context_tokens_estimate, expected);
    assert_eq!(snap.compact_eligible, auto_meter.manual_eligible());
    assert_eq!(snap.context_envelope.output_reserve, 1_024);

    // The auto trigger consumes the same projection/meter.
    pipeline.stage_working(rows);
    let compacted = pipeline
        .compact_step(
            &sessions,
            &sid,
            &request,
            litecode::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &baseline,
            1,
            &CancellationToken::new(),
        )
        .await
        .expect("auto compaction");
    assert!(compacted, "auto must trigger from the shared ContextMeter");
}

#[tokio::test(flavor = "current_thread")]
async fn covered_pad_is_not_charged_again_as_provider_suffix() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let model = model_with(8_000, 1_024);
    let data_root = fixture.dir.path();

    let mut seed: Vec<_> = (0..30)
        .map(|i| user_text(format!("pad-covered-{i} {}", "word ".repeat(240))))
        .collect();
    seed.push(common::function_call_item(
        "call-pad",
        "read",
        r#"{"path":"x"}"#,
        "fc-pad",
    ));
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let header = sessions
        .append_request_origin(
            &sid,
            &serde_json::json!({"schema": 1, "turn": "t", "step": 1}),
        )
        .unwrap();
    let usage = litecode::session::store::ProviderUsage {
        prompt_tokens: 7_000,
        completion_tokens: 10,
        cache_hit_tokens: 0,
        cache_miss_tokens: 7_000,
    };
    sessions
        .record_provider_usage(&sid, header, &usage)
        .unwrap();
    let suffix_item = user_text("uncovered suffix after the covered unanswered call");
    let suffix_tokens = litecode::session::estimate::compute_token_estimate(&[suffix_item.clone()]);
    sessions.insert_detail_rows(&sid, &[suffix_item]).unwrap();

    let request = provider_request(&sid, 8_000, 1_024);
    let envelope = ContextEnvelope::from_compaction_request(&request);
    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    let baseline = ProviderPromptBaseline::default();
    baseline.record(7_000, header);

    let pipeline = ContextPipeline::new(8_000, test_context(data_root), data_root.to_path_buf());
    pipeline.freeze_request_envelope(envelope.clone());
    pipeline.stage_working(rows.clone());
    pipeline
        .build_view(&sessions, &sid, &baseline, &model)
        .expect("build actual request view");
    let prepared = pipeline.prepared_view().expect("prepared view");
    assert!(
        prepared.items.len() > rows.len(),
        "fixture must add a synthetic unanswered-call pad"
    );
    assert!(
        prepared.item_seqs.iter().any(Option::is_none),
        "pad stays item_seqs=None for prefix identity"
    );
    assert!(
        prepared.coverage_seqs.iter().all(Option::is_some),
        "pad must inherit the covered call's billing seq"
    );
    let pad_tokens = prepared
        .items
        .iter()
        .find_map(|item| match item {
            Item::FunctionCallOutput(output) if output.call_id == "call-pad" => {
                Some(litecode::session::estimate::compute_token_estimate(&[
                    item.clone()
                ]))
            }
            _ => None,
        })
        .expect("synthetic pad");
    assert!(pad_tokens > 0, "the pad is a real token-bearing item");

    let auto_meter = measure_request_context(
        &sessions, &sid, &envelope, &baseline, &rows, &model, data_root,
    );
    let manual_meter =
        measure_persisted_context(&sessions, &sid, &envelope, &model, data_root).unwrap();
    let mut proj = Projection::new_with_model(
        sid.clone(),
        Arc::clone(&sessions),
        8_000,
        Some(model.clone()),
    );
    proj.set_context_envelope(envelope.clone());
    let snap = proj.snapshot("/proj", &binding(8_000, 1_024));

    let expected = 7_000 + suffix_tokens;
    for (label, meter) in [
        ("auto", &auto_meter),
        ("manual", &manual_meter),
        ("prepared", &prepared.context_meter),
        ("ui", &snap.context_meter),
    ] {
        assert_eq!(
            meter.used_tokens, expected,
            "{label} must not add pad again"
        );
        assert_eq!(
            meter.source,
            ContextMeterSource::ProviderPlusSuffix,
            "{label}"
        );
        assert_eq!(meter.compact_at, auto_meter.compact_at, "{label}");
    }
    assert_eq!(auto_meter.used_tokens, 7_000 + suffix_tokens);
    assert_ne!(
        auto_meter.used_tokens,
        7_000 + suffix_tokens + pad_tokens,
        "covered pad must not be counted as local suffix"
    );
    assert!(auto_meter.should_compact());
    assert!(auto_meter.manual_eligible());

    pipeline.stage_working(rows);
    let compacted = pipeline
        .compact_step(
            &sessions,
            &sid,
            &request,
            litecode::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &baseline,
            1,
            &CancellationToken::new(),
        )
        .await
        .expect("auto compaction");
    assert!(compacted, "auto must trigger from the padded request meter");
}

#[tokio::test(flavor = "current_thread")]
async fn local_meter_uses_the_padded_and_capability_projected_request() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let model = model_with(10_000, 1_024);
    let data_root = fixture.dir.path();

    let seed = vec![
        user_message("look", &["https://example.test/a.png".to_string()]),
        common::function_call_item("call-pad", "read", r#"{"path":"x"}"#, "fc-pad"),
    ];
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let request = provider_request(&sid, 10_000, 1_024);
    let envelope = ContextEnvelope::from_compaction_request(&request);
    let baseline = ProviderPromptBaseline::default();
    let rows = sessions.data().working_set_blocking(&sid).unwrap();

    let pipeline = ContextPipeline::new(10_000, test_context(data_root), data_root.to_path_buf());
    pipeline.freeze_request_envelope(envelope.clone());
    pipeline.stage_working(rows.clone());
    pipeline
        .build_view(&sessions, &sid, &baseline, &model)
        .expect("build actual request view");
    let prepared = pipeline.prepared_view().expect("prepared view");
    assert!(prepared.projected_parts > 0, "image capability projection");
    assert!(prepared.items.len() > rows.len(), "synthetic pad projected");
    assert!(prepared.item_seqs.iter().any(Option::is_none));
    assert!(prepared.coverage_seqs.iter().all(Option::is_some));
    assert!(
        prepared.items.iter().any(|item| match item {
            Item::FunctionCallOutput(output) => matches!(
                &output.output,
                litecode::authority::responses::FunctionCallOutput::Text(text)
                    if text.contains("interrupted")
            ),
            _ => false,
        }),
        "unanswered call pad is part of the measured request"
    );

    let expected = envelope.fixed_prompt_tokens()
        + litecode::session::estimate::compute_token_estimate(&prepared.items);
    let auto_meter = measure_request_context(
        &sessions, &sid, &envelope, &baseline, &rows, &model, data_root,
    );
    let manual_meter =
        measure_persisted_context(&sessions, &sid, &envelope, &model, data_root).unwrap();
    let mut proj =
        Projection::new_with_model(sid.clone(), Arc::clone(&sessions), 10_000, Some(model));
    proj.set_context_envelope(envelope.clone());
    let snap = proj.snapshot("/proj", &binding(10_000, 1_024));

    for (label, meter) in [
        ("auto", &auto_meter),
        ("manual", &manual_meter),
        ("prepared", &prepared.context_meter),
        ("ui", &snap.context_meter),
    ] {
        assert_eq!(meter.used_tokens, expected, "{label} projected items");
        assert_eq!(meter.source, ContextMeterSource::LocalEnvelope, "{label}");
        assert_eq!(meter.compact_at, auto_meter.compact_at, "{label}");
    }
    assert_eq!(snap.context_tokens_estimate, expected);
    assert_eq!(snap.compact_eligible, auto_meter.manual_eligible());
}

#[tokio::test(flavor = "current_thread")]
async fn degenerate_windows_share_one_meter_across_pipeline_auto_manual_and_ui() {
    let fixture = SessionDataFixture::new();
    let sessions = Arc::new(fixture.manager());
    let data_root = fixture.dir.path();

    for window in [0usize, 1, 2, 8] {
        let model = model_with(window, 8);
        let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
        sessions
            .insert_detail_rows(&sid, &[user_text("window fixture")])
            .unwrap();
        let request = provider_request(&sid, window, 8);
        let envelope = ContextEnvelope::from_compaction_request(&request);
        let rows = sessions.data().working_set_blocking(&sid).unwrap();
        let baseline = ProviderPromptBaseline::default();

        let pipeline =
            ContextPipeline::new(window, test_context(data_root), data_root.to_path_buf());
        pipeline.freeze_request_envelope(envelope.clone());
        pipeline.stage_working(rows.clone());
        pipeline
            .build_view(&sessions, &sid, &baseline, &model)
            .expect("build prepared view");
        let prepared = pipeline.prepared_view().expect("prepared view");
        assert_eq!(pipeline.context_window(), window, "pipeline raw W={window}");
        assert_eq!(
            prepared.context_meter.context_window, window,
            "prepared raw W={window}"
        );

        let auto = measure_request_context(
            &sessions, &sid, &envelope, &baseline, &rows, &model, data_root,
        );
        let manual =
            measure_persisted_context(&sessions, &sid, &envelope, &model, data_root).unwrap();
        let mut proj = Projection::new_with_model(
            sid.clone(),
            Arc::clone(&sessions),
            window,
            Some(model.clone()),
        );
        proj.set_context_envelope(envelope.clone());
        let snap = proj.snapshot("/proj", &binding(window, 8));

        assert_eq!(
            pipeline.current_token_estimate(&prepared.items),
            auto.used_tokens,
            "current estimate W={window}"
        );
        assert_eq!(
            pipeline.will_compact(&prepared.items, 0),
            auto.should_compact(),
            "will_compact W={window}"
        );

        for (label, meter) in [
            ("auto", &auto),
            ("manual", &manual),
            ("prepared", &prepared.context_meter),
            ("ui", &snap.context_meter),
        ] {
            assert_eq!(meter.context_window, window, "{label} W={window}");
            assert_eq!(
                meter.used_tokens, auto.used_tokens,
                "{label} used W={window}"
            );
            assert_eq!(meter.source, auto.source, "{label} source W={window}");
            assert_eq!(
                meter.prompt_limit, auto.prompt_limit,
                "{label} prompt_limit W={window}"
            );
            assert_eq!(
                meter.compact_at, auto.compact_at,
                "{label} compact_at W={window}"
            );
            assert_eq!(
                meter.should_compact(),
                auto.should_compact(),
                "{label} should_compact W={window}"
            );
            assert_eq!(
                meter.manual_eligible(),
                auto.manual_eligible(),
                "{label} manual W={window}"
            );
            assert_eq!(
                meter.is_over_prompt_limit(),
                auto.is_over_prompt_limit(),
                "{label} hard-limit W={window}"
            );
        }
        if window <= 1 {
            assert!(!prepared.context_meter.pressure_enabled());
            assert!(!auto.should_compact());
            assert!(!auto.manual_eligible());
            assert_eq!(auto.compact_at, 0);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn step_and_turn_completion_snapshots_keep_breakdown_aligned_with_meter() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let model = test_model();
    let data_root = fixture.dir.path();

    sessions
        .insert_detail_rows(&sid, &[user_text("covered prefix")])
        .unwrap();
    let header = sessions
        .append_request_origin(
            &sid,
            &serde_json::json!({"schema": 1, "turn": "t", "step": 1}),
        )
        .unwrap();
    sessions
        .record_provider_usage(
            &sid,
            header,
            &litecode::session::store::ProviderUsage {
                prompt_tokens: 7_000,
                completion_tokens: 10,
                cache_hit_tokens: 0,
                cache_miss_tokens: 7_000,
            },
        )
        .unwrap();

    let request = provider_request(&sid, 10_000, 1_024);
    let envelope = ContextEnvelope::from_compaction_request(&request);
    let mut proj = Projection::new_with_model(
        sid.clone(),
        Arc::clone(&sessions),
        10_000,
        Some(model.clone()),
    );
    proj.set_context_envelope(envelope.clone());
    let before = proj.snapshot("/proj", &binding(10_000, 1_024));
    assert_eq!(before.context_meter.used_tokens, 7_000);
    assert_eq!(before.context_tokens_estimate, 7_000);

    // StepCommitted must refresh both sides together after a suffix row lands.
    let first_suffix = user_text("suffix after the completed provider request");
    let first_suffix_tokens =
        litecode::session::estimate::compute_token_estimate(&[first_suffix.clone()]);
    sessions.insert_detail_rows(&sid, &[first_suffix]).unwrap();
    proj.on_event(
        InternalEvent::StepCommitted,
        "/proj",
        &binding(10_000, 1_024),
    );
    let out = proj.take_outgoing();
    let step_snapshot = out
        .iter()
        .rev()
        .find(|msg| msg["method"] == "session/snapshot")
        .expect("step-committed snapshot");
    let current_rows = sessions.data().working_set_blocking(&sid).unwrap();
    let expected_breakdown =
        request_token_breakdown_for_rows(&current_rows, &envelope, &model, data_root);
    assert_eq!(
        step_snapshot["params"]["context_meter"]["used_tokens"],
        7_000 + first_suffix_tokens
    );
    assert_eq!(
        step_snapshot["params"]["context_token_breakdown"],
        serde_json::to_value(&expected_breakdown).unwrap()
    );

    // TurnCompleted follows the same refresh path; a second suffix must update
    // both the meter and the mix again.
    let second_suffix = user_text("final assistant turn suffix");
    let second_suffix_tokens =
        litecode::session::estimate::compute_token_estimate(&[second_suffix.clone()]);
    sessions.insert_detail_rows(&sid, &[second_suffix]).unwrap();
    proj.on_event(
        InternalEvent::TurnCompleted {
            turn_id: "t".into(),
            final_text: Some("done".into()),
            reason: TurnEndReason::Completed,
            turn_token_stats: TurnTokenStats {
                prompt_tokens: 7_000,
                completion_tokens: 10,
                cache_hit_tokens: 0,
                cache_miss_tokens: 7_000,
            },
            committed_next_seq: 0,
            error_kind: None,
            cancelled_by: None,
        },
        "/proj",
        &binding(10_000, 1_024),
    );
    let out = proj.take_outgoing();
    let finished = out
        .iter()
        .rev()
        .find(|msg| msg["method"] == "agent/turn_finished")
        .expect("turn finished snapshot");
    let current_rows = sessions.data().working_set_blocking(&sid).unwrap();
    let expected_breakdown =
        request_token_breakdown_for_rows(&current_rows, &envelope, &model, data_root);
    assert_eq!(
        finished["params"]["snapshot"]["context_meter"]["used_tokens"],
        7_000 + first_suffix_tokens + second_suffix_tokens
    );
    assert_eq!(
        finished["params"]["snapshot"]["context_token_breakdown"],
        serde_json::to_value(&expected_breakdown).unwrap()
    );
}

#[test]
fn latest_persisted_envelope_round_trips_for_cold_snapshot() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let model = test_model();
    let tools = vec![litecode::llm::ToolDef {
        name: "read_file".into(),
        description: "read a file".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    }];
    let envelope = ContextEnvelope::from_request_parts("system prompt", &tools, 512);
    sessions
        .append_request_origin(
            &sid,
            &serde_json::json!({
                "schema": 1,
                "turn": "t",
                "step": 1,
                "context_envelope": envelope.clone(),
            }),
        )
        .unwrap();
    sessions
        .insert_detail_rows(&sid, &[user_text("cold history")])
        .unwrap();

    assert_eq!(
        litecode::context_pipeline::latest_persisted_envelope(&sessions, &sid),
        Some(envelope.clone())
    );
    let proj = Projection::new_with_model(sid.clone(), Arc::clone(&sessions), 10_000, Some(model));
    let snap = proj.snapshot("/proj", &binding(10_000, 512));
    let mut expected_snapshot_envelope = envelope.clone();
    expected_snapshot_envelope.output_reserve = 8_192;
    assert_eq!(snap.context_envelope, expected_snapshot_envelope);
    assert_eq!(snap.context_meter.output_reserve, 8_192);
    assert!(snap.context_envelope.tools_tokens > 0);
    assert_eq!(snap.context_meter.source, ContextMeterSource::LocalEnvelope);
    assert!(
        snap.context_meter.used_tokens
            >= snap.context_envelope.instructions_tokens
                + snap.context_envelope.tools_tokens
                + snap.context_envelope.prompt_overhead_tokens
    );
}
#[tokio::test(flavor = "current_thread")]
async fn standard_and_max_modes_select_product_window_but_share_model_prompt_limit() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let data_root = fixture.dir.path();
    let mut model = (*test_model()).clone();
    model.context_window = 256_000;
    model.context_window_max = 1_000_000;
    model.max_output = 128_000;
    let model = Arc::new(model);
    let envelope = ContextEnvelope::from_request_parts("system", &[], 128_000);
    let baseline = ProviderPromptBaseline::default();

    let standard = measure_request_context(
        &sessions,
        &sid,
        &envelope,
        &baseline,
        &[],
        &model,
        data_root,
    );
    assert_eq!(standard.context_window, 256_000);
    assert_eq!(standard.prompt_limit, 872_000);
    assert_eq!(standard.compact_at, 204_800);
    let standard_ui =
        measure_persisted_context(&sessions, &sid, &envelope, &model, data_root).unwrap();
    assert_eq!(standard_ui.context_window, 256_000);
    assert_eq!(standard_ui.compact_at, 204_800);

    sessions.set_context_mode(&sid, ContextMode::Max).unwrap();
    let max = measure_request_context(
        &sessions,
        &sid,
        &envelope,
        &baseline,
        &[],
        &model,
        data_root,
    );
    assert_eq!(max.context_window, 1_000_000);
    assert_eq!(max.prompt_limit, 872_000);
    assert_eq!(max.compact_at, 800_000);
    let max_ui = measure_persisted_context(&sessions, &sid, &envelope, &model, data_root).unwrap();
    assert_eq!(max_ui.context_window, 1_000_000);
    assert_eq!(max_ui.compact_at, 800_000);
}
