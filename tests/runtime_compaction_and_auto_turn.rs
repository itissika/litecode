//! V2 evidence tests for the runtime / context-pipeline surface.
//!
//! Two sections, deliberately different in kind:
//!
//! * **RED-FIRST** — asserts the behaviour the V2 audit says the product
//!   requires but the code does not have. These MUST fail on unmodified
//!   `HEAD`; they are the evidence for a fix, not a regression guard yet.
//! * **GREEN-FIRST** — asserts invariants that hold today and that the two
//!   planned dedup refactors (`AgentRuntime::compact_if_needed`'s four
//!   settlement arms, and the bash/subagent idle auto-turn pair) are most
//!   likely to break. These MUST pass on unmodified `HEAD`.
//!
//! No production file is edited here and no thread-spawning installer is
//! called: every test drives a public decision function or a pure policy
//! function with an in-memory fixture.

mod common;

use std::collections::HashSet;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use litecode::config::resolved::WorkspaceState;
use litecode::config::{AgentConfig, TurnGuard};
use litecode::context_pipeline::{BudgetPolicy, CompactPolicy, build_system_prompt};
use litecode::engines::WorkspaceEngines;
use litecode::ide_base::IdeBaseHandle;
use litecode::optional::EngineManager;
use litecode::runtime::bash_auto_turn::{
    IdleAutoTurn as BashDecision, try_begin_idle_auto_turn as try_begin_bash_auto_turn,
};
use litecode::runtime::subagent_auto_turn::{
    IdleAutoTurn as SubagentDecision, try_begin_idle_auto_turn as try_begin_subagent_auto_turn,
};
use litecode::runtime::RuntimeHandle;
use litecode::session::manager::SessionManager;
use litecode::session::project_items;
use litecode::session::working::WorkingRow;
use litecode::session::{Seq, SessionService};
use litecode::terminal::TerminalHub;
use litecode::tools::subagent::{CompletionRef, SubagentHub};
use litecode::workspace::WorkspaceService;

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn agent_config(role: &str, system_prompt: &str) -> AgentConfig {
    AgentConfig {
        role: role.into(),
        system_prompt: system_prompt.into(),
        ..AgentConfig::default()
    }
}

/// One idle-capable session plus the two independent mailboxes the auto-turn
/// listeners read. Nothing here starts a turn; the decision functions are pure
/// reads of session + hub state.
struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    runtime: RuntimeHandle,
    sessions: Arc<SessionManager>,
    service: Arc<SessionService>,
    terminal: Arc<TerminalHub>,
    subagents: Arc<SubagentHub>,
    sid: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();

        let tools: &[String] = &[];
        let resolved = common::runtime::test_resolved_with_budget("default", tools, 128_000);
        let workspace_state = WorkspaceState::new(root.clone());
        let workspace = WorkspaceService::new(root.clone()).expect("workspace service");
        let engines = Arc::new(WorkspaceEngines::new());
        let terminal = Arc::new(TerminalHub::new());
        let ide = IdeBaseHandle::new(workspace, Arc::clone(&engines), Arc::clone(&terminal));
        let runtime = RuntimeHandle::new(
            resolved,
            "default".into(),
            workspace_state,
            Arc::new(EngineManager::new()),
            engines,
            ide,
            Arc::new(AtomicU64::new(0)),
            root.join("global.db"),
        );

        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            root.join("sessions.db").to_string_lossy().to_string(),
        ));
        let service = Arc::new(SessionService::new(Arc::clone(&sessions)));
        let subagents = Arc::new(SubagentHub::new());
        let sid = sessions
            .open_session_sync(&root.display().to_string(), "default", None)
            .expect("open session");

        Self {
            _dir: dir,
            root,
            runtime,
            sessions,
            service,
            terminal,
            subagents,
            sid,
        }
    }

    fn project(&self) -> String {
        self.root.display().to_string()
    }

    fn attach_viewer(&self) {
        let _ = self.sessions.attach(&self.sid);
    }

    /// Reserve a turn owned by someone else so the session reads as busy.
    fn reserve_foreign_turn(&self) {
        self.sessions
            .reserve_turn(&self.sid, "turn-foreign".into(), 10, "default", &self.project())
            .expect("reserve foreign turn");
    }

    fn release_foreign_turn(&self) {
        self.sessions
            .release_turn_reservation(&self.sid, "turn-foreign");
    }

    fn seed_bash_exit(&self) {
        self.terminal
            .jobs
            .insert_finished_pipe_job(
                "bg_v2_000001",
                &self.sid,
                "",
                "echo hi",
                &self.root,
                Some(0),
                false,
            )
            .expect("finished pipe job");
    }

    fn seed_subagent_completion(&self) {
        self.subagents.completions.push(CompletionRef {
            parent_session_id: self.sid.clone(),
            child_session_id: "child-v2".into(),
            turn_id: "turn-child".into(),
            cancelled: false,
            cancelled_by: None,
        });
    }

    fn bash_decision(&self) -> BashDecision {
        try_begin_bash_auto_turn(
            &self.terminal,
            &self.runtime,
            &self.service,
            &self.root,
            &self.sid,
        )
    }

    fn subagent_decision(&self) -> SubagentDecision {
        try_begin_subagent_auto_turn(
            &self.subagents,
            &self.runtime,
            &self.service,
            &self.root,
            &self.sid,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Tag {
    Prepared,
    SkippedBusy,
    SkippedNoUi,
    SkippedEmptyMailbox,
    SkippedSessionGone,
}

fn bash_tag(decision: &BashDecision) -> Tag {
    match decision {
        BashDecision::Prepared { .. } => Tag::Prepared,
        BashDecision::SkippedBusy => Tag::SkippedBusy,
        BashDecision::SkippedNoUi => Tag::SkippedNoUi,
        BashDecision::SkippedEmptyMailbox => Tag::SkippedEmptyMailbox,
        BashDecision::SkippedSessionGone => Tag::SkippedSessionGone,
    }
}

fn subagent_tag(decision: &SubagentDecision) -> Tag {
    match decision {
        SubagentDecision::Prepared { .. } => Tag::Prepared,
        SubagentDecision::SkippedBusy => Tag::SkippedBusy,
        SubagentDecision::SkippedNoUi => Tag::SkippedNoUi,
        SubagentDecision::SkippedEmptyMailbox => Tag::SkippedEmptyMailbox,
        SubagentDecision::SkippedSessionGone => Tag::SkippedSessionGone,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 1 — RED-FIRST (must FAIL on unmodified HEAD)
// ─────────────────────────────────────────────────────────────────────────────

/// RED — `src/context_pipeline/system.rs::resolve_body` silently blanks the
/// editable body of a *non-built-in* agent whose stored prompt happens to look
/// like a built-in marker.
///
/// `is_builtin_prompt_marker("builtin:default")` is `true`, but
/// `builtin_prompt_for("my-custom-agent")` is `None`, so the current code takes
/// `unwrap_or("")` and drops the user's configured text on the floor: a custom
/// agent configured with `builtin:default` gets *no* system body at all, with no
/// error, no log line and no way for the user to notice from the UI.
///
/// Required product behaviour: an agent id with no built-in pack must never
/// lose its stored text. Either keep the text verbatim, or reject the marker at
/// config-resolution time with a visible error. Silent erasure is the bug.
#[test]
fn custom_agent_with_builtin_marker_keeps_its_stored_body() {
    const CUSTOM_ID: &str = "my-custom-agent";
    const STORED: &str = "builtin:default";

    let named = build_system_prompt(CUSTOM_ID, &agent_config("subagent", STORED), None);
    let blank = build_system_prompt(CUSTOM_ID, &agent_config("subagent", ""), None);

    assert!(
        named.contains(STORED),
        "a custom agent id has no built-in pack, so its stored prompt must survive \
         verbatim; `resolve_body` returned an empty editable body instead\n\
         stored = {STORED:?}\n\
         rendered system prompt len = {} (identical to a blank prompt: {})",
        named.len(),
        named == blank
    );
    assert_ne!(
        named, blank,
        "stored prompt text must change the rendered system prompt; \
         `{CUSTOM_ID}` with {STORED:?} rendered byte-identically to an empty body"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 2 — GREEN-FIRST (must PASS on unmodified HEAD)
// ─────────────────────────────────────────────────────────────────────────────

/// Guards the single-splice refactor in `system.rs`: normal and bot paths share
/// `splice_harness_with_role`, so their layer order must stay identical and the
/// bot-only `# Bot Role` segment must never leak into a normal agent prompt.
///
/// Also pins the part of the RED fix that is *not* allowed to change: a
/// built-in id stays locked to its compile-time pack, so the fix for an unknown
/// id must not start trusting stored text for known ones.
#[test]
fn normal_and_bot_prompts_share_one_layer_order() {
    let bot = litecode::context_pipeline::build_system_prompt_for_bot(
        None,
        Some("You are the release captain."),
        "PLATFORM SEGMENT",
    );
    let order = [
        "# System",
        "# Tool Runtime (`run_code`)",
        "LiteCode's default coding agent",
        "# Bot Role\nYou are the release captain.",
        "PLATFORM SEGMENT",
        "# Citations",
    ];
    let mut cursor = 0usize;
    for layer in order {
        let at = bot
            .find(layer)
            .unwrap_or_else(|| panic!("bot prompt lost layer {layer:?}"));
        assert!(
            at >= cursor,
            "bot layer {layer:?} moved ahead of the previous layer ({at} < {cursor})"
        );
        cursor = at;
    }

    let normal = build_system_prompt("default", &agent_config("primary", "builtin:default"), None);
    assert!(
        !normal.contains("# Bot Role"),
        "the bot-only segment must not appear in a normal agent prompt"
    );
    assert!(normal.contains("# Citations"), "primary keeps citations");
    assert!(
        normal.contains("# System") && normal.contains("# Tool Runtime (`run_code`)"),
        "normal prompt must still open with the harness layers"
    );

    // The RED fix must not unlock built-ins: stored text is ignored for a
    // built-in id, whether it is a marker or hand-written prose.
    let seeded = build_system_prompt("default", &agent_config("primary", "builtin:default"), None);
    let overridden =
        build_system_prompt("default", &agent_config("primary", "CUSTOM OVERRIDE"), None);
    assert_eq!(
        seeded, overridden,
        "a built-in agent id must stay locked to its compile-time pack"
    );
    assert!(
        !overridden.contains("CUSTOM OVERRIDE"),
        "stored text leaked into a locked built-in prompt"
    );
}

/// Guards the bash idle auto-turn contract that the bash/subagent 2-in-1 dedup
/// must preserve:
///
/// * `Prepared` holds *the* turn reservation — the session is busy, a second
///   decision cannot also prepare, and the returned `turn_id` is the owner that
///   can release it (so a failed spawn/start still has a release path);
/// * the mailbox is NOT drained by the decision — the seam owns delivery.
#[test]
fn bash_prepared_holds_one_reservation_and_keeps_mailbox_pending() {
    let fixture = Fixture::new();
    fixture.attach_viewer();
    fixture.seed_bash_exit();

    let turn_id = match fixture.bash_decision() {
        BashDecision::Prepared { turn_id, .. } => turn_id,
        other => panic!(
            "expected Prepared on an idle, watched session with a pending bash exit, got {:?}",
            bash_tag(&other)
        ),
    };
    assert!(
        fixture.sessions.is_session_busy_blocking(&fixture.sid),
        "Prepared must hold the turn reservation before the spawn happens"
    );
    assert!(
        fixture.terminal.jobs.mailbox_pending(&fixture.sid),
        "the decision must not drain the mailbox; the request seam writes the reminder"
    );

    assert_eq!(
        bash_tag(&fixture.bash_decision()),
        Tag::SkippedBusy,
        "a second decision must not prepare a second turn for the same session"
    );
    assert!(
        fixture.terminal.jobs.mailbox_pending(&fixture.sid),
        "a losing decision must leave the mailbox untouched"
    );

    assert!(
        fixture.sessions.release_turn_reservation(&fixture.sid, &turn_id),
        "the Prepared turn id must be the real reservation owner so failure paths can release it"
    );
    assert!(!fixture.sessions.is_session_busy_blocking(&fixture.sid));
}

/// Guards the bash decision's *ordering*: mailbox emptiness is answered before
/// the UI gate, so an idle session with nothing to report never reserves a turn
/// even when a viewer is attached.
#[test]
fn bash_empty_mailbox_skips_before_ui_gate() {
    let fixture = Fixture::new();
    fixture.attach_viewer();

    assert_eq!(
        bash_tag(&fixture.bash_decision()),
        Tag::SkippedEmptyMailbox,
        "no pending exit ⇒ SkippedEmptyMailbox, regardless of subscribers"
    );
    assert!(
        !fixture.sessions.is_session_busy_blocking(&fixture.sid),
        "a skip must never leave a reservation behind"
    );
}

/// Guards the subagent idle auto-turn contract, which today has *no* behavioural
/// coverage beyond the deny-sink check: neither a missing viewer nor a busy
/// session may consume or drop a settled child completion.
#[test]
fn subagent_skips_never_drain_completions() {
    let fixture = Fixture::new();
    fixture.seed_subagent_completion();

    assert_eq!(
        subagent_tag(&fixture.subagent_decision()),
        Tag::SkippedNoUi,
        "no subscriber ⇒ SkippedNoUi"
    );
    assert!(
        fixture.subagents.has_pending(&fixture.sid),
        "a no-UI skip must leave the settled child completion for a later wake"
    );
    assert!(!fixture.sessions.is_session_busy_blocking(&fixture.sid));

    fixture.attach_viewer();
    fixture.reserve_foreign_turn();
    assert_eq!(
        subagent_tag(&fixture.subagent_decision()),
        Tag::SkippedBusy,
        "a busy session ⇒ SkippedBusy"
    );
    assert!(
        fixture.subagents.has_pending(&fixture.sid),
        "a busy skip must leave the completion queued"
    );
    fixture.release_foreign_turn();
}

/// Mirror of the bash reservation test for the subagent listener. Written
/// separately on purpose: after a 2-in-1 dedup this assertion must still hold
/// for *both* mailboxes, so it may not be expressed once for the merged helper.
#[test]
fn subagent_prepared_holds_one_reservation_and_keeps_completions_pending() {
    let fixture = Fixture::new();
    fixture.attach_viewer();
    fixture.seed_subagent_completion();

    let turn_id = match fixture.subagent_decision() {
        SubagentDecision::Prepared { turn_id, .. } => turn_id,
        other => panic!(
            "expected Prepared on an idle, watched session with a settled child, got {:?}",
            subagent_tag(&other)
        ),
    };
    assert!(fixture.sessions.is_session_busy_blocking(&fixture.sid));
    assert!(
        fixture.subagents.has_pending(&fixture.sid),
        "the decision must not take completions; the seam writes `reminder/subagent_settled`"
    );
    assert_eq!(
        subagent_tag(&fixture.subagent_decision()),
        Tag::SkippedBusy,
        "a second decision must not prepare a second turn"
    );
    assert!(fixture.subagents.has_pending(&fixture.sid));
    assert!(
        fixture.sessions.release_turn_reservation(&fixture.sid, &turn_id),
        "the Prepared turn id must own the reservation"
    );
}

/// The direct refactor guard for merging `bash_auto_turn` and
/// `subagent_auto_turn`: for the same session state the two listeners must keep
/// producing the *same* decision shape, and the same precedence
/// (mailbox → UI gate → busy).
#[test]
fn bash_and_subagent_decisions_stay_symmetric() {
    // 1. Nothing pending: both answer the mailbox question first.
    let idle = Fixture::new();
    idle.attach_viewer();
    assert_eq!(bash_tag(&idle.bash_decision()), Tag::SkippedEmptyMailbox);
    assert_eq!(
        subagent_tag(&idle.subagent_decision()),
        Tag::SkippedEmptyMailbox
    );

    // 2. Pending, no viewer: both refuse without touching the mailboxes.
    let unwatched = Fixture::new();
    unwatched.seed_bash_exit();
    unwatched.seed_subagent_completion();
    let bash = bash_tag(&unwatched.bash_decision());
    let subagent = subagent_tag(&unwatched.subagent_decision());
    assert_eq!(bash, Tag::SkippedNoUi);
    assert_eq!(bash, subagent, "no-UI decision must be symmetric");
    assert!(unwatched.terminal.jobs.mailbox_pending(&unwatched.sid));
    assert!(unwatched.subagents.has_pending(&unwatched.sid));

    // 3. Pending, watched, busy: both refuse and leave their mailbox alone.
    let busy = Fixture::new();
    busy.attach_viewer();
    busy.reserve_foreign_turn();
    busy.seed_bash_exit();
    busy.seed_subagent_completion();
    let bash = bash_tag(&busy.bash_decision());
    let subagent = subagent_tag(&busy.subagent_decision());
    assert_eq!(bash, Tag::SkippedBusy);
    assert_eq!(bash, subagent, "busy decision must be symmetric");
    assert!(busy.terminal.jobs.mailbox_pending(&busy.sid));
    assert!(busy.subagents.has_pending(&busy.sid));
    busy.release_foreign_turn();

    // 4. Pending, watched, idle: both prepare exactly one turn each.
    let idle = Fixture::new();
    idle.attach_viewer();
    idle.seed_bash_exit();
    let bash_turn = match idle.bash_decision() {
        BashDecision::Prepared { turn_id, .. } => turn_id,
        other => panic!("expected Prepared, got {:?}", bash_tag(&other)),
    };
    assert!(
        idle.sessions
            .release_turn_reservation(&idle.sid, &bash_turn),
        "bash Prepared must be releasable"
    );
    idle.seed_subagent_completion();
    let child_turn = match idle.subagent_decision() {
        SubagentDecision::Prepared { turn_id, .. } => turn_id,
        other => panic!("expected Prepared, got {:?}", subagent_tag(&other)),
    };
    assert!(
        idle.sessions
            .release_turn_reservation(&idle.sid, &child_turn),
        "subagent Prepared must be releasable"
    );
}

/// Guards `context-compact.md`'s sealed-only trigger on the pure policy seam
/// that `AgentRuntime::compact_if_needed` calls: rows that have not crossed the
/// write gate can never make the ordinary/auto trigger eligible, however small
/// the recent-retention window is.
///
/// The positive half keeps this from being vacuously green: the identical rows
/// are compactable once they are sealed, so the negative cases fail because of
/// sealing, not because nothing was compactable.
#[test]
fn compact_eligibility_ignores_unsealed_rows() {
    let items = [
        common::fake_deps::assistant_text_item("alpha alpha alpha alpha", "v2-a"),
        common::fake_deps::assistant_text_item("beta beta beta beta", "v2-b"),
        common::fake_deps::assistant_text_item("gamma gamma gamma gamma", "v2-c"),
    ];
    let sealed: Vec<WorkingRow> = items
        .iter()
        .enumerate()
        .map(|(index, item)| WorkingRow::persisted(index as Seq + 1, item.clone()))
        .collect();
    let sealed_items = project_items(&sealed);
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);

    assert!(
        CompactPolicy::can_compact(&budget, &sealed_items),
        "precondition: this transcript has a keep-recent cut"
    );
    assert!(
        CompactPolicy::can_compact_sealed(&sealed, &HashSet::new(), &budget),
        "sealed rows that can be cut must be eligible"
    );

    // Same rows, nothing sealed yet: every row is still in flight.
    let in_progress: HashSet<Seq> = sealed.iter().filter_map(|row| row.log_seq).collect();
    assert_eq!(in_progress.len(), sealed.len());
    assert!(
        !CompactPolicy::can_compact_sealed(&sealed, &in_progress, &budget),
        "rows that have not crossed the write gate must never trigger compaction"
    );

    // Same rows, written but never sealed (pending): also ineligible.
    let pending: Vec<WorkingRow> = items
        .iter()
        .map(|item| WorkingRow::pending(item.clone()))
        .collect();
    assert!(
        !CompactPolicy::can_compact_sealed(&pending, &HashSet::new(), &budget),
        "an unsealed tail must not look compactable"
    );
    assert!(
        !CompactPolicy::can_compact_sealed(&[], &HashSet::new(), &BudgetPolicy::new(1)),
        "an empty working set is never eligible, even under absurd pressure"
    );
}

/// Keeps the two write-path facts the runtime relies on from drifting apart:
/// an empty session reports no sealed rows, and a sealed prefix needs a log
/// cursor (`WorkingRow::pending` carries `log_seq: None` by construction).
#[test]
fn working_row_sealing_is_explicit() {
    let item = common::fake_deps::assistant_text_item("hello", "v2-seal");
    assert_eq!(
        WorkingRow::pending(item.clone()).log_seq,
        None,
        "a pending row must not claim a log position"
    );
    assert_eq!(
        WorkingRow::persisted(7, item).log_seq,
        Some(7),
        "a sealed row must carry its real log position"
    );
}

/// Guards the critical asymmetry in `AgentRuntime::compact_if_needed` BEFORE
/// deduplicating its 4 settlement arms:
///
/// * A `Manual` compaction episode failing (even with `StopTurn`) must NEVER
///   abort a running turn — the turn waits past the failure and completes.
/// * An `Auto` compaction episode failing with `StopTurn` MUST abort the turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_stop_turn_failure_never_aborts_running_turn() {
    use common::runtime::{build_runtime_with_provider_and_observer, test_agent};
    use common::scripted_provider::ScriptedProvider;
    use litecode::runtime::observer::{
        ChannelObserver, CompactionFailKind, CompactionTrigger, InternalEvent,
    };
    use litecode::session::manager::{CompactionFailureAction, EpisodeOutcome};

    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(ScriptedProvider::with_text("turn survived manual failure"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = build_runtime_with_provider_and_observer(
        dir.path(),
        test_agent(vec![], "allow", 4),
        provider,
        ChannelObserver::new(tx),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = sessions
        .data()
        .list_sessions_blocking()
        .expect("list sessions")
        .into_iter()
        .next()
        .expect("seeded session")
        .id;

    // Put the session past the 128k wait line (~113.4k) but below the 119.8k hard line,
    // so `compact_if_needed` enters the active-episode wait branch.
    runtime.set_last_prompt_tokens(118_000);

    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished_bg = Arc::clone(&finished);
    let mut lease = sessions
        .try_begin_compaction_episode(&sid, CompactionTrigger::Manual)
        .expect("claim manual compaction lease");
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            if matches!(env.event, InternalEvent::StepStarted { .. }) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                break;
            }
        }
        lease.finish(EpisodeOutcome::Failed {
            kind: CompactionFailKind::SummaryFinish,
            message: "manual summary failed".into(),
            action: CompactionFailureAction::StopTurn,
        });
        finished_bg.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        runtime.run("keep going"),
    )
    .await
    .expect("manual compaction wait test must not hang");
    assert!(
        finished.load(std::sync::atomic::Ordering::SeqCst),
        "precondition: turn must have actually waited on the active manual episode (no fake-green!)"
    );
    match result {
        Ok(final_text) => {
            assert!(
                final_text.contains("turn survived manual failure"),
                "expected normal completion text, got: {final_text:?}"
            );
        }
        Err(err) => panic!(
            "manual compaction failure must NEVER abort a running turn, got: {err:?}"
        ),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_stop_turn_failure_aborts_running_turn() {
    use common::runtime::{build_runtime_with_provider_and_observer, test_agent};
    use common::scripted_provider::ScriptedProvider;
    use litecode::runtime::observer::{
        ChannelObserver, CompactionFailKind, CompactionTrigger, InternalEvent,
    };
    use litecode::session::manager::{CompactionFailureAction, EpisodeOutcome};

    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(ScriptedProvider::with_text("must not be reached"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = build_runtime_with_provider_and_observer(
        dir.path(),
        test_agent(vec![], "allow", 4),
        provider,
        ChannelObserver::new(tx),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = sessions
        .data()
        .list_sessions_blocking()
        .expect("list sessions")
        .into_iter()
        .next()
        .expect("seeded session")
        .id;

    runtime.set_last_prompt_tokens(118_000);

    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished_bg = Arc::clone(&finished);
    let mut lease = sessions
        .try_begin_compaction_episode(&sid, CompactionTrigger::Auto)
        .expect("claim auto compaction lease");
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            if matches!(env.event, InternalEvent::StepStarted { .. }) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                break;
            }
        }
        lease.finish(EpisodeOutcome::Failed {
            kind: CompactionFailKind::SummaryFinish,
            message: "auto summary failed".into(),
            action: CompactionFailureAction::StopTurn,
        });
        finished_bg.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        runtime.run("should stop"),
    )
    .await
    .expect("auto compaction wait test must not hang");
    assert!(
        finished.load(std::sync::atomic::Ordering::SeqCst),
        "precondition: turn must have waited on the active auto episode"
    );
    assert!(
        result.is_err(),
        "auto compaction StopTurn failure MUST abort the running turn, got: {result:?}"
    );
}
