//! CLI entry point — transitional escape hatch.
//! The CLI is NOT the target interaction path; the Electron client is.
//! This module exists for development convenience and may be removed in a future version.

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use clap::{Parser, Subcommand};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::client_protocol::connection::{ConnectionHandle, SessionRequest};
use crate::client_protocol::controller::SessionController;
use crate::client_protocol::project::{IncomingWire, classify_incoming};
use crate::client_protocol::protocol::{
    JsonRpcRequestEnvelope, TurnEndReason, TurnTokenStats, WireEvent,
};
use crate::config::{ConfigManager, ResolvedConfig, SettingsWriter, cli_turn_guard};
use crate::runtime::RuntimeHandle;
use crate::types::LitecodeError;

#[derive(Parser)]
#[command(name = "litecode", about = "A lightweight terminal coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, global = true)]
    resume: Option<String>,

    #[arg(long, global = true)]
    list_sessions: bool,

    /// Workspace root (canonical); defaults to absolute cwd.
    #[arg(long, global = true)]
    workspace: Option<String>,

    #[arg(long, global = true, default_value = "default")]
    agent: String,

    /// Approve every permission gate this run without asking.
    ///
    /// A run whose stdin is not a terminal refuses instead of prompting; this
    /// flag is how a headless run (script, container, evaluator) may act.
    /// Off by default: an interactive run still asks.
    #[arg(long, short = 'y', global = true)]
    yes: bool,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    prompt: Vec<String>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(hide = true, name = "code-search-worker")]
    CodeSearchWorker,
    #[command(name = "list-sessions")]
    ListSessions,
    #[command(name = "resume")]
    Resume { id: String },
    /// Serve the litecode backend.
    ///
    /// Escape-hatch for running without the Electron client.
    /// Not the target path; may be removed in a future version.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7483")]
        bind: String,
        /// Reject non-loopback bind addresses (local Electron / debug).
        /// Independent of --require-auth.
        #[arg(long, env = "LITECODE_LOOPBACK_ONLY")]
        loopback_only: bool,
        /// Require non-empty LITECODE_TOKEN. Does not imply loopback-only;
        /// non-loopback binds must set this flag.
        #[arg(long, env = "LITECODE_REQUIRE_AUTH")]
        require_auth: bool,
        /// Exit when this parent PID disappears (desktop host)
        #[arg(long)]
        parent_pid: Option<u32>,
        /// Exit when stdin reaches EOF (host keepalive pipe). Off by default.
        #[arg(long, env = "LITECODE_SHUTDOWN_ON_STDIN_EOF")]
        shutdown_on_stdin_eof: bool,
    },
    /// Bridge a running app's own MCP endpoint to stdio.
    ///
    /// The app picks a new port on every start, so a client definition that
    /// names `127.0.0.1:<port>` goes stale. This command resolves the live
    /// port per request, which keeps one client definition valid across
    /// restarts. Nothing runs it unless a client definition asks for it.
    #[command(name = "app-mcp")]
    AppMcp {
        /// App id from the Apps catalog, e.g. `nocodb`.
        app_id: String,
        /// Path appended to the app base URL, e.g. `/mcp/<endpoint>`.
        #[arg(long)]
        path: String,
        /// Extra HTTP header as 'name: value'. Repeatable.
        #[arg(long = "header")]
        headers: Vec<String>,
        /// Per-request timeout in seconds.
        #[arg(long, default_value_t = 60)]
        timeout: u64,
    },
    /// Configuration management
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a global settings key (writes to litecode.db)
    Set { key: String, value: String },
}

fn workspace_override(cli: &Cli) -> Option<&Path> {
    cli.workspace.as_deref().map(Path::new)
}

fn load_runtime_bundle_from_cli(cli: &Cli) -> anyhow::Result<ResolvedConfig> {
    // WP-166: serve warms the snapshot repo in the background after bind.
    let warm = if matches!(cli.command, Some(Commands::Serve { .. })) {
        crate::config::workspace::WarmSnapshot::Deferred
    } else {
        crate::config::workspace::WarmSnapshot::Now
    };
    ConfigManager::load_runtime_bundle_with(workspace_override(cli), warm).map_err(Into::into)
}

/// Key of the provider the default agent's model belongs to, else the first
/// configured provider.
fn get_api_key(resolved: &ResolvedConfig) -> anyhow::Result<String> {
    if let Some(model) = resolved.declared_model_for_agent("default")
        && let Some(key) = resolved.provider_api_key(&model.provider_id)
    {
        return Ok(key.to_string());
    }
    resolved
        .configured_providers()
        .first()
        .and_then(|provider| resolved.provider_api_key(&provider.id))
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!("no provider API key configured; add one in Settings → Providers")
        })
}

fn setup_logging(
    logs_dir: &std::path::Path,
) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(logs_dir)?;

    let log_path = logs_dir.join("litecode.log");
    let file_appender = std::fs::File::create(&log_path)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let level = crate::config::log_filter::resolve_level_from_db();
    let (env_filter, reload_handle) = crate::config::log_filter::new_reload_layer(&level);
    crate::config::log_filter::install_handle(reload_handle);

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_filter(crate::config::log_filter::file_filter());

    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_level(true)
        .with_writer(std::io::stderr);

    let broadcast_layer = crate::telemetry::log_broadcast_layer();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(broadcast_layer)
        .with(file_layer)
        .init();

    tracing::info!("log dir: {}", logs_dir.display());
    tracing::info!("log file: {}", log_path.display());
    Ok(guard)
}

/// How this run answers the human gates on the permission path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnswerPolicy {
    /// Read the terminal. Only chosen when stdin is one.
    Prompt,
    /// `--yes`: approve every gate, never read stdin.
    Approve,
    /// No terminal and no `--yes`: refuse every gate, never read stdin.
    Refuse,
}

/// The whole decision: a terminal asks, `--yes` approves, anything else refuses.
///
/// A run with neither cannot answer, so it must not block on a read that will
/// never complete — and it must not pretend a human said yes either.
fn answer_policy(yes: bool, stdin_is_terminal: bool) -> AnswerPolicy {
    if yes {
        AnswerPolicy::Approve
    } else if stdin_is_terminal {
        AnswerPolicy::Prompt
    } else {
        AnswerPolicy::Refuse
    }
}

/// One answer to a human gate, in the shape `agent/permission` expects.
#[derive(Debug, Default, PartialEq)]
struct GateAnswer {
    approved: bool,
    always: bool,
    free_text: Option<String>,
    answers: std::collections::HashMap<String, crate::permission::AskAnswer>,
}

/// What a run answers when nobody is asked.
///
/// `Permission` and `Approval` guard a tool, so `--yes` allows them. `ask_user`
/// is a question, not a grant: no policy can answer it, so it is declined
/// rather than guessed at.
fn auto_answer(policy: AnswerPolicy, kind: crate::permission::AskKind) -> GateAnswer {
    match (policy, kind) {
        (
            AnswerPolicy::Approve,
            crate::permission::AskKind::Permission | crate::permission::AskKind::Approval,
        ) => GateAnswer {
            approved: true,
            ..GateAnswer::default()
        },
        _ => GateAnswer::default(),
    }
}

/// Why a gate was decided without a human, for the cases that must be said out
/// loud. Nothing is returned for a plain `--yes` allow: the flag itself is the
/// statement, and a tool-loop turn would repeat the line per call.
fn auto_refusal(
    policy: AnswerPolicy,
    kind: crate::permission::AskKind,
    tool: &str,
) -> Option<String> {
    match (policy, kind) {
        (AnswerPolicy::Approve, crate::permission::AskKind::AskUser) => Some(format!(
            "[litecode] declined {tool}: a question needs a human, and --yes cannot answer one"
        )),
        (AnswerPolicy::Refuse, _) => Some(format!(
            "[litecode] denied {tool}: stdin is not a terminal and --yes was not given"
        )),
        _ => None,
    }
}

/// Ask the terminal, in the kind's own shape. Only reached with a TTY.
fn answer_from_terminal(
    kind: crate::permission::AskKind,
    tool: &str,
    summary: &str,
    questions: &[crate::permission::AskQuestion],
) -> GateAnswer {
    match kind {
        crate::permission::AskKind::AskUser => {
            let (approved, answers) = ask_cli_ask_user(summary, questions);
            GateAnswer {
                approved,
                answers,
                ..GateAnswer::default()
            }
        }
        crate::permission::AskKind::Approval => {
            let (approved, free_text) = ask_cli_approval(tool, summary);
            GateAnswer {
                approved,
                free_text,
                ..GateAnswer::default()
            }
        }
        crate::permission::AskKind::Permission => {
            let (approved, always) = ask_cli_permission(tool, summary);
            GateAnswer {
                approved,
                always,
                ..GateAnswer::default()
            }
        }
    }
}

fn ask_cli_permission(tool: &str, summary: &str) -> (bool, bool) {
    eprint!(
        "\n⚠ Permission: tool '{}' — {}. Allow? [y/n/always]: ",
        tool, summary
    );
    std::io::stderr().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return (false, false);
    }
    match input.trim().to_lowercase().as_str() {
        "always" => (true, true),
        "y" | "yes" => (true, false),
        _ => (false, false),
    }
}

/// Approve/Reject Ask for plan create (optional free-text opinion).
fn ask_cli_approval(tool: &str, summary: &str) -> (bool, Option<String>) {
    eprint!(
        "\n⚠ Approve {}: {}\n  [a]pprove / [r]eject: ",
        tool, summary
    );
    std::io::stderr().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return (false, None);
    }
    let approved = matches!(
        input.trim().to_lowercase().as_str(),
        "a" | "approve" | "y" | "yes"
    );
    eprint!("  Optional opinion (Enter to skip): ");
    std::io::stderr().flush().ok();
    let mut opinion = String::new();
    if std::io::stdin().read_line(&mut opinion).is_err() {
        return (approved, None);
    }
    let opinion = opinion.trim();
    let free_text = if opinion.is_empty() {
        None
    } else {
        Some(opinion.to_string())
    };
    (approved, free_text)
}

/// Label of the system-appended free-text item (always the last item).
const CLI_OTHER_LABEL: &str = "其他（自己填写）";

/// Parsed CLI pick: option indices (0-based) and whether the trailing
/// 「其他」 item was chosen. `None` = skip / empty / no valid pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliSelection {
    pub options: Vec<usize>,
    pub other: bool,
}

/// Pure parser for `ask_cli_one_question`. Item numbers are 1-based; item
/// `n_options + 1` is 「其他（自己填写）」. Single-select: first valid pick wins
/// (options and 「其他」 are mutually exclusive). Multi-select: they may coexist.
pub(crate) fn parse_cli_selection(
    input: &str,
    n_options: usize,
    multi_select: bool,
) -> Option<CliSelection> {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() || trimmed == "s" || trimmed == "skip" {
        return None;
    }
    let mut sel = CliSelection {
        options: Vec::new(),
        other: false,
    };
    for part in trimmed.split(|c: char| c == ',' || c.is_whitespace()) {
        let Ok(n) = part.parse::<usize>() else {
            continue;
        };
        if n == 0 || n > n_options + 1 {
            continue;
        }
        if n == n_options + 1 {
            sel.other = true;
        } else if !sel.options.contains(&(n - 1)) {
            sel.options.push(n - 1);
        }
        if !multi_select {
            break;
        }
    }
    if sel.options.is_empty() && !sel.other {
        return None;
    }
    Some(sel)
}

/// Build the answer from a parsed selection + free text. 「其他」 never goes
/// into `selected`; choosing it with empty text means not answered.
pub(crate) fn build_cli_answer(
    q: &crate::permission::AskQuestion,
    sel: &CliSelection,
    text: Option<&str>,
) -> Option<crate::permission::AskAnswer> {
    let free_text = text
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    if sel.other && free_text.is_none() {
        return None;
    }
    let selected: Vec<String> = sel
        .options
        .iter()
        .filter_map(|i| q.options.get(*i).map(|o| o.id.clone()))
        .collect();
    if selected.is_empty() && free_text.is_none() {
        return None;
    }
    Some(crate::permission::AskAnswer {
        selected,
        free_text,
    })
}

fn ask_cli_one_question(
    q: &crate::permission::AskQuestion,
) -> Option<crate::permission::AskAnswer> {
    // WP-081: 「其他（自己填写）」 is always the last item, regardless of q.free_text.
    eprintln!("\n⚠ [{}] {}", q.id, q.prompt);
    for (i, opt) in q.options.iter().enumerate() {
        eprintln!("  [{}] {} ({})", i + 1, opt.label, opt.id);
    }
    eprintln!("  [{}] {}", q.options.len() + 1, CLI_OTHER_LABEL);
    if q.multi_select {
        eprint!("  Pick item numbers comma-separated, or [s]kip: ");
    } else {
        eprint!("  Pick item number, or [s]kip: ");
    }
    std::io::stderr().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return None;
    }
    let sel = parse_cli_selection(&input, q.options.len(), q.multi_select)?;
    let text = if sel.other {
        eprint!("  {}: ", CLI_OTHER_LABEL);
        std::io::stderr().flush().ok();
        let mut t = String::new();
        if std::io::stdin().read_line(&mut t).is_err() {
            return None;
        }
        Some(t)
    } else {
        None
    };
    build_cli_answer(q, &sel, text.as_deref())
}

#[cfg(test)]
mod wp081_cli_tests {}

fn ask_cli_ask_user(
    summary: &str,
    questions: &[crate::permission::AskQuestion],
) -> (
    bool,
    std::collections::HashMap<String, crate::permission::AskAnswer>,
) {
    if !summary.is_empty() {
        eprintln!("\n⚠ {summary}");
    }
    let mut answers = std::collections::HashMap::new();
    for q in questions {
        match ask_cli_one_question(q) {
            Some(ans) => {
                answers.insert(q.id.clone(), ans);
            }
            None => return (false, std::collections::HashMap::new()),
        }
    }
    if answers.is_empty() {
        (false, answers)
    } else {
        (true, answers)
    }
}

/// Provider usage in the four fields the session meter keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RunTokens {
    prompt: u64,
    completion: u64,
    cache_hit: u64,
    cache_miss: u64,
}

impl RunTokens {
    fn from_stats(stats: &TurnTokenStats) -> Self {
        Self {
            prompt: stats.prompt_tokens,
            completion: stats.completion_tokens,
            cache_hit: stats.cache_hit_tokens,
            cache_miss: stats.cache_miss_tokens,
        }
    }

    /// `self - base`, saturating: a total that appears to go backwards means the
    /// row was reset, not that this run spent negative tokens.
    fn since(self, base: Self) -> Self {
        Self {
            prompt: self.prompt.saturating_sub(base.prompt),
            completion: self.completion.saturating_sub(base.completion),
            cache_hit: self.cache_hit.saturating_sub(base.cache_hit),
            cache_miss: self.cache_miss.saturating_sub(base.cache_miss),
        }
    }
}

/// One turn as the run sees it: what it said, how it ended, and what it cost.
struct TurnRun {
    final_text: String,
    reason: TurnEndReason,
    error: Option<String>,
    /// Usage of the turn's last request (the number the UI shows on a turn).
    last_request: RunTokens,
    /// Session meter at turn end, when the frame carries one.
    session_total: Option<RunTokens>,
    /// This run's share of the meter — set by `CliConversation::run_turn`.
    tokens: RunTokens,
    elapsed: std::time::Duration,
}

impl TurnRun {
    /// Exit status of a headless run: only a turn that reached its own end is a
    /// success. Cancelled, out of steps, or failed all are not.
    fn require_completed(&self) -> anyhow::Result<()> {
        match self.reason {
            TurnEndReason::Completed => Ok(()),
            TurnEndReason::Cancelled => anyhow::bail!("turn cancelled"),
            TurnEndReason::MaxSteps => anyhow::bail!("turn stopped at the step limit"),
            TurnEndReason::Error => anyhow::bail!(
                "turn failed: {}",
                self.error.as_deref().unwrap_or("no error message")
            ),
        }
    }
}

/// The one line a headless run leaves behind, so its cost is readable without
/// opening the UI. stderr keeps stdout the answer.
fn print_run_report(run: &TurnRun) {
    let tokens = run.tokens;
    eprintln!(
        "[litecode] {} in {:.1}s — tokens prompt={} completion={} cache_hit={} cache_miss={}",
        run.reason.as_log_reason(),
        run.elapsed.as_secs_f64(),
        tokens.prompt,
        tokens.completion,
        tokens.cache_hit,
        tokens.cache_miss,
    );
}

/// Run one agent turn with streaming output to stdout via the wire connection loop.
fn run_turn_streaming(
    rt: &tokio::runtime::Runtime,
    conn: &mut ConnectionHandle,
    prompt: &str,
    policy: AnswerPolicy,
) -> anyhow::Result<TurnRun> {
    conn.request_tx
        .send(SessionRequest::JsonRpc(JsonRpcRequestEnvelope {
            jsonrpc: "2.0".into(),
            id: serde_json::Value::String("start".into()),
            method: "agent/run".into(),
            params: serde_json::json!({
                "input": prompt,
            }),
        }))
        .map_err(|_| anyhow::anyhow!("connection closed"))?;

    let mut text_cursor = TextCursor::default();
    rt.block_on(async {
        loop {
            let Some(msg) = conn.next_envelope().await else {
                anyhow::bail!("connection closed");
            };
            // Check if it's a JSON-RPC response (has "id" and "result"/"error")
            if msg.get("id").is_some()
                && (msg.get("result").is_some() || msg.get("error").is_some())
            {
                // Response to our request - check for errors
                if let Some(err) = msg.get("error") {
                    let err_msg = err
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    anyhow::bail!("{}", err_msg);
                }
                // Success response, continue reading notifications
                continue;
            }
            // Otherwise treat as notification
            match classify_incoming(&msg) {
                IncomingWire::TurnEvent(ev) => print_wire_event(&ev),
                IncomingWire::AssistantText { seq, text } => text_cursor.absorb(seq, &text),
                IncomingWire::PermissionRequest {
                    request_id,
                    tool,
                    rule_id: _,
                    summary,
                    kind,
                    free_text: wants_free_text,
                    options,
                    multi_select,
                    questions,
                    ..
                } => {
                    let questions = if !questions.is_empty() {
                        questions
                    } else if kind == crate::permission::AskKind::AskUser && !options.is_empty() {
                        vec![crate::permission::AskQuestion {
                            id: "q0".into(),
                            prompt: summary.clone(),
                            options,
                            multi_select,
                            free_text: wants_free_text,
                        }]
                    } else {
                        questions
                    };
                    let answer = match policy {
                        AnswerPolicy::Prompt => {
                            answer_from_terminal(kind, &tool, &summary, &questions)
                        }
                        policy => {
                            if let Some(why) = auto_refusal(policy, kind, &tool) {
                                eprintln!("{why}");
                            }
                            auto_answer(policy, kind)
                        }
                    };
                    conn.request_tx
                        .send(SessionRequest::PermissionGrant {
                            request_id,
                            tool,
                            approved: answer.approved,
                            always: answer.always,
                            free_text: answer.free_text,
                            selected: Vec::new(),
                            answers: answer.answers,
                        })
                        .map_err(|_| anyhow::anyhow!("connection closed"))?;
                }
                IncomingWire::TurnFinished {
                    reason,
                    final_text,
                    error,
                    turn_token_stats,
                    cumulative_token_stats,
                    ..
                } => {
                    return Ok(TurnRun {
                        final_text: final_text.unwrap_or_default(),
                        reason,
                        error: error.map(|e| e.message),
                        last_request: turn_token_stats
                            .as_ref()
                            .map(RunTokens::from_stats)
                            .unwrap_or_default(),
                        session_total: cumulative_token_stats.as_ref().map(RunTokens::from_stats),
                        tokens: RunTokens::default(),
                        elapsed: std::time::Duration::ZERO,
                    });
                }
                IncomingWire::OperationFailed(msg) => anyhow::bail!("{}", msg),
                IncomingWire::Ignored => {}
            }
        }
    })
}

/// Print the answer as it lands in the log.
///
/// One row holds the whole answer and grows in place, so the cursor prints a
/// `seq`'s text once and afterwards only what was appended to it — the log is the
/// only body, which is why a resumed or reconnected run prints the same text a
/// reload would show rather than re-printing or losing a chunk.
#[derive(Default)]
struct TextCursor {
    seq: Option<u64>,
    printed: usize,
}

impl TextCursor {
    fn absorb(&mut self, seq: u64, text: &str) {
        if self.seq != Some(seq) {
            self.seq = Some(seq);
            self.printed = 0;
        }
        if text.len() <= self.printed {
            return;
        }
        let Some(suffix) = text.get(self.printed..) else {
            // The row was rewritten rather than appended to; print it whole.
            self.printed = 0;
            print!("{text}");
            std::io::stdout().flush().ok();
            self.printed = text.len();
            return;
        };
        print!("{suffix}");
        std::io::stdout().flush().ok();
        self.printed = text.len();
    }
}

/// Print control-plane wire events to stdout.
fn print_wire_event(event: &WireEvent) {
    match event {
        WireEvent::Error { message, .. } => {
            tracing::error!(error = %message, "agent error");
            eprintln!("\n[error: {}]", message);
        }
        WireEvent::TodoProgress {
            pending,
            in_progress,
            completed,
            items: _,
        } => {
            tracing::debug!(pending, in_progress, completed, "todo progress");
            eprintln!("[todos: ○{} ◐{} ●{}]", pending, in_progress, completed);
        }
        _ => {}
    }
}

struct CliConversation {
    rt: tokio::runtime::Runtime,
    conn: ConnectionHandle,
    _lease: crate::session::WorkspaceWriteLease,
    /// Session meter when the session opened, so a turn's tokens are its own.
    baseline: RunTokens,
}

impl CliConversation {
    fn open(
        runtime: RuntimeHandle,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        let turn_guard = cli_turn_guard();
        let db_path = runtime.db_path();
        let litecode_dir = Path::new(&db_path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let lease = crate::session::WorkspaceWriteLease::acquire(&litecode_dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let data = crate::session::SessionData::open(&lease, Path::new(&db_path))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let sessions = std::sync::Arc::new(crate::session::SessionManager::from_data(
            turn_guard.clone(),
            data,
        ));
        runtime
            .workspace_engines
            .set_session_reader(sessions.reader());
        let service = std::sync::Arc::new(crate::session::SessionService::new(sessions));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()?;
        let session_id = rt.block_on(service.open_session(project, agent_id, model_id))?;
        // The run's tokens are the session meter's move across it. A run is a
        // fresh session today, but the delta is what makes the number "this
        // run" rather than "this session", and it reuses the same rows the UI
        // reads instead of adding a counter.
        let baseline = service
            .meter(&session_id)
            .map(|meter| RunTokens {
                prompt: meter.cum_prompt_tokens,
                completion: meter.cum_completion_tokens,
                cache_hit: meter.cum_cache_hit_tokens,
                cache_miss: meter.cum_cache_miss_tokens,
            })
            .unwrap_or_default();
        let mut controller = SessionController::with_service(runtime, service)?;
        let conn = rt.block_on(async {
            controller.subscribe(&session_id).await;
            ConnectionHandle::spawn(controller)
        });
        Ok(Self {
            rt,
            conn,
            _lease: lease,
            baseline,
        })
    }

    fn run_turn(&mut self, prompt: &str, policy: AnswerPolicy) -> anyhow::Result<TurnRun> {
        let started = std::time::Instant::now();
        let mut run = run_turn_streaming(&self.rt, &mut self.conn, prompt, policy)?;
        run.elapsed = started.elapsed();
        run.tokens = match run.session_total {
            Some(total) => {
                let metered = total.since(self.baseline);
                self.baseline = total;
                metered
            }
            // No meter on the frame (older host): the last request is the only
            // usage there is.
            None => run.last_request,
        };
        Ok(run)
    }
}

/// Run an interactive REPL loop via the wire connection.
fn run_interactive(session: &mut CliConversation, policy: AnswerPolicy) -> anyhow::Result<()> {
    println!("\n--- interactive mode (type /help for commands) ---");

    loop {
        print!("litecode> ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        // 0 bytes is EOF, not an error: a terminal that closes must end the
        // loop instead of re-prompting forever on a read that never fills.
        match std::io::stdin().read_line(&mut input) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        match input {
            "/exit" | "/quit" => break,
            "/help" => {
                println!("commands:");
                println!("  /exit, /quit   — quit");
                println!("  anything else  — send to the agent");
                continue;
            }
            _ if input.starts_with('/') => {
                println!("unknown command: {}. available: /exit, /help", input);
                continue;
            }
            _ => {}
        }

        let run = session.run_turn(input, policy)?;

        if !run.final_text.is_empty() {
            println!();
        }
    }

    Ok(())
}

fn resolve_session_id(cli: &Cli) -> Option<String> {
    cli.resume.clone().or_else(|| match &cli.command {
        Some(Commands::Resume { id }) => Some(id.clone()),
        _ => None,
    })
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Some(Commands::CodeSearchWorker)) {
        return crate::cli::code_search_worker::run();
    }

    if let Some(Commands::AppMcp {
        app_id,
        path,
        headers,
        timeout,
    }) = &cli.command
    {
        let workspace = cli.workspace.as_deref().map(Path::new);
        return crate::apps::mcp_bridge::run(app_id, path, headers, *timeout, workspace);
    }

    if let Some(Commands::Config { command }) = &cli.command {
        match command {
            ConfigCommands::Set { key, value } => {
                let writer = SettingsWriter::new(cli_turn_guard());
                match writer.set_key(key, value) {
                    Ok((revision, restart_required)) => {
                        if restart_required {
                            println!(
                                "settings saved (revision {revision}); restart_required: true"
                            );
                        } else {
                            println!("settings saved (revision {revision})");
                        }
                    }
                    Err(LitecodeError::Config(msg)) if msg == "turn_in_progress" => {
                        eprintln!("error: turn_in_progress");
                        std::process::exit(1);
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        return Ok(());
    }

    let resolved = load_runtime_bundle_from_cli(&cli)?;
    let workspace = resolved.workspace().clone();
    let db_path = workspace.paths.sessions_db.clone();

    let _log_guard = setup_logging(&workspace.paths.logs_dir)?;
    // WP-188 observe-only: process panic → diagnostics/_process or session crash-*.log
    let panic_fallback = workspace
        .paths
        .logs_dir
        .parent()
        .unwrap_or(workspace.paths.logs_dir.as_path())
        .to_path_buf();
    crate::runtime::diagnostics::install_panic_hook(panic_fallback);
    tracing::info!(
        workspace = %workspace.workspace_root.display(),
        session_db = %db_path.display(),
        "workspace ready"
    );

    if let Some(Commands::Serve {
        bind,
        loopback_only,
        require_auth,
        parent_pid,
        shutdown_on_stdin_eof,
    }) = &cli.command
    {
        // Provider resolution is deferred to runtime — the web server must
        // start regardless of provider configuration so the Settings UI is
        // available.  A missing / invalid provider will surface as a clear
        // error at chat time and self-heal once the user configures it.
        let session_id = resolve_session_id(&cli);
        crate::serve::run(
            resolved,
            cli.agent.clone(),
            workspace,
            session_id,
            crate::serve::ServeOptions {
                bind: bind.clone(),
                loopback_only: *loopback_only,
                require_auth: *require_auth,
                parent_pid: *parent_pid,
                shutdown_on_stdin_eof: *shutdown_on_stdin_eof,
            },
        )?;
        return Ok(());
    }

    if cli.list_sessions || matches!(cli.command, Some(Commands::ListSessions)) {
        let data_root = db_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("sessions.db has no parent directory"))?;
        let lease = crate::session::WorkspaceWriteLease::acquire(data_root)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let data = crate::session::SessionData::open(&lease, &db_path)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let sessions = data.reader().list_sessions_blocking()?;
        for row in sessions {
            let preview_str = if row.preview.is_empty() {
                ""
            } else {
                &format!("  |  {}", row.preview)
            };
            println!(
                "{}  {}  {}{}",
                row.id, row.project, row.updated_at, preview_str
            );
        }
        data.shutdown();
        return Ok(());
    }

    // Only the agent-run path needs something to read. Subcommands with their
    // own stdin contract (`serve`, `list-sessions`, ...) returned above.
    let stdin_is_terminal = std::io::stdin().is_terminal();
    if cli.prompt.is_empty() && !stdin_is_terminal {
        anyhow::bail!("nothing to run: pass a prompt, or use a terminal for the REPL");
    }

    let api_key = get_api_key(&resolved)?;
    let _ = api_key;
    let project = workspace.workspace_root.to_string_lossy().to_string();
    let settings_revision = Arc::new(AtomicU64::new(0));
    let engine_manager = Arc::new(crate::optional::EngineManager::new());
    let workspace_engines = Arc::new(crate::engines::WorkspaceEngines::new());
    engine_manager.reconcile(&resolved);
    workspace_engines.reconcile(&resolved);
    let ide = crate::ide_base::IdeBaseHandle::open(
        workspace.workspace_root.clone(),
        Arc::clone(&workspace_engines),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let runtime = RuntimeHandle::new(
        resolved,
        cli.agent.clone(),
        workspace,
        engine_manager,
        workspace_engines,
        ide,
        settings_revision,
        crate::config::global_db::default_db_path(),
    );

    let agent_id = runtime.desired_primary_agent().to_string();
    let model_id = runtime
        .resolved
        .agents()
        .get(&agent_id)
        .map(|p| p.model_ref.clone())
        .filter(|s| !s.is_empty());
    let mut cli_session = CliConversation::open(runtime, &project, &agent_id, model_id.as_deref())?;

    let policy = answer_policy(cli.yes, stdin_is_terminal);
    if policy == AnswerPolicy::Approve {
        eprintln!("[litecode] --yes: approving every permission gate for this run");
    }

    if !cli.prompt.is_empty() {
        let prompt = cli.prompt.join(" ");
        let run = cli_session.run_turn(&prompt, policy)?;

        if !run.final_text.is_empty() {
            println!();
        }

        // Without a terminal the one prompt is the whole process: report what
        // it cost and leave with its status, instead of opening a REPL that has
        // nothing to read.
        if !stdin_is_terminal {
            print_run_report(&run);
            return run.require_completed();
        }
    }

    run_interactive(&mut cli_session, policy)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_mcp_takes_the_shape_a_client_definition_writes() {
        let cli = Cli::try_parse_from([
            "litecode",
            "app-mcp",
            "nocodb",
            "--path",
            "/mcp/abc",
            "--header",
            "xc-mcp-token: secret",
            "--timeout",
            "30",
        ])
        .expect("parse");
        match cli.command {
            Some(Commands::AppMcp {
                app_id,
                path,
                headers,
                timeout,
            }) => {
                assert_eq!(app_id, "nocodb");
                assert_eq!(path, "/mcp/abc");
                assert_eq!(headers, vec!["xc-mcp-token: secret".to_string()]);
                assert_eq!(timeout, 30);
            }
            _ => panic!("expected app-mcp"),
        }
    }

    use crate::permission::AskKind;

    #[test]
    fn answer_policy_asks_only_where_a_human_can_answer() {
        assert_eq!(answer_policy(false, true), AnswerPolicy::Prompt);
        assert_eq!(answer_policy(true, true), AnswerPolicy::Approve);
        assert_eq!(answer_policy(false, false), AnswerPolicy::Refuse);
        // Nothing on stdin to wait for, so --yes is the only way to run.
        assert_eq!(answer_policy(true, false), AnswerPolicy::Approve);
    }

    #[test]
    fn yes_allows_the_gates_that_guard_a_tool() {
        for kind in [AskKind::Permission, AskKind::Approval] {
            let answer = auto_answer(AnswerPolicy::Approve, kind);
            assert!(
                answer.approved,
                "{kind:?} guards a tool and --yes allows it"
            );
            // A headless run records no durable grant: it approves this call.
            assert!(!answer.always, "{kind:?} must not write a session grant");
            assert!(answer.free_text.is_none());
            assert!(answer.answers.is_empty());
        }
        let refused = auto_answer(AnswerPolicy::Refuse, AskKind::Permission);
        assert!(!refused.approved, "no terminal and no --yes is a deny");
        assert!(!refused.always);
    }

    #[test]
    fn ask_user_is_never_answered_without_a_human() {
        for policy in [AnswerPolicy::Approve, AnswerPolicy::Refuse] {
            let answer = auto_answer(policy, AskKind::AskUser);
            assert!(!answer.approved, "a question is not a grant");
            assert!(answer.answers.is_empty(), "no answer may be fabricated");
        }
    }

    #[test]
    fn a_denial_always_says_why_an_allow_does_not_need_to() {
        let denied = auto_refusal(AnswerPolicy::Refuse, AskKind::Permission, "bash")
            .expect("a headless denial must be announced");
        assert!(denied.contains("--yes"), "{denied}");
        let unanswerable = auto_refusal(AnswerPolicy::Approve, AskKind::AskUser, "ask_user")
            .expect("declining a question must be announced");
        assert!(unanswerable.contains("needs a human"), "{unanswerable}");
        assert!(auto_refusal(AnswerPolicy::Approve, AskKind::Permission, "bash").is_none());
        assert!(auto_refusal(AnswerPolicy::Prompt, AskKind::Permission, "bash").is_none());
    }

    fn run_ending(reason: TurnEndReason, error: Option<&str>) -> TurnRun {
        TurnRun {
            final_text: String::new(),
            reason,
            error: error.map(str::to_string),
            last_request: RunTokens::default(),
            session_total: None,
            tokens: RunTokens::default(),
            elapsed: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn only_a_completed_turn_exits_zero() {
        run_ending(TurnEndReason::Completed, None)
            .require_completed()
            .expect("a completed turn is a success");
        assert!(
            run_ending(TurnEndReason::Cancelled, None)
                .require_completed()
                .is_err()
        );
        assert!(
            run_ending(TurnEndReason::MaxSteps, None)
                .require_completed()
                .is_err()
        );
        let failed = run_ending(TurnEndReason::Error, Some("upstream exploded"))
            .require_completed()
            .expect_err("an error turn is not a success");
        assert!(failed.to_string().contains("upstream exploded"), "{failed}");
    }

    #[test]
    fn tokens_are_the_meter_delta_and_never_go_negative() {
        let before = RunTokens {
            prompt: 10,
            completion: 2,
            cache_hit: 4,
            cache_miss: 6,
        };
        let after = RunTokens {
            prompt: 30,
            completion: 7,
            cache_hit: 9,
            cache_miss: 21,
        };
        assert_eq!(
            after.since(before),
            RunTokens {
                prompt: 20,
                completion: 5,
                cache_hit: 5,
                cache_miss: 15,
            }
        );
        // A reset meter row must not report negative spending.
        assert_eq!(before.since(after), RunTokens::default());
    }

    #[test]
    fn turn_finished_carries_the_usage_a_headless_run_reports() {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "agent/turn_finished",
            "params": {
                "session_id": "s1",
                "turn_id": "t1",
                "reason": "completed",
                "final_text": "done",
                "turn_token_stats": {
                    "prompt_tokens": 10, "completion_tokens": 2,
                    "cache_hit_tokens": 4, "cache_miss_tokens": 6
                },
                "snapshot": { "cumulative_token_stats": {
                    "prompt_tokens": 30, "completion_tokens": 7,
                    "cache_hit_tokens": 9, "cache_miss_tokens": 21
                }}
            }
        });
        let IncomingWire::TurnFinished {
            reason,
            final_text,
            turn_token_stats,
            cumulative_token_stats,
            ..
        } = classify_incoming(&frame)
        else {
            panic!("expected turn_finished");
        };
        assert_eq!(reason, TurnEndReason::Completed);
        assert_eq!(final_text.as_deref(), Some("done"));
        assert_eq!(
            RunTokens::from_stats(&turn_token_stats.expect("last request usage")),
            RunTokens {
                prompt: 10,
                completion: 2,
                cache_hit: 4,
                cache_miss: 6,
            }
        );
        assert_eq!(
            RunTokens::from_stats(&cumulative_token_stats.expect("session meter")),
            RunTokens {
                prompt: 30,
                completion: 7,
                cache_hit: 9,
                cache_miss: 21,
            }
        );
    }

    #[test]
    fn a_frame_without_a_meter_reports_no_session_total() {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "agent/turn_finished",
            "params": { "session_id": "s1", "turn_id": "t1", "reason": "cancelled" }
        });
        let IncomingWire::TurnFinished {
            reason,
            cumulative_token_stats,
            ..
        } = classify_incoming(&frame)
        else {
            panic!("expected turn_finished");
        };
        assert_eq!(reason, TurnEndReason::Cancelled);
        assert!(cumulative_token_stats.is_none());
    }
}
