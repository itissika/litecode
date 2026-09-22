//! Subagent tools expose durable Session turn facts without a second outcome model.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::ScriptedProvider;
use common::scripted_provider::HangProvider;
use common::subagent_fixture::SubagentHarness;
use litecode::client_protocol::controller::SessionController;
use litecode::config::resolved::resolve;
use litecode::config::schema::{AgentProfile, AgentRole, GlobalSettings};
use litecode::config::workspace::set_runtime_paths;
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::platform_knobs::{ContextMode, ThinkingSpec, ThinkingTier};
use litecode::runtime::{AgentIdentity, ProviderRegistry, TurnOptions, resolve_session_llm};
use litecode::types::{Item, Result, StreamEvents, ToolSignalLevel};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_child_result_is_read_from_the_durable_turn() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text(
        "child finished cleanly",
    )));
    let child = harness.launch_reviewer("call_ok", "do the thing").await;
    harness.wait_child_settled(&child);

    let result = harness
        .sessions
        .data()
        .latest_turn_result_blocking(&child)
        .unwrap()
        .expect("turn result");
    assert_eq!(result.reason, "completed");
    assert_eq!(result.output, "child finished cleanly");
    assert!(result.transcript_path.ends_with(&format!("{child}.md")));
    assert!(result.start_line.is_some() && result.end_line.is_some());

    let listed = harness.list("call_list_idle").await;
    assert_eq!(listed.level, ToolSignalLevel::Ok, "{}", listed.content);
    assert!(listed.content.contains(&format!("- id: {child}")), "{}", listed.content);
    assert!(listed.content.contains("agent: reviewer"), "{}", listed.content);
    assert!(listed.content.contains("responsibility: test"), "{}", listed.content);
    assert!(listed.content.contains("last_send:"), "{}", listed.content);
    assert!(listed.content.contains("state: idle"), "{}", listed.content);
    assert!(listed.content.contains("reason: completed"), "{}", listed.content);
    assert!(!listed.content.contains("turn_age:"), "{}", listed.content);

    let waited = harness
        .wait(
            "call_wait_idle",
            serde_json::json!({ "ids": [child.clone(), "missing-id"] }),
        )
        .await;
    assert_eq!(waited.level, ToolSignalLevel::Ok, "{}", waited.content);
    assert!(waited.content.contains("status: settled"), "{}", waited.content);
    assert!(waited.content.contains("reason: completed"), "{}", waited.content);
    assert!(waited.content.contains("responsibility: test"), "{}", waited.content);
    assert!(waited.content.contains("skipped: 1"), "{}", waited.content);
    assert!(waited.content.contains("skipped_id: missing-id"), "{}", waited.content);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_freezes_a_child_turn_and_returns_its_session_result() {
    let hang = HangProvider::until_cancel();
    let started = Arc::clone(&hang.started);
    let harness = SubagentHarness::new(Arc::new(hang));
    let child = harness.launch_reviewer("call_hang", "hang forever").await;
    harness.wait_for(|| started.load(Ordering::SeqCst), "child LLM start");

    let listed = harness.list("call_list_run").await;
    assert_eq!(listed.level, ToolSignalLevel::Ok, "{}", listed.content);
    assert!(listed.content.contains("state: running"), "{}", listed.content);
    assert!(listed.content.contains("turn_age:"), "{}", listed.content);
    assert!(listed.content.contains("step:"), "{}", listed.content);

    let (waited, stopped) = tokio::join!(
        harness.wait(
            "call_wait_hang",
            serde_json::json!({ "ids": [child.clone()] }),
        ),
        async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            harness.stop("call_stop_hang", &child).await
        }
    );
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);
    assert!(stopped.content.contains("status: stop_requested"));
    assert_eq!(waited.level, ToolSignalLevel::Ok, "{}", waited.content);
    assert!(
        waited.content.contains("status: settled"),
        "{}",
        waited.content
    );
    assert!(
        waited.content.contains("reason: cancelled"),
        "{}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_after_completion_returns_the_last_durable_result() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("done")));
    let child = harness.launch_reviewer("call_done", "quick task").await;
    harness.wait_child_settled(&child);

    let stopped = harness.stop("call_stop_done", &child).await;
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);
    assert!(stopped.content.contains("status: already ended"));
    assert!(stopped.content.contains("reason: completed"));
    assert!(stopped.content.contains("responsibility: test"));
    assert!(stopped.content.contains("output:\ndone"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_reminder_resolves_the_reference_at_delivery_time() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("reminder body")));
    let child = harness
        .launch_reviewer("call_reminder", "produce a reminder")
        .await;
    harness.wait_mailbox(&harness.parent_id);

    let completions = harness
        .runtime
        .subagent_hub
        .take_completions(&harness.parent_id);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].child_session_id, child);
    let reminder = litecode::tools::subagent::status::format_completion_reminder(
        &harness.sessions,
        &completions,
    );
    assert!(reminder.starts_with("<system-reminder>"));
    assert!(reminder.contains("source: subagent"));
    assert!(reminder.contains("reason: completed"));
    assert!(reminder.contains("responsibility: test"));
    assert!(reminder.contains("output:\nreminder body"));
}

// ── Turn config parity: the knobs live on the session row, children included ──

/// Records the model + thinking spec each turn actually asked the provider for.
#[derive(Clone)]
struct CaptureProvider {
    inner: ScriptedProvider,
    captured: Arc<Mutex<Vec<(String, ThinkingSpec)>>>,
}

impl CaptureProvider {
    fn new(texts: &[&str]) -> Self {
        Self {
            inner: ScriptedProvider::with_texts(texts),
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured(&self) -> Vec<(String, ThinkingSpec)> {
        self.captured.lock().expect("captured").clone()
    }
}

impl LlmProvider for CaptureProvider {
    fn endpoint(&self) -> &str {
        "capture://child-config"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.captured
            .lock()
            .expect("captured")
            .push((request.model.clone(), request.thinking));
        self.inner
            .complete_with_stream_events(request, api_key, on_event, cancel)
    }
}

fn api_model_id(resolved: &litecode::config::ResolvedConfig, model_id: &str) -> String {
    litecode::platform_knobs::effective_api_model_id(
        resolved.models().get(model_id).expect("model in test catalog"),
    )
}

/// A child turn is a session turn: model / thinking tier / context mode come from
/// the child's **own** row, so the writes the (derived) subagent UI makes must
/// change what the NEXT child turn sends. Regression pin for the deleted
/// child-side defaults (`ThinkingTier::default()` / `ContextMode::default()`
/// hardcoded at the call site).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_turn_reads_config_from_its_own_session_row() {
    let provider = CaptureProvider::new(&["ack one", "ack two"]);
    let harness = SubagentHarness::new(Arc::new(provider.clone()));
    let child = harness.launch_reviewer("call_cfg_1", "first").await;
    harness.wait_child_settled(&child);

    let launch_calls = provider.captured();
    let seed_call = launch_calls.last().expect("launch turn called the provider");
    assert_eq!(seed_call.0, api_model_id(&harness.resolved, "default"));
    assert_eq!(seed_call.1, ThinkingSpec::Tier(ThinkingTier::Medium));

    // Exactly the RPCs the subagent panel issues on a child session row.
    harness
        .sessions
        .set_session_model_id(&child, Some("compaction".into()))
        .expect("set model");
    harness
        .sessions
        .set_thinking_tier(&child, ThinkingTier::High)
        .expect("set tier");
    harness
        .sessions
        .set_context_mode(&child, ContextMode::Max)
        .expect("set mode");

    let sent = harness.send("call_cfg_2", &child, "second").await;
    assert!(!sent.content.starts_with("Error:"), "{}", sent.content);
    harness.wait_child_settled(&child);

    let all_calls = provider.captured();
    assert_eq!(
        all_calls.len(),
        launch_calls.len() + 1,
        "second child turn = one more provider call"
    );
    let turn_call = all_calls.last().expect("second turn called the provider");
    assert_eq!(
        turn_call.0,
        api_model_id(&harness.resolved, "compaction"),
        "child model comes from its own row"
    );
    assert_eq!(
        turn_call.1,
        ThinkingSpec::Tier(ThinkingTier::High),
        "child thinking tier comes from its own row"
    );
    // Identity is untouched by knob writes.
    assert_eq!(harness.sessions.agent_id(&child).as_deref(), Some("reviewer"));
}

fn child_binding(
    resolved: &litecode::config::ResolvedConfig,
    sessions: &Arc<litecode::session::manager::SessionManager>,
    session_id: &str,
) -> litecode::runtime::TurnLlmBinding {
    resolve_session_llm(
        resolved,
        &mut ProviderRegistry::new(),
        sessions,
        session_id,
        0,
    )
    .expect("child binding")
}

/// Display == execution: the window a child turn runs with is derived from its
/// own row's `context_mode`, through the single resolve entry `spawn_turn` uses
/// for every session kind.
#[tokio::test(flavor = "current_thread")]
async fn child_binding_window_follows_its_own_context_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path();
    let workspace = common::test_workspace(cwd);
    set_runtime_paths(workspace.paths.clone());

    // Declared window 1M → standard 200K vs max 1M, so the modes differ and the
    // assertion can actually fail.
    let mut global = GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        AgentProfile {
            model_ref: "default".into(),
            ..Default::default()
        },
    );
    global.agents.insert(
        "compaction".into(),
        AgentProfile {
            role: AgentRole::Hidden,
            model_ref: "compaction".into(),
            ..Default::default()
        },
    );
    common::insert_test_llm_registry(&mut global, "http://127.0.0.1:9", "test-key", 1_000_000);
    let resolved = resolve(global, workspace);

    let sessions = common::test_sessions_manager(
        resolved.paths().sessions_db.to_string_lossy().to_string(),
    );
    let project = cwd.to_string_lossy().to_string();
    let parent = sessions
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent session");
    let child = sessions
        .open_child_session(&project, "reviewer", Some("default"), &parent, "call-window")
        .expect("child session");

    let standard = child_binding(&resolved, &sessions, &child);
    assert_eq!(standard.context_mode, ContextMode::Standard);
    assert_eq!(
        standard.context_window,
        litecode::platform_knobs::CONTEXT_STANDARD_OPEN
    );

    sessions
        .set_context_mode(&child, ContextMode::Max)
        .expect("set mode");
    sessions
        .set_thinking_tier(&child, ThinkingTier::High)
        .expect("set tier");

    let max = child_binding(&resolved, &sessions, &child);
    assert_eq!(max.context_mode, ContextMode::Max);
    assert_eq!(max.thinking_tier, ThinkingTier::High);
    assert_eq!(
        max.context_window, 1_000_000,
        "turn budget follows the row's context mode"
    );
}

/// `TurnOptions` carries identity only — the LLM binding is never supplied by a
/// caller (that caller-supplied fork is what got deleted).
#[test]
fn turn_options_carry_identity_only() {
    let human = TurnOptions::default();
    assert!(matches!(human.identity, AgentIdentity::FromSession));
    let child = TurnOptions::child("reviewer");
    assert!(matches!(child.identity, AgentIdentity::Named(ref name) if name == "reviewer"));
}

/// D1: a child's agent identity is fixed by its profile. `agent/set-primary` is
/// answered with an explicit invalid-request refusal and the child keeps its own
/// `agent_id` — while model / tier / mode stay editable (their resolve path is
/// the same as a primary session's).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_identity_is_not_switchable_by_set_primary() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("ok")));
    let child = harness.launch_reviewer("call_d1", "hi").await;
    harness.wait_child_settled(&child);

    let mut ctrl = SessionController::new(
        harness.runtime.clone(),
        None,
        Arc::clone(&harness.sessions),
    )
    .expect("controller");
    ctrl.subscribe_checked(&child).await.expect("subscribe");
    let _ = ctrl.take_outgoing_for(&child);

    ctrl.set_active_primary(&child, "default")
        .expect("a refusal is reported on the wire, not as a hard error");

    let frames = ctrl.take_outgoing_for(&child);
    let refusal = frames
        .iter()
        .find(|f| {
            f["method"] == "agent/operation_result"
                && f["params"]["op"] == "set_active_primary"
        })
        .unwrap_or_else(|| panic!("set-primary answered on the wire: {frames:#?}"));
    assert_eq!(refusal["params"]["ok"], false);
    assert_eq!(refusal["params"]["error"]["code"], "invalid_request");
    assert_eq!(harness.sessions.agent_id(&child).as_deref(), Some("reviewer"));
}
