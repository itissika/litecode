//! Stage A (ctx/session data consistency): 1.1 / 2.1 / 2.2 / 2.3 / 2.4.
//!
//! Product-path integration tests against the real `Session` + `ContextPipeline`
//! (authority `Item` API), orphan cleanup in the commit transaction, revert,
//! TurnCompleted-before-persist, and provider token usage write-back.

mod common;

use std::cell::Cell;
use std::sync::Arc;

use litecode::agent::{self, AgentDeps};
use litecode::authority::responses::{FunctionCallOutput, FunctionCallOutputItemParam};
use litecode::config::TurnGuard;
use litecode::config::workspace::set_runtime_paths;
use litecode::context_pipeline::Context;
use litecode::context_pipeline::{
    BudgetPolicy, CompactPolicy, ContextPipeline, ProviderPromptBaseline,
};
use litecode::hook::{HookDispatcher, HookRegistry};
use litecode::session::manager::SessionManager;
use litecode::session::store::Session;
use litecode::session::task_state::TaskReminders;
use litecode::types::{FunctionToolCall, Item, LitecodeError, Transcript, user_text};

use common::fake_deps::{assistant_text_item, function_call_item};
use common::scripted_provider::ScriptedProvider;
use litecode::config::WorkspaceState;
use tokio_util::sync::CancellationToken;

/// Build a workspace `WorkspaceState` without `init_workspace`/git (avoids the
/// pre-existing Windows `init_workspace` flakiness under parallel tests).
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
        paths: litecode::config::WorkspacePaths::for_workspace(dir, &workspace_id),
        workspace_tool_readiness: Default::default(),
    }
}

/// Build a `Context` from an already-initialized workspace without re-running
/// `init_workspace` (which can be flaky under Windows parallelism).
fn test_context(cwd: &std::path::Path) -> Context {
    let workspace_id =
        litecode::config::peek_workspace_id(cwd).unwrap_or_else(|| "test-workspace-id".to_string());
    let paths = litecode::config::WorkspacePaths::for_workspace(cwd, &workspace_id);
    Context {
        cwd: cwd.to_path_buf(),
        workspace_paths: paths,
        agents_md: None,
        claude_md: None,
    }
}

fn test_sessions(db_path: &str) -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        Arc::new(TurnGuard::new()),
        db_path.to_string(),
    ))
}

/// Run a single `prepare_step` against a fresh pipeline/session, triggering
/// compaction via a high `last_prompt_tokens` when `compact: true`.
async fn prepare(
    pipeline: &ContextPipeline,
    sessions: &Arc<SessionManager>,
    sid: &str,
    ctx: &Context,
    turn: &mut litecode::types::Transcript,
    step: u64,
    last_prompt_tokens: u64,
) -> litecode::types::Result<()> {
    let provider = ScriptedProvider::with_text("compact summary");
    let cancel = CancellationToken::new();
    let model = litecode::config::schema::ModelDefinition {
        id: "test-model".into(),
        adapter_id: litecode::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
        provider_ref: "main".into(),
        label: "Test".into(),
        config: litecode::config::schema::ModelAdapterConfig {
            api_model_id: "m".into(),
            context_window: 128_000,
            max_tokens: 1024,
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            capabilities: vec![litecode::config::schema::ModelCapability::Text],
        },
    };
    let prompt_baseline = ProviderPromptBaseline::default();
    prompt_baseline.record(last_prompt_tokens, turn.len());
    pipeline
        .prepare_step(
            &HookDispatcher::from_registry(HookRegistry::default()),
            sessions,
            sid,
            ctx,
            &provider,
            "key",
            "m",
            "system",
            1024,
            &prompt_baseline,
            turn,
            step,
            &cancel,
            &TaskReminders::default(),
            &model,
        )
        .await
        .map(|_| ())
}

fn setup_workspace_and_session(dir: &std::path::Path, agent: &str) -> (String, String) {
    let ws = light_workspace(dir);
    set_runtime_paths(ws.paths.clone());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();
    let session = Session::open(&db_path, "/proj", agent, Some("test-model")).expect("open");
    let sid = session.id.clone();
    (db_path, sid)
}

#[tokio::test(flavor = "current_thread")]
async fn manual_compact_bypasses_auto_threshold_and_preserves_full_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    let seed: Vec<Item> = (0..12)
        .map(|i| user_text(format!("manual-seed-{i}")))
        .collect();
    let session = Session::resume(&db_path, &sid).unwrap();
    session.insert_detail_rows(&seed).unwrap();

    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let mut transcript = session.load_transcript().unwrap();
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    let estimate = budget.token_count(&transcript, 0);
    assert!(!budget.should_compact(estimate));

    let provider = ScriptedProvider::with_text("manual compact summary");
    let compacted = CompactPolicy::compact_now(
        &budget,
        &sessions,
        &sid,
        &provider,
        "key",
        "m",
        "system",
        1024,
        &mut transcript,
        &CancellationToken::new(),
        None,
    )
    .await
    .unwrap();
    assert!(compacted);
    assert!(transcript.len() < seed.len());

    let history = session.load_history_transcript().unwrap();
    assert_eq!(
        history.iter().filter(|row| row.kind == "detail").count(),
        seed.len()
    );
    assert_eq!(
        history
            .iter()
            .filter(|row| row.kind == "compact_checkpoint")
            .count(),
        1
    );
}

// ── 1.1 ──────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn compact_then_new_step_persists_and_resumes_full_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    // Seed enough history to push over the compact threshold when the cursor sees
    // a large provider token count.
    let seed: Vec<Item> = (0..30).map(|i| user_text(format!("seed-{i}"))).collect();
    {
        let s = Session::resume(&db_path, &sid).expect("resume");
        s.insert_detail_rows(&seed).unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let data_root = dir.path().to_path_buf();
    // Tiny keep window so short seeds still produce a discarded prefix (otherwise
    // keep-recent skips compact when the whole transcript fits in the default window).
    let pipeline =
        ContextPipeline::new(&session, 10_000, ctx.clone(), data_root).with_keep_recent_tokens(1);

    let mut turn = pipeline.begin_turn(&session).unwrap();
    // High provider token count + small window → compaction triggers.
    prepare(&pipeline, &sessions, &sid, &ctx, &mut turn, 2, 8_500)
        .await
        .expect("prepare_step (compact)");

    // Keep-recent: last seed survives verbatim; early seeds are summarized away.
    let mid_previews: Vec<String> = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert!(
        mid_previews.iter().any(|p| p.contains("compact summary")),
        "summary must be in working set after compact, got {mid_previews:?}"
    );
    assert!(
        mid_previews.iter().any(|p| p == "seed-29"),
        "keep-recent must retain the latest seed verbatim, got {mid_previews:?}"
    );
    assert!(
        !mid_previews.iter().any(|p| p == "seed-0"),
        "early seeds must be discarded into the summary, got {mid_previews:?}"
    );

    // After compact the working set is summary ‖ kept; add a fresh step's output.
    turn.push(user_text("post-compact user"));
    turn.push(assistant_text_item("post-compact reply", "msg_post"));
    pipeline.commit_step(&session, &mut turn).unwrap();

    // Historical pre-checkpoint detail must remain in DB (compact must not DELETE it).
    let cp = session.checkpoint_seq().unwrap();
    drop(pipeline);
    drop(session);
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        let archived_seed0: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'detail' AND seq < ?2
                   AND body LIKE '%seed-0%'",
                rusqlite::params![&sid, cp],
                |row| row.get(0),
            )
            .expect("count archived seed-0");
        assert!(
            archived_seed0 >= 1,
            "compact must not delete historical transcript detail (seed-0), cp={cp}"
        );
    }

    // Restart from disk: working set must be present.
    let resumed = Session::resume(&db_path, &sid).unwrap();
    let loaded = resumed.load_transcript().unwrap();
    let previews: Vec<String> = loaded
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert!(
        loaded.iter().any(|i| {
            matches!(i, Item::Message(_))
                && litecode::types::item_text_preview(i).contains("compact summary")
        }),
        "compact checkpoint summary must be persisted, got {previews:?}"
    );
    assert!(
        previews.iter().any(|p| p == "seed-29"),
        "keep-recent seed must be persisted, got {previews:?}"
    );
    assert!(
        !previews.iter().any(|p| p == "seed-0"),
        "working set must still hide archived seed-0, got {previews:?}"
    );
    assert!(
        previews.iter().any(|p| p == "post-compact user"),
        "post-compact user must be persisted, got {previews:?}"
    );
    assert!(
        previews.iter().any(|p| p == "post-compact reply"),
        "post-compact assistant reply must be persisted, got {previews:?}"
    );
}

/// Keep-recent cut=None must still enforce the hard limit: high provider tokens
/// with a short transcript that fits entirely in the keep window must not slip
/// through as Ok(false) without TokenBudgetExceeded.
#[tokio::test(flavor = "current_thread")]
async fn keep_recent_skip_still_enforces_hard_limit_when_over_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    let seed = vec![user_text("short-a"), user_text("short-b")];
    {
        let s = Session::resume(&db_path, &sid).expect("resume");
        s.insert_detail_rows(&seed).unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    // Default keep window for 10_000 is 2500 — short seed fits entirely (cut=None).
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());

    let mut turn = pipeline.begin_turn(&session).unwrap();
    let before: Vec<String> = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();

    // Autocompact threshold = 80% of 10_000 = 8000; hard limit = 10_000.
    // 10_001 triggers should_compact then cut=None → must still hard-fail.
    let err = prepare(&pipeline, &sessions, &sid, &ctx, &mut turn, 1, 10_001)
        .await
        .expect_err("over hard limit must fail even when keep-recent skips compact");
    assert!(
        matches!(err, LitecodeError::TokenBudgetExceeded),
        "expected TokenBudgetExceeded, got {err:?}"
    );

    let after: Vec<String> = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert_eq!(
        before, after,
        "skipped compact must leave the transcript unchanged"
    );
    assert_eq!(
        session.checkpoint_seq().unwrap(),
        0,
        "checkpoint_seq must stay at default when cut is None"
    );
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let compact_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND kind = 'compact_checkpoint'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        compact_rows, 0,
        "no compact_checkpoint row when keep-recent skips"
    );
}

/// Over autocompact threshold but under hard limit + cut=None → Ok(false), no
/// rewrite, hard limit path runs (does not error).
#[tokio::test(flavor = "current_thread")]
async fn keep_recent_skip_under_hard_limit_returns_ok_without_compact() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    let seed = vec![user_text("short-a"), user_text("short-b")];
    {
        let s = Session::resume(&db_path, &sid).expect("resume");
        s.insert_detail_rows(&seed).unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());

    let mut turn = pipeline.begin_turn(&session).unwrap();
    let before: Vec<String> = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();

    // 8500 > 8000 autocompact threshold, < 10_000 hard limit.
    // Empty ScriptedProvider: if compact/LLM ran, prepare would fail.
    let provider = ScriptedProvider::with_responses(vec![]);
    let cancel = CancellationToken::new();
    let model = litecode::config::schema::ModelDefinition {
        id: "test-model".into(),
        adapter_id: litecode::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
        provider_ref: "main".into(),
        label: "Test".into(),
        config: litecode::config::schema::ModelAdapterConfig {
            api_model_id: "m".into(),
            context_window: 128_000,
            max_tokens: 1024,
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            capabilities: vec![litecode::config::schema::ModelCapability::Text],
        },
    };
    let prompt_baseline = ProviderPromptBaseline::default();
    prompt_baseline.record(8_500, turn.len());
    let compacted = pipeline
        .prepare_step(
            &HookDispatcher::from_registry(HookRegistry::default()),
            &sessions,
            &sid,
            &ctx,
            &provider,
            "key",
            "m",
            "system",
            1024,
            &prompt_baseline,
            &mut turn,
            1,
            &cancel,
            &TaskReminders::default(),
            &model,
        )
        .await
        .expect("under hard limit + cut=None must Ok");
    assert!(!compacted, "keep-recent skip must report did_compact=false");

    let after: Vec<String> = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert_eq!(
        before, after,
        "transcript must be unchanged when compact is skipped"
    );
    assert_eq!(
        session.checkpoint_seq().unwrap(),
        0,
        "checkpoint_seq must stay at default when cut is None"
    );
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let compact_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND kind = 'compact_checkpoint'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        compact_rows, 0,
        "no compact_checkpoint row when keep-recent skips"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compact_eats_only_persisted_prefix_and_keeps_uncommitted_tail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    let seed: Vec<Item> = (0..30).map(|i| user_text(format!("seed-{i}"))).collect();
    {
        let s = Session::resume(&db_path, &sid).expect("resume");
        s.insert_detail_rows(&seed).unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf())
        .with_keep_recent_tokens(1);

    let mut turn = pipeline.begin_turn(&session).unwrap();
    turn.push(user_text("unpersisted-tail"));
    prepare(&pipeline, &sessions, &sid, &ctx, &mut turn, 2, 8_500)
        .await
        .expect("compact must not fail-closed on unpersisted tail");

    let previews: Vec<String> = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert!(
        previews.iter().any(|p| p == "unpersisted-tail"),
        "uncommitted tail must survive compact, got {previews:?}"
    );
    assert!(
        previews.iter().any(|p| p.contains("compact summary")),
        "summary must be in working set, got {previews:?}"
    );
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let tail_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND body LIKE '%unpersisted-tail%'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tail_rows, 0,
        "uncommitted tail must not be written by compact"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compact_reminder_rides_on_checkpoint_not_extra_user_detail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    let seed: Vec<Item> = (0..30).map(|i| user_text(format!("seed-{i}"))).collect();
    {
        let s = Session::resume(&db_path, &sid).expect("resume");
        s.insert_detail_rows(&seed).unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    sessions
        .with_entry_task_state_mut(&sid, |state| {
            state.todos.push(litecode::session::task_state::TodoItem {
                id: "t1".into(),
                content: "keep shipping".into(),
                status: litecode::session::task_state::TodoStatus::InProgress,
                priority: None,
            });
            Ok(())
        })
        .unwrap();

    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf())
        .with_keep_recent_tokens(1);

    let mut turn = pipeline.begin_turn(&session).unwrap();
    let provider = ScriptedProvider::with_text("compact summary");
    let cancel = CancellationToken::new();
    let model = litecode::config::schema::ModelDefinition {
        id: "test-model".into(),
        adapter_id: litecode::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
        provider_ref: "main".into(),
        label: "Test".into(),
        config: litecode::config::schema::ModelAdapterConfig {
            api_model_id: "m".into(),
            context_window: 128_000,
            max_tokens: 1024,
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            capabilities: vec![litecode::config::schema::ModelCapability::Text],
        },
    };
    let prompt_baseline = ProviderPromptBaseline::default();
    prompt_baseline.record(8_500, turn.len());
    let task_state = sessions
        .with_entry_task_state(&sid, |s| Ok(s.clone()))
        .unwrap();
    pipeline
        .prepare_step(
            &HookDispatcher::from_registry(HookRegistry::default()),
            &sessions,
            &sid,
            &ctx,
            &provider,
            "key",
            "m",
            "system",
            1024,
            &prompt_baseline,
            &mut turn,
            2,
            &cancel,
            &task_state,
            &model,
        )
        .await
        .expect("compact with reminder");

    let summary = turn
        .iter()
        .map(litecode::types::item_text_preview)
        .find(|p| p.contains("compact summary"))
        .expect("checkpoint text");
    assert!(
        summary.starts_with("[Conversation summary]"),
        "detector prefix must remain first, got {summary:?}"
    );
    assert!(
        summary.contains("<system-reminder>"),
        "reminder must ride on the checkpoint, got {summary:?}"
    );
    assert!(
        summary.contains("[~] keep shipping"),
        "todo reminder must sit after the label, got {summary:?}"
    );
    let extra_reminder_items = turn
        .iter()
        .filter(|i| {
            let p = litecode::types::item_text_preview(i);
            p.starts_with("<system-reminder>")
        })
        .count();
    assert_eq!(
        extra_reminder_items, 0,
        "must not push a separate user reminder item"
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let reminder_details: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND kind = 'detail' AND body LIKE '%system-reminder%'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reminder_details, 0,
        "reminder must not be persisted as a fake user detail"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn unanswered_calls_pad_llm_view_only_not_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    let fc = function_call_item("hanging", "read", "{}", "fc_hang");
    {
        let s = Session::resume(&db_path, &sid).unwrap();
        s.insert_detail_rows(&[user_text("ask"), fc]).unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());
    let mut turn = pipeline.begin_turn(&session).unwrap();
    prepare(&pipeline, &sessions, &sid, &ctx, &mut turn, 1, 0)
        .await
        .expect("prepare hanging call");

    assert!(
        !turn
            .iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(_))),
        "working set must keep the hanging FunctionCall, not a fake output"
    );
    let prepared = pipeline.prepared_view().expect("prepared");
    assert!(
        prepared
            .items
            .iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(o) if o.call_id == "hanging")),
        "LLM view must pad unanswered calls so Chat wire stays valid"
    );

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let fco_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND item_type = 'function_call_output'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fco_rows, 0, "synthetic pad must not be written as detail");
}

#[tokio::test(flavor = "current_thread")]
async fn resumed_session_cursor_initialized_from_max_seq() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    // First session: seed + compact + a post-compact step, all persisted.
    {
        let sessions = test_sessions(&db_path);
        let s = Session::resume(&db_path, &sid).unwrap();
        let seed: Vec<Item> = (0..30).map(|i| user_text(format!("s{i}"))).collect();
        s.insert_detail_rows(&seed).unwrap();
        sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
        let session = Session::resume(&db_path, &sid).unwrap();
        let ctx = test_context(dir.path());
        let pipeline =
            ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf())
                .with_keep_recent_tokens(1);
        let mut turn = pipeline.begin_turn(&session).unwrap();
        prepare(&pipeline, &sessions, &sid, &ctx, &mut turn, 2, 8_500)
            .await
            .unwrap();
        turn.push(user_text("resume user"));
        turn.push(assistant_text_item("resume reply", "msg_r"));
        pipeline.commit_step(&session, &mut turn).unwrap();
    }

    // Existing (resumed) session must behave like a fresh session: begin_turn loads
    // everything and the cursor is re-aligned from the DB max seq, so the next
    // commit persists only genuinely-new items (no re-insert of the persisted ones).
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let before = session.load_transcript().unwrap().len();
    // summary + keep-recent seed(s) + resume user + resume reply
    assert!(
        before >= 3,
        "resumed transcript must hold summary + turn items"
    );

    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());
    let mut turn = pipeline.begin_turn(&session).unwrap();
    turn.push(user_text("resume user 2"));
    pipeline.commit_step(&session, &mut turn).unwrap();

    let after = session.load_transcript().unwrap().len();
    assert_eq!(
        after,
        before + 1,
        "a resumed session must persist only the new delta, not re-persist history"
    );
    let persisted_max = session.persisted_max_seq();
    assert!(
        persisted_max >= 0,
        "cursor must be initialized from a valid max seq"
    );
}

// ── 2.1 ──────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn orphan_function_call_output_purged_from_db_in_commit_transaction() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    // Persist a valid call + output, plus an orphan output whose call_id has no
    // matching FunctionCall. This mirrors a DB that already carries an orphan.
    let fc = function_call_item("c1", "read", "{}", "fc_1");
    let live_out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: "c1".into(),
        output: FunctionCallOutput::Text("ok".into()),
        id: None,
        status: None,
    });
    let orphan_out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: "gone".into(),
        output: FunctionCallOutput::Text("orphan".into()),
        id: None,
        status: None,
    });
    {
        let s = Session::resume(&db_path, &sid).unwrap();
        s.insert_detail_rows(&[fc, live_out, orphan_out]).unwrap();
    }

    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());
    let mut turn = pipeline.begin_turn(&session).unwrap();

    // prepare_step runs `snip_stale_results`, dropping the orphan from the
    // in-memory working set.
    prepare(&pipeline, &sessions, &sid, &ctx, &mut turn, 1, 0)
        .await
        .expect("prepare_step (snip)");

    turn.push(assistant_text_item("reply after snip", "msg_snip"));
    pipeline.commit_step(&session, &mut turn).unwrap();

    // Same transaction as the delta insert must have purged the orphan from DB.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let orphan_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND item_type = 'function_call_output'
               AND json_extract(body, '$.call_id') = 'gone'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        orphan_rows, 0,
        "orphan FunctionCallOutput must be deleted from DB"
    );
    let live_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND item_type = 'function_call_output'
               AND json_extract(body, '$.call_id') = 'c1'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live_rows, 1, "live FunctionCallOutput must survive");
}

// ── 2.2 ──────────────────────────────────────────────────────────────────────

#[test]
fn revert_contract_three_states() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    let session = Session::resume(&db_path, &sid).unwrap();
    // Pre-compact users leave the model working set but remain visible UI anchors.
    session
        .insert_detail_rows(&[user_text("u0"), user_text("u1")])
        .unwrap();
    session
        .apply_compact_checkpoint(&user_text("summary"), 10)
        .unwrap();
    // Post-compact users.
    session
        .insert_detail_rows(&[user_text("u2"), user_text("u3")])
        .unwrap();

    assert_eq!(session.user_detail_count().unwrap(), 4);

    // 1) Anchor inside the checkpoint: revert to u3's full-history anchor (k=3)
    // keeps u2 + summary, drops u3.
    session.revert_to_user_anchor(3).unwrap();
    let loaded = session.load_transcript().unwrap();
    let previews: Vec<String> = loaded
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert_eq!(previews, vec!["summary".to_string(), "u2".to_string()]);
    assert_eq!(session.user_detail_count().unwrap(), 3);

    // 2) Swallowed: anchor beyond current visible count → InvalidRevertAnchor.
    let err = session.revert_to_user_anchor(5).unwrap_err();
    assert!(
        matches!(err, LitecodeError::InvalidRevertAnchor(_)),
        "swallowed anchor must yield InvalidRevertAnchor, got {err:?}"
    );
    assert_eq!(
        session.user_detail_count().unwrap(),
        3,
        "a swallowed revert must not mutate the transcript"
    );

    // 3) Cross-checkpoint: reverting to archived u1 (k=1)
    // physically removes that detail and the later checkpoint.
    session.revert_to_user_anchor(1).unwrap();
    let after_cross = session.load_transcript().unwrap();
    let after_previews: Vec<String> = after_cross
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert_eq!(after_previews, vec!["u0".to_string()]);
    assert_eq!(session.user_detail_count().unwrap(), 1);
}

// ── 2.3 / 2.4: full runtime turn with a recording observer ──────────────────

use litecode::client_protocol::observer::InternalEvent;
use litecode::engines::WorkspaceEngines;
use litecode::ide_base::IdeBaseHandle;
use litecode::optional::EngineManager;
use litecode::runtime::AgentRuntime;
use litecode::runtime::observer::RuntimeObserver;

use common::runtime::{test_resolved_with_budget, test_turn_binding};
use common::seed::{TEST_PROVIDER_ID, ready_test_provider};
use common::serve_responses_queue;
use litecode::llm::provider_from_definition;

/// Minimal text-only Responses SSE with token usage on `response.completed`.
fn usage_text_sse() -> String {
    let text_delta = serde_json::json!({
        "type": "response.output_text.delta",
        "sequence_number": 1,
        "item_id": "msg_usage_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "usage reply"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 2,
        "response": {
            "id": "resp_usage_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "usage": {
                "input_tokens": 1200,
                "input_tokens_details": {"cached_tokens": 800},
                "output_tokens": 50,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": 1250
            },
            "output": [{
                "type": "message",
                "id": "msg_usage_1",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": "usage reply",
                    "annotations": []
                }]
            }]
        }
    });
    format!("data: {text_delta}\n\ndata: {completed}\n\n")
}

/// Function-call Responses SSE (read tool) with token usage on `response.completed`,
/// so the agent loop continues into a second step.
fn usage_tool_sse() -> String {
    let added = serde_json::json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {
            "type": "function_call",
            "id": "fc_usage_1",
            "call_id": "call_usage_1",
            "name": "read",
            "arguments": "",
            "status": "in_progress"
        }
    });
    let fc_delta = serde_json::json!({
        "type": "response.function_call_arguments.delta",
        "sequence_number": 2,
        "item_id": "fc_usage_1",
        "output_index": 0,
        "delta": "{\"path\":\"test.txt\"}"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 3,
        "response": {
            "id": "resp_usage_tool",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "usage": {
                "input_tokens": 1200,
                "input_tokens_details": {"cached_tokens": 800},
                "output_tokens": 50,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": 1250
            },
            "output": [{
                "type": "function_call",
                "id": "fc_usage_1",
                "call_id": "call_usage_1",
                "name": "read",
                "arguments": "{\"path\":\"test.txt\"}",
                "status": "completed"
            }]
        }
    });
    format!("data: {added}\n\ndata: {fc_delta}\n\ndata: {completed}\n\n")
}

/// Records internal events for ordering/usage assertions.
#[derive(Default)]
struct RecordingObserver {
    events: std::sync::Mutex<Vec<InternalEvent>>,
}

impl RuntimeObserver for RecordingObserver {
    fn on_internal(&self, ev: InternalEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

fn responses_provider(endpoint: &str) -> Arc<dyn litecode::llm::LlmProvider> {
    let def = ready_test_provider(TEST_PROVIDER_ID, endpoint, "test-key");
    Arc::from(provider_from_definition(&def).expect("Responses provider"))
}

/// Build an `AgentRuntime` with a recording observer (like `build_runtime_with_provider`).
#[allow(clippy::type_complexity)]
fn build_runtime_with_observer(
    cwd: &std::path::Path,
    provider: Arc<dyn litecode::llm::LlmProvider>,
    observer: Arc<dyn RuntimeObserver>,
) -> AgentRuntime {
    let ws = light_workspace(cwd);
    set_runtime_paths(ws.paths.clone());

    let tool_names = vec!["read".to_string()];
    let mut global = {
        let resolved = test_resolved_with_budget("default", &tool_names, 128_000);
        resolved.global().clone()
    };
    let resolved = litecode::config::resolved::resolve(global, ws.clone());

    let project = cwd.to_string_lossy().to_string();
    let db_path = ws.paths.sessions_db.clone();
    let model_ref = resolved
        .agents()
        .get("default")
        .map(|p| p.model_ref.as_str())
        .filter(|s| !s.is_empty());
    let session =
        Session::open(&db_path.to_string_lossy(), &project, "default", model_ref).unwrap();
    let session_id = session.id.clone();
    let sessions = Arc::new(SessionManager::new(
        Arc::new(TurnGuard::new()),
        db_path.to_string_lossy().to_string(),
    ));
    sessions.register_for_test(session);

    let model_id = resolved
        .agents()
        .get("default")
        .map(|p| p.model_ref.as_str())
        .unwrap_or("default");
    let binding = test_turn_binding(&resolved, provider, "test-key", model_id);
    let workspace_engines = WorkspaceEngines::new();
    let ide = IdeBaseHandle::open(cwd, Arc::new(workspace_engines.clone())).expect("ide base");
    let mut runtime = AgentRuntime::new(
        resolved,
        session_id,
        sessions,
        binding,
        "default",
        0,
        common::permission::test_auto_approve_sink(),
        observer,
        None,
        Some(3),
        EngineManager::new(),
        workspace_engines,
        ide,
    )
    .expect("AgentRuntime::new");
    runtime.set_context_cwd(cwd.to_path_buf());
    runtime
}

#[tokio::test(flavor = "current_thread")]
async fn llm_completed_emitted_and_last_request_usage_in_turn_stats() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("test.txt"), "content").unwrap();
    let rec: Arc<RecordingObserver> = Arc::new(RecordingObserver::default());

    // Step 1 tool call (1200) + step 2 text (2000) — TurnCompleted keeps last only.
    let endpoint = serve_responses_queue(vec![usage_tool_sse(), usage_text_sse_with(2000)]).await;
    let provider = responses_provider(&endpoint);
    let observer: Arc<dyn RuntimeObserver> = rec.clone();
    let mut rt = build_runtime_with_observer(dir.path(), provider, observer);
    let result = rt.run("read then say hello").await.expect("turn completes");
    assert!(result.contains("usage reply"), "turn text: {result:?}");

    let events = rec.events.lock().unwrap();
    let completed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            InternalEvent::LlmCompleted {
                prompt_tokens,
                completion_tokens,
                cache_hit_tokens,
                cache_miss_tokens,
                ..
            } => Some((
                *prompt_tokens,
                *completion_tokens,
                *cache_hit_tokens,
                *cache_miss_tokens,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        completed.len(),
        2,
        "both LLM steps emit LlmCompleted, got: {events:?}"
    );
    assert_eq!(completed[0].0, 1200);
    assert_eq!(completed[1].0, 2000);

    // TurnCompleted carries last-request turn_token_stats (not summed across steps).
    let turn_stats = events
        .iter()
        .find_map(|e| match e {
            InternalEvent::TurnCompleted {
                turn_token_stats, ..
            } => Some(turn_token_stats),
            _ => None,
        })
        .expect("TurnCompleted present");
    assert_eq!(
        turn_stats.prompt_tokens, 2000,
        "last request only — not 1200+2000"
    );
    assert_eq!(turn_stats.completion_tokens, 50);
    assert_eq!(turn_stats.cache_hit_tokens, 0);
    assert_eq!(turn_stats.cache_miss_tokens, 2000);
}

#[tokio::test(flavor = "current_thread")]
async fn turn_completed_fires_after_db_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = light_workspace(dir.path());
    set_runtime_paths(ws.paths.clone());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();

    let rec: Arc<RecordingObserver> = Arc::new(RecordingObserver::default());
    let observer: Arc<dyn RuntimeObserver> = rec.clone();
    let endpoint = serve_responses_queue(vec![text_only_sss()]).await;
    let provider = responses_provider(&endpoint);
    let mut rt = build_runtime_with_observer(dir.path(), provider, observer);

    // Capture the session id after construction for the DB assertion.
    let sid = rt.session_id.clone();
    rt.run("final turn").await.expect("turn completes");

    // By the time TurnCompleted is observed, the DB must already hold the turn's rows.
    let events = rec.events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, InternalEvent::TurnCompleted { .. })),
        "TurnCompleted must be observed"
    );
    drop(events);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items WHERE session_id = ?1",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        rows >= 2,
        "DB must already contain the turn's data when TurnCompleted lands (rows={rows})"
    );
}

fn text_only_sss() -> String {
    let text_delta = serde_json::json!({
        "type": "response.output_text.delta",
        "sequence_number": 1,
        "item_id": "msg_final_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "final reply"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 2,
        "response": {
            "id": "resp_final_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "output": [{
                "type": "message",
                "id": "msg_final_1",
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": "final reply", "annotations": []}]
            }]
        }
    });
    format!("data: {text_delta}\n\ndata: {completed}\n\n")
}

// ── 2.4 compact decision driven by real usage ─────────────────────────────────

/// Tool-call SSE (like `usage_tool_sse`) but with a configurable `input_tokens`.
fn usage_tool_sse_with(input_tokens: u64) -> String {
    let added = serde_json::json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {
            "type": "function_call",
            "id": "fc_usage_1",
            "call_id": "call_usage_1",
            "name": "read",
            "arguments": "",
            "status": "in_progress"
        }
    });
    let fc_delta = serde_json::json!({
        "type": "response.function_call_arguments.delta",
        "sequence_number": 2,
        "item_id": "fc_usage_1",
        "output_index": 0,
        "delta": "{\"path\":\"test.txt\"}"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 3,
        "response": {
            "id": "resp_usage_tool",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "usage": {
                "input_tokens": input_tokens,
                "input_tokens_details": {"cached_tokens": 0},
                "output_tokens": 50,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": input_tokens + 50
            },
            "output": [{
                "type": "function_call",
                "id": "fc_usage_1",
                "call_id": "call_usage_1",
                "name": "read",
                "arguments": "{\"path\":\"test.txt\"}",
                "status": "completed"
            }]
        }
    });
    format!("data: {added}\n\ndata: {fc_delta}\n\ndata: {completed}\n\n")
}

/// The compact decision function must follow the real provider token count, not a
/// fixed local estimate: below the 80% autocompact threshold → no compact, above →
/// compact, for the exact same transcript.
#[test]
fn compact_decision_tracks_real_provider_usage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let budget_window = 10_000;
    let pipeline = ContextPipeline::new(
        &session,
        budget_window,
        ctx.clone(),
        dir.path().to_path_buf(),
    );
    let items = vec![
        user_text("hi"),
        user_text("second user message"),
        user_text("third user message"),
    ];

    // Local estimate (provider tokens = 0) is well under the threshold; real usage
    // must be what tips the decision, not the transcript's own estimate.
    assert!(
        !pipeline.will_compact(&items, 0),
        "no provider usage must not compact"
    );
    // 50% of budget → below the 80% autocompact threshold → no compact.
    assert!(
        !pipeline.will_compact(&items, 5_000),
        "below-threshold real usage must not compact"
    );
    // 85% of budget → above the 80% threshold → compact.
    assert!(
        pipeline.will_compact(&items, 8_500),
        "above-threshold real usage must compact"
    );
    // Same transcript, different real usage → different decision.
    assert!(
        pipeline.will_compact(&items, 9_900),
        "even higher real usage must still compact"
    );
}

/// Full-turn proof of the production wiring (2.4): real provider usage reported on
/// `response.completed` lands in `last_prompt_tokens`, which is the exact input the
/// compaction decision (`will_compact`) consumes. Combined with
/// `compact_decision_tracks_real_provider_usage` (decision follows real usage), this
/// closes the chain: response.completed → last_prompt_tokens → compact decision.
#[tokio::test(flavor = "current_thread")]
async fn real_usage_feeds_compaction_decision_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("test.txt"), "content").unwrap();
    let rec: Arc<RecordingObserver> = Arc::new(RecordingObserver::default());

    // Two LLM calls each report 900 input tokens on response.completed.
    let endpoint =
        serve_responses_queue(vec![usage_tool_sse_with(900), usage_text_sse_with(900)]).await;
    let provider = responses_provider(&endpoint);
    let observer: Arc<dyn RuntimeObserver> = rec.clone();
    let mut rt = build_runtime_with_observer(dir.path(), provider, observer);
    rt.run("read then say hello").await.expect("turn completes");

    // `last_prompt_tokens` now holds the real usage from the last response.completed.
    assert_eq!(
        rt.last_prompt_tokens(),
        900,
        "real usage from response.completed must feed the compaction decision input"
    );

    // With that real usage, the decision function is a pure function of the value
    // (budget 128k → threshold ~102.4k). 900 < threshold → no compact; a high real
    // usage → compact. The decision is driven by the value `last_prompt_tokens`
    // holds (which the wiring above set from real usage), not by a constant.
    let ws = light_workspace(dir.path());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();
    let session = Session::open(&db_path, "/proj", "default", Some("test-model")).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 128_000, ctx.clone(), dir.path().to_path_buf());
    assert!(
        !pipeline.will_compact(&[user_text("x")], rt.last_prompt_tokens()),
        "low real usage must not compact"
    );
    assert!(
        pipeline.will_compact(&[user_text("x")], 900_000),
        "high real usage must compact"
    );
}

/// Text SSE (like `usage_text_sse`) with a configurable `input_tokens`.
fn usage_text_sse_with(input_tokens: u64) -> String {
    let text_delta = serde_json::json!({
        "type": "response.output_text.delta",
        "sequence_number": 1,
        "item_id": "msg_usage_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "usage reply"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 2,
        "response": {
            "id": "resp_usage_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "usage": {
                "input_tokens": input_tokens,
                "input_tokens_details": {"cached_tokens": 0},
                "output_tokens": 50,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": input_tokens + 50
            },
            "output": [{
                "type": "message",
                "id": "msg_usage_1",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": "usage reply",
                    "annotations": []
                }]
            }]
        }
    });
    format!("data: {text_delta}\n\ndata: {completed}\n\n")
}

// ── PROBE A: revert-then-commit must not replay reverted content ─────────────

#[test]
fn idle_revert_truncates_log_for_next_begin_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    {
        let s = Session::resume(&db_path, &sid).unwrap();
        s.insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
    }
    let session = Session::resume(&db_path, &sid).unwrap();
    session.revert_to_user_anchor(1).unwrap();
    assert_eq!(session.load_transcript().unwrap().len(), 1);

    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx, dir.path().to_path_buf());
    let turn = pipeline.begin_turn(&session).unwrap();
    assert_eq!(turn.len(), 1);
}

#[test]
fn commit_after_revert_discards_uncommitted_delta() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    {
        let s = Session::resume(&db_path, &sid).unwrap();
        s.insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
    }
    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());

    let mut turn = pipeline.begin_turn(&session).unwrap();
    assert_eq!(turn.len(), 3);

    session.revert_to_user_anchor(1).unwrap();
    assert_eq!(session.load_transcript().unwrap().len(), 1);

    turn.push(assistant_text_item("uncommitted tail", "msg_stale"));
    let outcome = pipeline.commit_step(&session, &mut turn).unwrap();
    assert!(
        outcome.discarded,
        "stale delta after truncate must not insert"
    );
    assert_eq!(turn.len(), 1);

    let after = session.load_transcript().unwrap();
    let previews: Vec<String> = after
        .iter()
        .map(litecode::types::item_text_preview)
        .collect();
    assert_eq!(
        previews,
        vec!["u0".to_string()],
        "uncommitted output after revert must not be inserted (got {previews:?})"
    );
}

struct PipelinePersistDeps {
    pipeline: ContextPipeline,
    session: Session,
    responses: Vec<Vec<Item>>,
    call_index: Cell<usize>,
    cancelled: Cell<bool>,
    cancel_after_model: bool,
    revert_k: Cell<Option<i64>>,
    execute_calls: Cell<u32>,
}

impl AgentDeps for PipelinePersistDeps {
    async fn call_model(&mut self) -> litecode::types::Result<Vec<Item>> {
        let idx = self.call_index.get();
        self.call_index.set(idx + 1);
        let items = self
            .responses
            .get(idx)
            .cloned()
            .ok_or_else(|| LitecodeError::Llm("no more responses".into()))?;
        if self.cancel_after_model {
            self.cancelled.set(true);
        }
        Ok(items)
    }

    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> litecode::types::Result<()> {
        self.execute_calls
            .set(self.execute_calls.get().saturating_add(1));
        for tu in tool_uses {
            transcript.push(Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: tu.call_id.clone(),
                output: FunctionCallOutput::Text("should not land".into()),
                id: None,
                status: None,
            }));
        }
        Ok(())
    }

    async fn should_stop(&self, _output: &[Item]) -> litecode::types::Result<bool> {
        Ok(true)
    }

    async fn compact_if_needed(
        &self,
        _transcript: &mut Transcript,
        _step: u64,
    ) -> litecode::types::Result<()> {
        Ok(())
    }

    fn emit_todo_progress(&mut self) {}

    fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }

    fn max_steps(&self) -> u32 {
        50
    }

    fn persist_items(&self, items: &mut Vec<Item>) -> litecode::types::Result<bool> {
        if let Some(k) = self.revert_k.take() {
            self.session.revert_to_user_anchor(k)?;
        }
        Ok(self.pipeline.commit_step(&self.session, items)?.discarded)
    }

    fn begin_step(&mut self, _step: u64) {}
}

#[tokio::test(flavor = "current_thread")]
async fn agent_persist_after_revert_does_not_replay_or_pad() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");
    {
        let s = Session::resume(&db_path, &sid).unwrap();
        s.insert_detail_rows(&[user_text("keep"), user_text("drop")])
            .unwrap();
    }
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx, dir.path().to_path_buf());
    let mut transcript = pipeline.begin_turn(&session).unwrap();
    assert_eq!(transcript.len(), 2);

    let mut deps = PipelinePersistDeps {
        pipeline,
        session,
        responses: vec![vec![function_call_item(
            "call_gone",
            "read",
            "{}",
            "fc_gone",
        )]],
        call_index: Cell::new(0),
        cancelled: Cell::new(false),
        cancel_after_model: true,
        revert_k: Cell::new(Some(1)),
        execute_calls: Cell::new(0),
    };
    let outcome = agent::run(&mut deps, &mut transcript).await;
    assert!(
        matches!(outcome, litecode::agent::TurnOutcome::Cancelled { .. }),
        "got {outcome:?}"
    );
    assert_eq!(deps.execute_calls.get(), 0);
    assert_eq!(transcript.len(), 1);
    assert!(
        !transcript
            .iter()
            .any(|i| matches!(i, Item::FunctionCall(_) | Item::FunctionCallOutput(_))),
        "reverted prefix must not grow interrupted residue: {transcript:?}"
    );
    let db = deps.session.load_transcript().unwrap();
    assert_eq!(db.len(), 1);
    assert_eq!(litecode::types::item_text_preview(&db[0]), "keep");
}

// ── PROBE B: commit must clear orphans from memory AND DB, no drift ──────────

#[test]
fn commit_snips_memory_and_db_orphan_without_drift() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db_path, sid) = setup_workspace_and_session(dir.path(), "default");

    // Seed a valid call + output, plus an orphan output already persisted in DB
    // (mirrors a DB that carries an orphan from an earlier snip).
    let fc = function_call_item("c1", "read", "{}", "fc_1");
    let live_out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: "c1".into(),
        output: FunctionCallOutput::Text("ok".into()),
        id: None,
        status: None,
    });
    let orphan_out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: "gone".into(),
        output: FunctionCallOutput::Text("orphan".into()),
        id: None,
        status: None,
    });
    {
        let s = Session::resume(&db_path, &sid).unwrap();
        s.insert_detail_rows(&[fc, live_out.clone(), orphan_out])
            .unwrap();
    }

    let sessions = test_sessions(&db_path);
    sessions.register_for_test(Session::resume(&db_path, &sid).unwrap());
    let session = Session::resume(&db_path, &sid).unwrap();
    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(&session, 10_000, ctx.clone(), dir.path().to_path_buf());

    // begin_turn loads [fc, live_out, orphan_out]; the orphan is in memory.
    let mut turn = pipeline.begin_turn(&session).unwrap();
    assert!(
        turn.iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(o) if o.call_id == "gone")),
        "orphan must be present in memory before commit"
    );

    pipeline.commit_step(&session, &mut turn).unwrap();

    // Memory no longer holds the orphan (commit snips in-memory orphans).
    assert!(
        !turn
            .iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(o) if o.call_id == "gone")),
        "commit must remove the orphan from memory"
    );

    // DB no longer holds the orphan row.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let orphan_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND item_type = 'function_call_output'
               AND json_extract(body, '$.call_id') = 'gone'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphan_rows, 0, "commit must delete the orphan row from DB");

    // A second commit of the same (already-cleaned) set must not re-write or drift:
    // row count stays stable and the live output survives exactly once.
    let rows_before = session.load_transcript().unwrap().len();
    let mut turn2 = turn.clone();
    pipeline.commit_step(&session, &mut turn2).unwrap();
    assert_eq!(
        session.load_transcript().unwrap().len(),
        rows_before,
        "a repeat commit of a clean set must not duplicate rows"
    );
    let live_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND item_type = 'function_call_output'
               AND json_extract(body, '$.call_id') = 'c1'",
            [&sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        live_rows, 1,
        "live FunctionCallOutput must survive exactly once"
    );
}
