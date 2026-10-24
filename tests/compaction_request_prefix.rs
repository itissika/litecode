//! PR-1: compaction is a call mode of the main request, not a hidden model.
//!
//! A fake provider captures normal turn request(s) and the matching manual or
//! auto compaction request. Compaction must reuse the main model, instructions,
//! active tools, and the model-projected discarded prefix; only the trailing
//! instruction and output-side controls may differ.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::SessionDataFixture;
use common::recording_provider::{CapturedLlmRequest, RecordingProvider};
use litecode::authority::responses::{
    FunctionToolCall, InputContent, MessageItem, OutputStatus, ReasoningItem, ReasoningItemContent,
    ReasoningTextContent,
};
use litecode::client_protocol::controller::SessionController;
use litecode::config::global_db;
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard, WorkspaceState};
use litecode::context_pipeline::{
    BudgetPolicy, CompactPolicy, Context, ContextEnvelope, ContextMeter, ContextPipeline,
    ProviderPromptBaseline, keep_recent::find_keep_recent_cut, safety_headroom,
};
use litecode::engines::WorkspaceEngines;
use litecode::llm::{
    COMPACTION_INSTRUCTION, CompactionRequestContext, LlmProvider, RequestPurpose,
};
use litecode::optional::EngineManager;
use litecode::platform_knobs::ThinkingSpec;
use litecode::provider_catalog::ProviderCatalog;
use litecode::runtime::RuntimeHandle;
use litecode::runtime::observer::CompactionTrigger;
use litecode::session::EventType;
use litecode::session::manager::SessionManager;
use litecode::session::project_items;
use litecode::session::working::project_items as project_items_for_compaction;
use litecode::types::{Item, UserInput, item_text_preview, user_message, user_text};
use tokio_util::sync::CancellationToken;

fn controller(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
    provider: Arc<RecordingProvider>,
) -> SessionController {
    let catalog = common::test_catalog_toml(common::TEST_CATALOG_ENDPOINT, 128_000, 8192);
    controller_with_catalog(sessions, workspace_root, provider, &catalog)
}

fn controller_with_catalog(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
    provider: Arc<RecordingProvider>,
    catalog_text: &str,
) -> SessionController {
    controller_with_catalog_options(sessions, workspace_root, provider, catalog_text, false)
}

fn controller_with_catalog_options(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
    provider: Arc<RecordingProvider>,
    catalog_text: &str,
    remove_hidden_compaction: bool,
) -> SessionController {
    let db = workspace_root.join("global-litecode.db");
    let catalog = Arc::new(
        ProviderCatalog::parse(catalog_text, std::path::Path::new("<test-catalog>"))
            .expect("fixture catalog"),
    );
    litecode::provider_catalog::pin(&db, catalog);
    let mut baseline = common::test_resolved("default", &[]).global().clone();
    if remove_hidden_compaction {
        baseline.agents.remove("compaction");
    }
    common::insert_test_llm_registry(&mut baseline, "test-key");
    global_db::import_into(&db, &baseline).expect("seed global db");

    let guard = Arc::new(TurnGuard::new());
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(&db, guard);
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let revision = writer.revision_handle();
    let settings = writer.load_settings().expect("load");
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

async fn wait_until(mut condition: impl FnMut() -> bool, label: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if condition() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for {label}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn assert_same_main_envelope(main: &CapturedLlmRequest, compact: &CapturedLlmRequest) {
    assert_eq!(compact.model, main.model, "model must be inherited");
    assert_eq!(
        compact.instructions, main.instructions,
        "instructions must be byte-identical"
    );
    assert_eq!(compact.tools, main.tools, "active tools must be identical");
    assert_eq!(
        compact.request.session_id, main.request.session_id,
        "session affinity must be inherited"
    );
    assert_eq!(
        compact.request.json_output, main.request.json_output,
        "non-output-side request flags must be inherited"
    );
    assert_eq!(main.purpose, RequestPurpose::Main);
    assert_eq!(compact.purpose, RequestPurpose::Compaction);
    assert_eq!(
        compact.thinking, main.thinking,
        "compaction must inherit the main request thinking exactly"
    );
    assert_eq!(
        compact.max_output_tokens, 8192,
        "summary_cap must clamp to the model max_output"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compaction_request_reuses_main_model_instructions_tools_and_projected_prefix() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    // Roughly 90k local tokens on a 128k window: under the auto-compaction
    // threshold so the first turn records a clean main request, but above the
    // 30% manual-compaction eligibility floor.
    let seed: Vec<_> = (0..9)
        .map(|i| user_text(format!("prefix-seed-{i} {}", "x".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![
            vec![common::assistant_text_item("first answer", "msg_first")],
            vec![common::assistant_text_item("PR-1 summary", "msg_summary")],
        ],
    ));
    let captured = Arc::clone(&provider);
    let mut ctrl = controller(Arc::clone(&sessions), fixture.dir.path(), provider);

    ctrl.start_turn(
        &sid,
        UserInput::text("continue"),
        common::test_auto_approve_sink(),
        "turn-prefix-main",
        false,
    )
    .await
    .expect("start main turn");
    wait_until(
        || captured.captured().len() >= 1 && !sessions.is_turn_running_blocking(&sid),
        "first main request",
    )
    .await;

    let before_manual = sessions.data().working_set_blocking(&sid).unwrap();
    let budget = BudgetPolicy::new(128_000);
    let cut = find_keep_recent_cut(&project_items(&before_manual), budget.keep_recent_tokens)
        .expect("fixture must leave a discarded prefix");
    let main = captured.captured().remove(0);
    assert!(
        main.input.len() >= cut,
        "captured main request must cover the later discarded prefix"
    );

    ctrl.start_manual_compact(&sid)
        .await
        .expect("manual compaction starts from the session's main model");
    wait_until(
        || {
            captured.captured().len() >= 2
                && sessions
                    .data()
                    .events_blocking(&sid)
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event.event_type, EventType::Compacted))
        },
        "captured compaction request and durable checkpoint",
    )
    .await;

    let all = captured.captured();
    let compact = all
        .iter()
        .find(|request| request.purpose == RequestPurpose::Compaction)
        .expect("compaction request captured");
    assert_same_main_envelope(&main, compact);

    assert_eq!(
        compact.model,
        common::TEST_PRIMARY_MODEL_REF
            .split_once('/')
            .expect("provider/model ref")
            .1,
        "manual compaction must use the session's main model, not hidden compaction"
    );
    assert_eq!(
        &compact.input[..cut],
        &main.input[..cut],
        "discarded prefix must be the main request's projected prefix in original order"
    );
    assert_eq!(
        compact.input.len(),
        cut + 1,
        "only the trailing compaction instruction may follow the discarded prefix"
    );
    let instruction = compact
        .request
        .input
        .last()
        .expect("compaction instruction");
    assert_eq!(item_text_preview(instruction), COMPACTION_INSTRUCTION);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_request_uses_current_turn_binding_and_main_prefix() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    // First turn stays below the auto threshold and captures a clean main
    // request. A later direct history append pushes the next turn over 80%.
    let seed: Vec<_> = (0..9)
        .map(|i| user_text(format!("auto-prefix-seed-{i} {}", "x".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![
            vec![common::assistant_text_item(
                "first answer",
                "msg_auto_first",
            )],
            vec![common::assistant_text_item(
                "auto summary",
                "msg_auto_summary",
            )],
            vec![common::assistant_text_item(
                "second answer",
                "msg_auto_second",
            )],
        ],
    ));
    let captured = Arc::clone(&provider);
    let mut ctrl = controller(Arc::clone(&sessions), fixture.dir.path(), provider);

    ctrl.start_turn(
        &sid,
        UserInput::text("first turn"),
        common::test_auto_approve_sink(),
        "turn-auto-main-1",
        false,
    )
    .await
    .expect("start first turn");
    wait_until(
        || captured.captured().len() >= 1 && !sessions.is_turn_running_blocking(&sid),
        "first main request",
    )
    .await;
    let main = captured.captured().remove(0);

    let extra: Vec<_> = (0..12)
        .map(|i| user_text(format!("auto-prefix-extra-{i} {}", "y".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &extra).unwrap();
    let rows_before_second_turn = sessions.data().working_set_blocking(&sid).unwrap();
    let envelope = ContextEnvelope::cold(8192);
    let occupancy = ContextMeter::measure_local(
        128_000,
        envelope.output_reserve,
        safety_headroom(128_000),
        &envelope,
        &project_items(&rows_before_second_turn),
    );
    assert!(occupancy.should_compact(), "used={}", occupancy.used_tokens);
    let cut_before_second_turn = find_keep_recent_cut(
        &project_items(&rows_before_second_turn),
        BudgetPolicy::new(128_000).keep_recent_tokens,
    )
    .expect("pre-turn history must leave a discarded prefix");

    ctrl.start_turn(
        &sid,
        UserInput::text("second turn"),
        common::test_auto_approve_sink(),
        "turn-auto-main-2",
        false,
    )
    .await
    .expect("start second turn");
    wait_until(
        || {
            !sessions.is_turn_running_blocking(&sid)
                && sessions
                    .data()
                    .events_blocking(&sid)
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event.event_type, EventType::Compacted))
        },
        "auto compaction and second turn completion",
    )
    .await;

    let all = captured.captured();
    let compact = all
        .iter()
        .find(|request| request.purpose == RequestPurpose::Compaction)
        .expect("auto compaction request captured");
    assert_same_main_envelope(&main, compact);
    assert_eq!(
        compact.input.len(),
        cut_before_second_turn + 1,
        "auto compaction must use the exact pre-turn projected cut plus one instruction"
    );
    assert_eq!(
        &compact.input[..main.input.len()],
        &main.input[..],
        "auto compaction must reuse the current turn's main-model projected prefix"
    );
    let instruction = compact
        .request
        .input
        .last()
        .expect("compaction instruction");
    assert_eq!(item_text_preview(instruction), COMPACTION_INSTRUCTION);
}

fn signed_reasoning(text: &str, signature: &str) -> litecode::types::Item {
    litecode::types::Item::Reasoning(ReasoningItem {
        id: Some(format!("rs-{signature}")),
        summary: vec![],
        content: Some(vec![ReasoningItemContent::ReasoningText(
            ReasoningTextContent { text: text.into() },
        )]),
        encrypted_content: Some(signature.into()),
        status: Some(OutputStatus::Completed),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compaction_prefix_keeps_signed_reasoning_identical_to_main_request() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());

    let seed: Vec<_> = (0..9)
        .map(|i| user_text(format!("reasoning-prefix-seed-{i} {}", "x".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![
            vec![
                signed_reasoning("signed plan", "sig-main"),
                common::assistant_text_item("first answer", "msg_reasoning_first"),
            ],
            vec![common::assistant_text_item(
                "second answer",
                "msg_reasoning_second",
            )],
            vec![common::assistant_text_item(
                "reasoning summary",
                "msg_reasoning_summary",
            )],
        ],
    ));
    let captured = Arc::clone(&provider);
    // Anthropic Messages is the port profile whose signed thinking replay is
    // conditional on thinking intent, so it catches a compaction prefix built
    // with Off instead of the main request's replay mode.
    let catalog = common::test_catalog_toml(common::TEST_CATALOG_ENDPOINT, 128_000, 8192).replace(
        "endpoint_type = \"responses\"",
        "endpoint_type = \"messages\"",
    );
    let mut ctrl = controller_with_catalog(
        Arc::clone(&sessions),
        fixture.dir.path(),
        provider,
        &catalog,
    );

    ctrl.start_turn(
        &sid,
        UserInput::text("first turn"),
        common::test_auto_approve_sink(),
        "turn-reasoning-1",
        false,
    )
    .await
    .expect("start first turn");
    wait_until(
        || captured.captured().len() >= 1 && !sessions.is_turn_running_blocking(&sid),
        "first main request",
    )
    .await;

    // Stay below auto-compaction, but leave more than keep-recent to discard so
    // the signed reasoning is inside the later compaction prefix.
    let extra: Vec<_> = (0..4)
        .map(|i| user_text(format!("reasoning-prefix-extra-{i} {}", "y".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &extra).unwrap();

    ctrl.start_turn(
        &sid,
        UserInput::text("second turn"),
        common::test_auto_approve_sink(),
        "turn-reasoning-2",
        false,
    )
    .await
    .expect("start second turn");
    wait_until(
        || captured.captured().len() >= 2 && !sessions.is_turn_running_blocking(&sid),
        "second main request with signed reasoning",
    )
    .await;

    let main = captured
        .captured()
        .into_iter()
        .filter(|request| request.purpose == RequestPurpose::Main)
        .last()
        .expect("second main request");
    assert!(
        main.request
            .input
            .iter()
            .any(|item| matches!(item, litecode::types::Item::Reasoning(_))),
        "main request must carry the signed reasoning block"
    );

    let before_manual = sessions.data().working_set_blocking(&sid).unwrap();
    let cut = find_keep_recent_cut(
        &project_items(&before_manual),
        BudgetPolicy::new(128_000).keep_recent_tokens,
    )
    .expect("fixture must leave a discarded prefix");
    assert!(cut <= main.input.len(), "main request must cover the cut");
    ctrl.start_manual_compact(&sid)
        .await
        .expect("manual compaction");
    wait_until(
        || {
            captured.captured().len() >= 3
                && sessions
                    .data()
                    .events_blocking(&sid)
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event.event_type, EventType::Compacted))
        },
        "signed-reasoning compaction request",
    )
    .await;

    let all = captured.captured();
    let compact = all
        .iter()
        .find(|request| request.purpose == RequestPurpose::Compaction)
        .expect("compaction request captured");
    assert_same_main_envelope(&main, compact);
    assert_eq!(
        &compact.input[..cut],
        &main.input[..cut],
        "signed reasoning in the discarded prefix must be byte-for-byte identical"
    );
    assert!(
        compact.request.input[..cut]
            .iter()
            .any(|item| matches!(item, litecode::types::Item::Reasoning(_))),
        "compaction prefix must still carry signed reasoning"
    );
}

/// Auto compaction must append exactly one instruction item to the model
/// projection cut, including synthetic FunctionCallOutput pads and text-only
/// media placeholders. Those two projection mutations are not persisted source
/// rows, so a naive source-index/cut comparison would lose them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_uses_exact_cut_plus_one_with_pad_and_media_prefix() {
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        String::new(),
    ));
    let sid = sessions
        .open_session_sync("/proj", "default", Some("p/m"))
        .unwrap();
    let mut seed = vec![
        user_text("old-one"),
        user_message("see image", &["litecode-media:missing.png".into()]),
        Item::FunctionCall(FunctionToolCall {
            call_id: "call-unanswered".into(),
            name: "read".into(),
            arguments: "{}".into(),
            id: Some("fc-unanswered".into()),
            status: None,
            namespace: None,
        }),
        user_text("after the unanswered call"),
    ];
    seed.extend((0..8).map(|i| user_text(format!("tail-{i}-{}", "word ".repeat(200)))));
    seed.push(user_text("latest-source-anchor"));
    seed.push(user_text("kept-tail"));
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let rows = sessions.data().working_set_blocking(&sid).unwrap();
    let target = ProviderCatalog::parse(
        r#"version = 1
[[providers]]
id = "p"
name = "Probe"
endpoint = "https://probe.invalid/v1"
endpoint_type = "responses"
[[models]]
id = "m"
provider_id = "p"
context_window = 128000
context_window_max = 128000
max_output = 8192
modalities = ["text"]
"#,
        Path::new("<prefix-catalog>"),
    )
    .unwrap()
    .model("p/m")
    .unwrap()
    .clone();

    let mut pipeline = ContextPipeline::new(
        128_000,
        Context {
            cwd: PathBuf::from("."),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(Path::new(".")),
            agents_md: None,
            claude_md: None,
        },
        sessions.data_root_path(),
    );
    let baseline = ProviderPromptBaseline::default();
    pipeline
        .build_view(&sessions, &sid, &baseline, &target)
        .expect("main model view");
    let prepared = pipeline.take_prepared_view().expect("prepared view");

    let cut = find_keep_recent_cut(&project_items_for_compaction(&rows), 1)
        .expect("fixture must leave a discarded prefix");
    assert!(cut > 0, "fixture must exercise a discarded prefix");
    assert!(
        prepared.items.iter().any(|item| matches!(
            item,
            Item::FunctionCallOutput(output) if output.call_id == "call-unanswered"
        )),
        "main projection must pad the unanswered call"
    );
    assert!(
        prepared.items.iter().any(|item| matches!(
            item,
            Item::Message(MessageItem::Input(message))
                if message.content.iter().any(|part| matches!(
                    part,
                    InputContent::InputText(text)
                        if text.text.contains("omitted: model 'p/m' does not support image")
                ))
        )),
        "main projection must replace the unsupported image with a placeholder"
    );

    let target_wire_model = target.id.clone();
    let summary = common::dsh_summary("latest-source-anchor");
    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![vec![common::assistant_text_item(
            &summary,
            "msg_pipeline_summary",
        )]],
    ));
    let captured_provider = Arc::clone(&provider);
    let provider: Arc<dyn LlmProvider> = provider;
    let request = CompactionRequestContext {
        provider,
        api_key: "test-key".into(),
        model: target.id.clone(),
        context_window: target.context_window,
        max_output_tokens: target.max_output,
        target,
        thinking: ThinkingSpec::Tier(Default::default()),
        instructions: "same system".into(),
        tools: Vec::new(),
        session_id: sid.clone(),
    };
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    let mut compact_rows = rows;
    let outcome = CompactPolicy::compact_transcript(
        &budget,
        &sessions,
        &sid,
        &request,
        litecode::context_pipeline::keep_recent::COMPACT_MAX_OUTPUT_TOKENS,
        &baseline,
        &mut compact_rows,
        &CancellationToken::new(),
        CompactionTrigger::Auto,
        None,
        &sessions.data_root_path(),
    )
    .await
    .expect("auto compaction");
    let artifact = outcome.expect("fixture must compact");
    assert_eq!(
        artifact.finish,
        litecode::context_pipeline::SummaryFinish::Completed,
        "successful auto compaction must return the provider finish"
    );
    assert_eq!(artifact.model, target_wire_model);

    let captured = captured_provider.captured();
    let compact = captured
        .iter()
        .find(|request| request.purpose == RequestPurpose::Compaction)
        .expect("compaction request captured");
    // The source cut is 11, but the projection inserts one synthetic
    // FunctionCallOutput pad before the cut boundary. The instruction is the
    // only item appended after that projected prefix.
    assert_eq!(
        compact.input.len(),
        cut + 2,
        "auto compaction input must be projected cut (including the pad) + one instruction"
    );
    let projected_prefix = &compact.request.input[..compact.request.input.len() - 1];
    assert_eq!(
        projected_prefix,
        &prepared.items[..projected_prefix.len()],
        "the complete compaction prefix must equal the main model projection, including pads and media rewrites"
    );
    assert!(
        projected_prefix.iter().any(|item| matches!(
            item,
            Item::FunctionCallOutput(output) if output.call_id == "call-unanswered"
        )),
        "synthetic pad must survive into the compaction prefix"
    );
    assert!(
        projected_prefix.iter().any(|item| matches!(
            item,
            Item::Message(MessageItem::Input(message))
                if message.content.iter().any(|part| matches!(
                    part,
                    InputContent::InputText(text)
                        if text.text.contains("omitted: model 'p/m' does not support image")
                ))
        )),
        "media placeholder must survive into the compaction prefix"
    );
    assert_eq!(
        item_text_preview(compact.request.input.last().expect("instruction")),
        COMPACTION_INSTRUCTION
    );
}

fn primary_wire_model() -> &'static str {
    common::TEST_PRIMARY_MODEL_REF
        .split_once('/')
        .expect("provider/model ref")
        .1
}

fn seed_manual_compactable(sessions: &SessionManager, sid: &str) {
    let seed: Vec<_> = (0..9)
        .map(|i| user_text(format!("manual-gate-seed-{i} {}", "z".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(sid, &seed).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_works_without_the_hidden_compaction_agent() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    seed_manual_compactable(&sessions, &sid);

    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![vec![common::assistant_text_item(
            "no hidden profile summary",
            "msg_no_hidden",
        )]],
    ));
    let captured = Arc::clone(&provider);
    let catalog = common::test_catalog_toml(common::TEST_CATALOG_ENDPOINT, 128_000, 8192);
    let mut ctrl = controller_with_catalog_options(
        Arc::clone(&sessions),
        fixture.dir.path(),
        provider,
        &catalog,
        true,
    );

    ctrl.start_manual_compact(&sid)
        .await
        .expect("manual compaction must not resolve a hidden compaction agent");
    wait_until(
        || {
            captured.captured().len() >= 1
                && sessions
                    .data()
                    .events_blocking(&sid)
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event.event_type, EventType::Compacted))
        },
        "manual compaction without hidden profile",
    )
    .await;

    let compact = captured
        .captured()
        .into_iter()
        .find(|request| request.purpose == RequestPurpose::Compaction)
        .expect("compaction request");
    assert_eq!(
        compact.model,
        primary_wire_model(),
        "compaction must use the session's primary main model, never a fallback profile"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_repairs_a_stale_session_model_like_launch_turn() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    seed_manual_compactable(&sessions, &sid);

    let provider = Arc::new(RecordingProvider::new_legacy_accepting_compaction_summary(
        vec![vec![common::assistant_text_item(
            "repaired model summary",
            "msg_repaired_model",
        )]],
    ));
    let captured = Arc::clone(&provider);
    let mut ctrl = controller(Arc::clone(&sessions), fixture.dir.path(), provider);

    sessions
        .set_session_model_id(&sid, Some("test/model-that-was-removed".into()))
        .expect("inject stale sticky model");
    ctrl.start_manual_compact(&sid)
        .await
        .expect("manual compaction start");
    wait_until(
        || {
            captured.captured().len() >= 1
                && sessions
                    .data()
                    .events_blocking(&sid)
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event.event_type, EventType::Compacted))
        },
        "manual compaction after stale-model repair",
    )
    .await;

    assert_eq!(
        sessions.session_model_id(&sid).as_deref(),
        Some(common::TEST_PRIMARY_MODEL_REF),
        "manual compaction must repair the sticky model with ensure_session_model"
    );
    let compact = captured
        .captured()
        .into_iter()
        .find(|request| request.purpose == RequestPurpose::Compaction)
        .expect("compaction request");
    assert_eq!(
        compact.model,
        primary_wire_model(),
        "the repaired session model must be the one frozen into the request"
    );
}
