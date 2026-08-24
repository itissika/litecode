//! CLI entry point — transitional escape hatch.
//! The CLI is NOT the target interaction path; the Electron client is.
//! This module exists for development convenience and may be removed in a future version.

use std::io::Write;
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
use crate::client_protocol::protocol::{JsonRpcRequestEnvelope, WireEvent};
use crate::config::{ConfigManager, ResolvedConfig, SettingsWriter, cli_turn_guard};
use crate::runtime::RuntimeHandle;
use crate::session::store::Session;
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
    ConfigManager::load_runtime_bundle(workspace_override(cli)).map_err(Into::into)
}

fn get_api_key(resolved: &ResolvedConfig) -> anyhow::Result<String> {
    if let Some(agent) = resolved.agents().get("default")
        && !agent.model_ref.is_empty()
        && let Some(model) = resolved.models().get(&agent.model_ref)
        && let Some(provider) = resolved.providers().get(&model.provider_ref)
    {
        let key = provider.config.api_key.trim();
        if !key.is_empty() {
            return Ok(key.to_string());
        }
    }
    resolved
        .providers()
        .values()
        .find(|p| crate::llm::provider_ready(p))
        .map(|p| p.config.api_key.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no ready provider api_key; configure providers via Web Settings or `litecode config set`"
            )
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
        .with_filter(tracing_subscriber::EnvFilter::new("info"));

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

/// Run one agent turn with streaming output to stdout via the wire connection loop.
fn run_turn_streaming(
    rt: &tokio::runtime::Runtime,
    conn: &mut ConnectionHandle,
    prompt: &str,
) -> anyhow::Result<String> {
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
                IncomingWire::PermissionRequest {
                    request_id,
                    tool,
                    rule_id: _,
                    summary,
                    ..
                } => {
                    let (approved, always) = ask_cli_permission(&tool, &summary);
                    conn.request_tx
                        .send(SessionRequest::PermissionGrant {
                            request_id,
                            tool,
                            approved,
                            always,
                        })
                        .map_err(|_| anyhow::anyhow!("connection closed"))?;
                }
                IncomingWire::TurnFinished { final_text, .. } => {
                    return Ok(final_text.unwrap_or_default());
                }
                IncomingWire::OperationFailed(msg) => anyhow::bail!("{}", msg),
                IncomingWire::Ignored => {}
            }
        }
    })
}

/// Print streaming wire events to stdout.
fn print_wire_event(event: &WireEvent) {
    match event {
        WireEvent::StreamEvent { event: stream } => {
            use crate::authority::responses::ResponseStreamEvent;
            match stream {
                ResponseStreamEvent::ResponseOutputTextDelta(e) => {
                    print!("{}", e.delta);
                    std::io::stdout().flush().ok();
                }
                ResponseStreamEvent::ResponseReasoningSummaryTextDelta(e) => {
                    // Reasoning stream: ignore for CLI stdout (text only).
                    let _ = e;
                }
                _ => {}
            }
        }
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
            tracing::info!(pending, in_progress, completed, "todo progress");
            eprintln!("[todos: ○{} ◐{} ●{}]", pending, in_progress, completed);
        }
        _ => {}
    }
}

struct CliSession {
    rt: tokio::runtime::Runtime,
    conn: ConnectionHandle,
}

impl CliSession {
    fn new(runtime: RuntimeHandle, session_id: &str) -> anyhow::Result<Self> {
        let turn_guard = cli_turn_guard();
        let db_path = runtime.db_path();
        let sessions = std::sync::Arc::new(crate::session::SessionManager::new(
            turn_guard.clone(),
            db_path,
        ));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()?;
        let mut controller =
            SessionController::with_turn_guard(runtime, Some(session_id.to_string()), sessions)?;
        let conn = rt.block_on(async {
            // Ensure proper broadcast subscription for the session.
            controller.subscribe(session_id).await;
            ConnectionHandle::spawn(controller)
        });
        Ok(Self { rt, conn })
    }

    fn run_turn(&mut self, prompt: &str) -> anyhow::Result<String> {
        run_turn_streaming(&self.rt, &mut self.conn, prompt)
    }
}

/// Run an interactive REPL loop via the wire connection.
fn run_interactive(session: &mut CliSession) -> anyhow::Result<()> {
    println!("\n--- interactive mode (type /help for commands) ---");

    loop {
        print!("litecode> ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            break;
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

        let result = session.run_turn(input)?;

        if !result.is_empty() {
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
        let sessions = Session::list_sessions(db_path.to_str().expect("valid path"))?;
        for (id, project, updated, preview, _agent_id, _model_id) in sessions {
            let preview_str = if preview.is_empty() {
                ""
            } else {
                &format!("  |  {}", preview)
            };
            println!("{}  {}  {}{}", id, project, updated, preview_str);
        }
        return Ok(());
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
        .map(|p| p.model_ref.as_str())
        .filter(|s| !s.is_empty());
    let session = Session::open(runtime.db_path().as_str(), &project, &agent_id, model_id)?;
    let mut cli_session = CliSession::new(runtime, &session.id)?;

    if !cli.prompt.is_empty() {
        let prompt = cli.prompt.join(" ");
        let result = cli_session.run_turn(&prompt)?;

        if !result.is_empty() {
            println!();
        }
    }

    run_interactive(&mut cli_session)?;

    Ok(())
}
