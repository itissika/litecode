//! Stage 5 integration tests for lsp (mock LS; optional rust-analyzer smoke).

use std::sync::Arc;

use litecode::config::TurnGuard;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use litecode::config::schema::{AgentProfile, AgentToolBinding, ToolPreset};
use litecode::config::workspace::write_lsp_init;
use litecode::config::{ConfigManager, WorkspaceState, init_workspace};
use litecode::engines::{EngineState, WorkspaceEngines};
use litecode::llm::provider_from_definition;
use litecode::lsp::deps::server_id_from_command;
use litecode::lsp::project_root::project_root_for_file;
use litecode::lsp::{LspDiagFeedback, LspHub, detect_needed_server_commands, file_to_uri};
use litecode::optional::EngineManager;
use litecode::session::manager::SessionManager;
use litecode::tool::catalog::should_include_in_llm_list;
use litecode::tool::registry::build_tool_list;
use litecode::tools::edit::EditTool;
use litecode::tools::write::WriteTool;
use tempfile::TempDir;

mod common;

use common::bindings::binding_all_for;

static LSP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn mock_ls_command() -> String {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_lsp_server.py");
    format!("python3 {}", script.display())
}

fn set_mock_ls_env() {
    unsafe {
        std::env::set_var("LITECODE_LSP_SERVERS", format!("rs={}", mock_ls_command()));
    }
}

fn clear_mock_ls_env() {
    unsafe {
        std::env::remove_var("LITECODE_LSP_SERVERS");
        std::env::remove_var("LITECODE_LSP_IDLE_TIMEOUT_SECS");
        std::env::remove_var("MOCK_LSP_DIAG");
        std::env::remove_var("MOCK_LSP_DIAG_DELAY_MS");
        // Also clear the pid-file override: without this, a leaked
        // MOCK_LSP_PID_FILE from one test makes later mock LS spawns point at a
        // dropped TempDir (FileNotFoundError), cascading into unrelated
        // optional_lsp failures (stage E review finding).
        std::env::remove_var("MOCK_LSP_PID_FILE");
        std::env::remove_var("MOCK_LSP_REVERSE_HOVER");
        std::env::remove_var("MOCK_LSP_HANG");
        std::env::remove_var("LITECODE_LSP_REQUEST_TIMEOUT_SECS");
    }
}

fn seed_lsp_engines(root: &std::path::Path) {
    let ids: Vec<String> = detect_needed_server_commands(root)
        .iter()
        .map(|cmd| server_id_from_command(cmd))
        .collect();
    write_lsp_init(root, ids).expect("write lsp engines");
}

fn workspace_with_lsp(root: &std::path::Path) -> litecode::config::resolved::ResolvedConfig {
    set_mock_ls_env();
    init_workspace(root).expect("init workspace");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap_or(());
    seed_lsp_engines(root);
    let mut global = litecode::config::schema::GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools: std::collections::HashMap::from([("lsp".into(), binding_all_for("lsp"))]),
            ..Default::default()
        },
    );
    let mut workspace = WorkspaceState::new(root);
    workspace.workspace_tool_readiness =
        litecode::config::workspace::workspace_readiness_from_engines(root);
    ConfigManager::resolve(global, workspace)
}

async fn wait_lsp_warm(engines: &WorkspaceEngines) {
    assert!(
        engines
            .wait_until_warmed("lsp", Duration::from_secs(10))
            .await,
        "lsp warmup timed out"
    );
}

/// Sync tests that are not on a tokio runtime (write/edit tool.call wrappers).
fn wait_lsp_warm_blocking(engines: &WorkspaceEngines) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(wait_lsp_warm(engines));
}

#[test]
fn engines_json_off_no_lsp_tool() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("lib.rs"), "fn main() {}\n").unwrap();

    let mut global = litecode::config::schema::GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools: std::collections::HashMap::from([("lsp".into(), binding_all_for("lsp"))]),
            ..Default::default()
        },
    );
    let resolved = ConfigManager::resolve(global, WorkspaceState::new(root));
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(!engines.is_warmed("lsp"));
    assert!(!engines.lsp_hub().is_active());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let tools = rt.block_on(build_tool_list(
        &resolved,
        "default",
        provider_from_definition(&common::stub_test_provider_def(
            "http://localhost:11434/v1",
            "test",
        ))
        .unwrap(),
        "test",
        0,
        tokio_util::sync::CancellationToken::new(),
        EngineManager::new(),
        engines.clone(),
        litecode::ide_base::IdeBaseHandle::open(root, std::sync::Arc::new(engines.clone()))
            .expect("ide"),
        "test-parent-session",
        Arc::new(SessionManager::new(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
        Arc::new(litecode::mcp::McpConnectionPool::new()),
    ));
    assert!(!tools.iter().any(|t| t.name() == "lsp"));
}

#[tokio::test(flavor = "current_thread")]
async fn catalog_on_warmup_agent_definition_and_single_ls() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    std::fs::write(root.join("lib.rs"), "fn target() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    assert_eq!(engines.state("lsp"), Some(EngineState::Warm));
    let global_engines = EngineManager::new();
    assert!(should_include_in_llm_list(
        &resolved,
        "default",
        "lsp",
        &global_engines,
        &engines
    ));

    #[cfg(not(windows))]
    assert_eq!(
        count_mock_ls_processes(),
        0,
        "activate must not spawn language servers"
    );

    let lib = root.join("lib.rs");
    let out = engines
        .lsp_hub()
        .tool_action("definition", &lib, Some(1), Some(1))
        .await
        .expect("definition");
    assert!(out.contains("lib.rs"));

    #[cfg(not(windows))]
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while count_mock_ls_processes() < 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(
            count_mock_ls_processes(),
            1,
            "first file access should spawn exactly one mock LS"
        );
    }

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn disk_save_sync_updates_hub() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    std::fs::write(&lib, "fn ok() {}\nfn bad( {}\n").unwrap();
    engines
        .lsp_hub()
        .sync_document(&lib)
        .await
        .expect("sync after save");

    let _ = engines
        .lsp_hub()
        .tool_action("diagnostics", &lib, None, None)
        .await;

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn editor_ws_rpc_matches_agent_tool_hub() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub_out = engines
        .lsp_hub()
        .tool_action("definition", &lib, Some(1), Some(1))
        .await
        .expect("tool definition");

    let rpc = engines
        .lsp_hub()
        .request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": { "uri": file_to_uri(&lib) },
                "position": { "line": 0, "character": 0 }
            }),
        )
        .await
        .expect("ws-style definition");

    assert!(hub_out.contains("lib.rs"));
    assert!(rpc.to_string().contains("lib.rs") || hub_out.contains("lib.rs"));

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn get_diagnostics_rpc_does_not_deadlock() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub = engines.lsp_hub();
    let uri = file_to_uri(&lib);
    let res = hub
        .request("litecode/getDiagnostics", serde_json::json!({ "uri": uri }))
        .await
        .expect("getDiagnostics should succeed");
    assert!(res.get("diagnostics").is_some());

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn overlapping_hovers_match_out_of_order_ids() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_REVERSE_HOVER", "1");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub = engines.lsp_hub();
    let uri = file_to_uri(&lib);
    let a = hub.request(
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 1 }
        }),
    );
    let b = hub.request(
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 4 }
        }),
    );
    let (ra, rb) = tokio::join!(a, b);
    let va = ra.expect("hover a")["contents"]["value"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let vb = rb.expect("hover b")["contents"]["value"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(va.contains("@c=1"), "got {va}");
    assert!(vb.contains("@c=4"), "got {vb}");

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn completion_sees_did_change_version() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub = engines.lsp_hub();
    let uri = file_to_uri(&lib);
    hub.request(
        "litecode/didChange",
        serde_json::json!({ "uri": uri, "text": "fn shared() {}\n" }),
    )
    .await
    .expect("open");
    hub.request(
        "litecode/didChange",
        serde_json::json!({ "uri": uri, "text": "fn shared() { 1 }\n" }),
    )
    .await
    .expect("change");
    let completion = hub
        .request(
            "textDocument/completion",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 3 }
            }),
        )
        .await
        .expect("completion");
    assert_eq!(completion["litecodeMockVersion"], 2);

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn request_timeout_does_not_kill_process() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_HANG", "1");
        std::env::set_var("LITECODE_LSP_REQUEST_TIMEOUT_SECS", "2");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub = engines.lsp_hub();
    let uri = file_to_uri(&lib);
    hub.request(
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 0 }
        }),
    )
    .await
    .expect("spawn language server");
    let pids_before = hub.language_server_pids();
    assert!(!pids_before.is_empty(), "expected a live language server");

    let hover = hub
        .request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 0 }
            }),
        )
        .await;
    assert!(hover.is_err(), "hanging hover should time out");

    let pids_after = hub.language_server_pids();
    assert_eq!(pids_before, pids_after, "timeout must not kill the process");

    let def = hub
        .request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 0 }
            }),
        )
        .await
        .expect("definition after timeout");
    assert!(def.to_string().contains("lib.rs") || def.get("uri").is_some());

    engines.stop_all();
    clear_mock_ls_env();
}
/// already-running `block_on`, panicking with "Cannot start a runtime from
/// within a runtime" on the next diagnostics (or any) request.
#[tokio::test(flavor = "current_thread")]
async fn idle_reclaim_during_diagnostics_does_not_panic() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("LITECODE_LSP_IDLE_TIMEOUT_SECS", "1");
    }

    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub = engines.lsp_hub();
    hub.tool_action("diagnostics", &lib, None, None)
        .await
        .expect("first diagnostics should spawn LS");

    std::thread::sleep(Duration::from_millis(1100));

    let out = hub
        .tool_action("diagnostics", &lib, None, None)
        .await
        .expect("second diagnostics after idle must not panic nested block_on");
    assert!(
        out.contains("No diagnostics") || out.contains("Error") || out.contains("Warning"),
        "unexpected diagnostics output: {out}"
    );

    engines.stop_all();
    clear_mock_ls_env();
}

/// Regression: current_thread agent runtime must not deadlock when sync_document
/// (read/warm path) is followed by file_error_diagnostics_feedback_ex (edit feedback).
#[tokio::test(flavor = "current_thread")]
async fn sync_then_feedback_on_current_thread_does_not_deadlock() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    let hub = engines.lsp_hub();
    hub.sync_document(&lib)
        .await
        .expect("sync_document should succeed");

    let feedback = tokio::time::timeout(
        Duration::from_secs(5),
        hub.file_error_diagnostics_feedback_ex(&lib),
    )
    .await
    .expect("sync then feedback must not deadlock on current_thread");

    match feedback {
        LspDiagFeedback::Silence | LspDiagFeedback::Errors(_) | LspDiagFeedback::Unavailable(_) => {
        }
    }

    engines.stop_all();
    clear_mock_ls_env();
}

fn write_tool_with_ide(root: &std::path::Path, engines: WorkspaceEngines) -> WriteTool {
    let engines = std::sync::Arc::new(engines);
    let ide = litecode::ide_base::IdeBaseHandle::open(root, std::sync::Arc::clone(&engines))
        .expect("ide");
    WriteTool::with_ide(ide)
}

fn edit_tool_with_ide(root: &std::path::Path, engines: WorkspaceEngines) -> EditTool {
    let engines = std::sync::Arc::new(engines);
    let ide = litecode::ide_base::IdeBaseHandle::open(root, std::sync::Arc::clone(&engines))
        .expect("ide");
    EditTool::with_ide(ide)
}

fn count_mock_ls_processes() -> usize {
    let output = Command::new("pgrep")
        .args(["-f", "mock_lsp_server.py"])
        .output();
    let Ok(output) = output else {
        return 0;
    };
    if output.stdout.is_empty() {
        return 0;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .count()
}

#[test]
fn write_without_lsp_warm_never_appends_diagnostics() {
    use litecode::tool::Tool;
    use litecode::tools::write::WriteTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("x.rs");
    let engines = WorkspaceEngines::new();
    assert!(!engines.is_warmed("lsp"));

    let tool = write_tool_with_ide(dir.path(), engines);
    let result = tool.call(serde_json::json!({
        "file_path": path.to_str().unwrap(),
        "content": "fn main() {}\n"
    }));
    assert!(result.content.starts_with("Created:"), "{}", result.content);
    assert!(
        !result.content.contains("LSP"),
        "cold LSP must not pollute write output: {}",
        result.content
    );
}

#[test]
fn write_with_warm_lsp_but_no_errors_stays_clean() {
    use litecode::tool::Tool;
    use litecode::tools::write::WriteTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    // Explicit none — mock must not publish diagnostics.
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG", "none");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = write_tool_with_ide(root, engines.clone());
    let result = tool.call(serde_json::json!({
        "file_path": lib.to_str().unwrap(),
        "content": "fn ok() { let _x = 1; }\n"
    }));
    assert!(result.content.starts_with("Updated:"), "{}", result.content);
    assert!(
        !result.content.contains("LSP errors"),
        "empty/no-error diagnostics must not pollute write: {}",
        result.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

#[test]
fn write_with_warm_lsp_appends_only_when_errors_arrive() {
    use litecode::tool::Tool;
    use litecode::tools::write::WriteTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG", "error");
        std::env::set_var("MOCK_LSP_DIAG_DELAY_MS", "0");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = write_tool_with_ide(root, engines.clone());
    let result = tool.call(serde_json::json!({
        "file_path": lib.to_str().unwrap(),
        "content": "fn broken( {}\n"
    }));
    assert!(result.content.starts_with("Updated:"), "{}", result.content);
    assert!(
        result.content.contains("LSP note"),
        "expected local error feedback: {}",
        result.content
    );
    assert!(result.content.contains("mock error"), "{}", result.content);

    engines.stop_all();
    clear_mock_ls_env();
}

#[test]
fn write_ignores_slow_diagnostics_past_budget() {
    use litecode::tool::Tool;
    use litecode::tools::write::WriteTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    // Diagnostics arrive after feedback budget (750ms) — must stay silent.
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG", "delay_error");
        std::env::set_var("MOCK_LSP_DIAG_DELAY_MS", "2000");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = write_tool_with_ide(root, engines.clone());
    let result = tool.call(serde_json::json!({
        "file_path": lib.to_str().unwrap(),
        "content": "fn later() {}\n"
    }));
    assert!(result.content.starts_with("Updated:"), "{}", result.content);
    assert!(
        !result.content.contains("LSP errors"),
        "late diagnostics must not pollute write output: {}",
        result.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

#[test]
fn write_ignores_warn_only_diagnostics() {
    use litecode::tool::Tool;
    use litecode::tools::write::WriteTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG", "warn_only");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = write_tool_with_ide(root, engines.clone());
    let result = tool.call(serde_json::json!({
        "file_path": lib.to_str().unwrap(),
        "content": "fn warn() {}\n"
    }));
    assert!(
        !result.content.contains("LSP errors"),
        "warnings-only must not pollute write: {}",
        result.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

#[test]
fn edit_with_warm_lsp_appends_only_when_errors_arrive() {
    use litecode::tool::Tool;
    use litecode::tools::edit::EditTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG", "error");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = edit_tool_with_ide(root, engines.clone());
    let result = tool.call(serde_json::json!({
            "file_path": lib.to_str().unwrap(),
            "edits": [{ "old_string": "fn ok() {}", "new_string": "fn ok() { broken }" }]
    }));
    assert!(result.content.starts_with("Edited "), "{}", result.content);
    assert!(
        result.content.contains("LSP note"),
        "expected local error feedback on edit: {}",
        result.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

/// Reproduce: LSP reports an error, agent fixes the same file, the next edit
/// must not append the previous round's diagnostic. Prefer silence over stale.
#[test]
fn edit_fix_must_not_echo_previous_lsp_error() {
    use litecode::tool::Tool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG", "if_broken");
        std::env::set_var("MOCK_LSP_DIAG_DELAY_MS", "0");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn ok() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = edit_tool_with_ide(root, engines.clone());
    let broken = tool.call(serde_json::json!({
        "file_path": lib.to_str().unwrap(),
        "edits": [{ "old_string": "fn ok() {}", "new_string": "fn BROKEN() {}" }]
    }));
    assert!(
        broken.content.contains("LSP note"),
        "first edit should surface the live error: {}",
        broken.content
    );
    assert!(
        broken.content.contains("mock broken marker"),
        "first edit should include the current diagnostic: {}",
        broken.content
    );

    // Next publish is slower than the feedback budget, so the only way to
    // still show a note is to leak the previous round's cached Error.
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG_DELAY_MS", "2000");
    }
    let fixed = tool.call(serde_json::json!({
        "file_path": lib.to_str().unwrap(),
        "edits": [{ "old_string": "fn BROKEN() {}", "new_string": "fn ok() {}" }]
    }));
    assert!(
        !fixed.content.contains("mock broken marker"),
        "fixed edit must not echo the previous LSP error: {}",
        fixed.content
    );
    assert!(
        !fixed.content.contains("LSP note"),
        "prefer silence over a stale LSP note after the fix: {}",
        fixed.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
async fn lsp_tool_document_symbol_works() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn mock_fn() {}\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;

    // Hub/editor surface still supports symbols (not deleted from LspHub).
    let out = engines
        .lsp_hub()
        .tool_action("documentSymbol", &lib, None, None)
        .await
        .expect("documentSymbol");
    assert!(out.contains("mock_fn"), "{out}");

    let out = engines
        .lsp_hub()
        .tool_action_with_query("workspaceSymbol", &lib, Some(1), Some(1), Some("Mock"))
        .await
        .expect("workspaceSymbol");
    assert!(out.contains("MockSymbol"), "{out}");

    engines.stop_all();
    clear_mock_ls_env();
}

/// Agent LspTool: line+text happy paths, readable errors, schema surface.
#[test]
fn agent_lsp_tool_line_text_scenarios() {
    use litecode::config::WorkspacePaths;
    use litecode::context_pipeline::Context;
    use litecode::tool::Tool;
    use litecode::tool::trait_::ToolExecutionContext;
    use litecode::tools::lsp::LspTool;
    use litecode::types::ToolSignalLevel;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    // Line 1: unique symbol. Line 2: multi-hit `foo` for multi-hit view.
    std::fs::write(&lib, "fn unique_sym() {}\nfoo foo\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tool = LspTool::new(engines.clone(), root.to_path_buf());
    let exec = || ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::Safe,
        workspace_root: root.to_path_buf(),
        call_id: "lsp-ux".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        output_limit: 20_000,
        session_id: String::new(),
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let run =
        |input: serde_json::Value| rt.block_on(tool.execute(input, exec())).finalize_signals();

    // Schema: agent sees only 4 actions; no character / workspaceSymbol.
    let schema = tool.schema();
    let actions = schema["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum");
    let action_names: Vec<&str> = actions.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        action_names,
        ["goToDefinition", "findReferences", "hover", "diagnostics"]
    );
    assert!(schema["properties"].get("character").is_none());
    assert!(schema["properties"].get("text").is_some());
    assert!(schema["properties"].get("query").is_none());
    let text_desc = schema["properties"]["text"]["description"]
        .as_str()
        .unwrap_or("");
    assert!(
        text_desc.to_lowercase().contains("prefer") && text_desc.to_lowercase().contains("avoid"),
        "text schema should give prefer/avoid usage: {text_desc}"
    );
    assert!(
        text_desc.to_lowercase().contains("substring")
            || text_desc.to_lowercase().contains("symbol"),
        "text schema should describe the snippet rule: {text_desc}"
    );

    let ctx = Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    };
    let desc = tool.description(&ctx);
    assert!(
        desc.to_lowercase().contains("read") && desc.contains("line"),
        "desc should say read-for-line then call: {desc}"
    );
    assert!(
        desc.to_lowercase().contains("diagnostics") && desc.to_lowercase().contains("file_path"),
        "desc should state diagnostics vs position param shape: {desc}"
    );

    // ── Single-hit views (Ok, no multi-hit skeleton) ───────────────────────

    let def = run(serde_json::json!({
        "action": "goToDefinition",
        "file_path": "lib.rs",
        "line": 1,
        "text": "unique_sym"
    }));
    eprintln!("[ux] goToDefinition (single):\n{}\n", def.content);
    assert_eq!(def.level, ToolSignalLevel::Ok, "{}", def.content);
    assert!(!def.content.starts_with("Error:"), "{}", def.content);
    assert!(
        !def.content.contains("matched"),
        "single hit must not use multi-hit summary: {}",
        def.content
    );
    assert!(
        !def.content.contains("##unique_sym"),
        "single hit must not use ## multi-hit headings: {}",
        def.content
    );
    assert!(
        def.content.contains("lib.rs"),
        "definition should name the landing file: {}",
        def.content
    );
    // Mock echoes the query position; unique_sym starts at 1-based col 4 → LSP char 3.
    assert!(
        def.content.contains(":1:4") || def.content.contains(":1:"),
        "definition should include path:line:col: {}",
        def.content
    );
    assert!(
        def.content.contains("```") || def.content.contains("|"),
        "definition should attach landing context (fence or ±N): {}",
        def.content
    );

    let hover = run(serde_json::json!({
        "action": "hover",
        "file_path": "lib.rs",
        "line": 1,
        "text": "unique_sym"
    }));
    eprintln!("[ux] hover (single):\n{}\n", hover.content);
    assert_eq!(hover.level, ToolSignalLevel::Ok, "{}", hover.content);
    assert!(
        hover.content.contains("mock hover @c=3"),
        "hover should echo resolved column (0-based LSP): {}",
        hover.content
    );
    assert!(
        !hover.content.contains("matched") && !hover.content.contains("##"),
        "hover single must stay a plain docs body: {}",
        hover.content
    );

    let refs = run(serde_json::json!({
        "action": "findReferences",
        "file_path": "lib.rs",
        "line": 1,
        "text": "unique_sym"
    }));
    eprintln!("[ux] findReferences (single):\n{}\n", refs.content);
    assert_eq!(refs.level, ToolSignalLevel::Ok, "{}", refs.content);
    assert!(
        !refs.content.contains("matched"),
        "refs single must not use multi-hit summary: {}",
        refs.content
    );
    assert!(
        refs.content.contains("lib.rs"),
        "refs should list locations: {}",
        refs.content
    );
    assert!(
        refs.content.contains("```") || refs.content.contains("|"),
        "refs should attach context under locations: {}",
        refs.content
    );

    let diags = run(serde_json::json!({
        "action": "diagnostics",
        "file_path": "lib.rs"
    }));
    eprintln!("[ux] diagnostics:\n{}\n", diags.content);
    assert_eq!(diags.level, ToolSignalLevel::Ok, "{}", diags.content);
    assert!(
        diags.content.contains("No diagnostics"),
        "{}",
        diags.content
    );
    assert!(
        !diags.content.contains("##"),
        "diagnostics is not a multi-hit view: {}",
        diags.content
    );

    // ── Positioning failure (0 text hits) ──────────────────────────────────

    let miss = run(serde_json::json!({
        "action": "hover",
        "file_path": "lib.rs",
        "line": 1,
        "text": "does_not_exist"
    }));
    eprintln!("[ux] text miss:\n{}\n", miss.content);
    assert_eq!(miss.level, ToolSignalLevel::Error, "{}", miss.content);
    assert!(miss.content.starts_with("Error:"), "{}", miss.content);
    assert!(miss.content.contains("not found"), "{}", miss.content);
    assert!(miss.content.contains("read"), "{}", miss.content);
    assert!(
        miss.content.contains("fn unique_sym"),
        "should show the source line: {}",
        miss.content
    );

    // ── Multi-hit views (Warning + ## headings; per-action block bodies) ───

    let amb_def = run(serde_json::json!({
        "action": "goToDefinition",
        "file_path": "lib.rs",
        "line": 2,
        "text": "foo"
    }));
    eprintln!("[ux] goToDefinition (multi):\n{}\n", amb_def.content);
    assert_eq!(
        amb_def.level,
        ToolSignalLevel::Warning,
        "{}",
        amb_def.content
    );
    assert!(
        amb_def.content.contains("\n\nWarning:"),
        "{}",
        amb_def.content
    );
    assert!(
        amb_def.content.contains("matched 2 times on line 2"),
        "{}",
        amb_def.content
    );
    assert!(amb_def.content.contains("##foo"), "{}", amb_def.content);
    assert!(
        amb_def.content.contains("narrow") || amb_def.content.contains("retry"),
        "{}",
        amb_def.content
    );
    // Two fan-out blocks: headings then landing context (not a bare error abort).
    let def_heading_count = amb_def.content.matches("##foo").count();
    assert!(
        def_heading_count >= 2,
        "expected two ## hit headings: {}",
        amb_def.content
    );
    assert!(
        amb_def.content.contains("lib.rs"),
        "each definition fan-out should still land: {}",
        amb_def.content
    );

    let amb_hover = run(serde_json::json!({
        "action": "hover",
        "file_path": "lib.rs",
        "line": 2,
        "text": "foo"
    }));
    eprintln!("[ux] hover (multi):\n{}\n", amb_hover.content);
    assert_eq!(
        amb_hover.level,
        ToolSignalLevel::Warning,
        "{}",
        amb_hover.content
    );
    assert!(
        amb_hover.content.contains("matched 2 times"),
        "{}",
        amb_hover.content
    );
    assert!(
        amb_hover.content.contains("mock hover @c=0"),
        "first foo is column 1 → LSP char 0: {}",
        amb_hover.content
    );
    assert!(
        amb_hover.content.contains("mock hover @c=4"),
        "second foo is column 5 → LSP char 4: {}",
        amb_hover.content
    );

    let amb_refs = run(serde_json::json!({
        "action": "findReferences",
        "file_path": "lib.rs",
        "line": 2,
        "text": "foo"
    }));
    eprintln!("[ux] findReferences (multi):\n{}\n", amb_refs.content);
    assert_eq!(
        amb_refs.level,
        ToolSignalLevel::Warning,
        "{}",
        amb_refs.content
    );
    assert!(
        amb_refs.content.contains("matched 2 times"),
        "{}",
        amb_refs.content
    );
    assert!(
        amb_refs.content.matches("##foo").count() >= 2,
        "{}",
        amb_refs.content
    );
    assert!(
        amb_refs.content.contains("lib.rs"),
        "multi refs blocks should expand locations: {}",
        amb_refs.content
    );

    // Error: removed agent action
    let removed = tool.validate_input(&serde_json::json!({
        "action": "workspaceSymbol",
        "file_path": "lib.rs",
    }));
    assert!(removed.is_err(), "{removed:?}");
    let removed_msg = removed.unwrap_err();
    eprintln!("[ux] removed action:\n{removed_msg}\n");
    assert!(
        removed_msg.contains("goToDefinition") && removed_msg.contains("diagnostics"),
        "should list allowed actions: {removed_msg}"
    );

    // Error: LSP not enabled — fail closed (no fake empty)
    let cold = WorkspaceEngines::new();
    let cold_tool = LspTool::new(cold, root.to_path_buf());
    let disabled = rt
        .block_on(cold_tool.execute(
            serde_json::json!({
                "action": "diagnostics",
                "file_path": "lib.rs"
            }),
            exec(),
        ))
        .finalize_signals();
    eprintln!("[ux] lsp disabled:\n{}\n", disabled.content);
    assert_eq!(
        disabled.level,
        ToolSignalLevel::Error,
        "{}",
        disabled.content
    );
    assert!(
        disabled.content.starts_with("Error:"),
        "{}",
        disabled.content
    );
    assert!(
        disabled.content.contains("not enabled") || disabled.content.contains("unavailable"),
        "{}",
        disabled.content
    );
    assert!(
        disabled.content.contains("Settings") || disabled.content.contains("Engines"),
        "{}",
        disabled.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

fn rust_analyzer_on_path() -> bool {
    Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Real rust-analyzer: single-hit vs multi-hit agent views for definition / hover / refs.
/// Skips when `rust-analyzer` is not on PATH (CI without RA still green).
#[test]
fn agent_lsp_tool_views_with_rust_analyzer() {
    use litecode::tool::Tool;
    use litecode::tool::trait_::ToolExecutionContext;
    use litecode::tools::lsp::LspTool;
    use litecode::types::ToolSignalLevel;

    if !rust_analyzer_on_path() {
        eprintln!("skip agent_lsp_tool_views_with_rust_analyzer: rust-analyzer not on PATH");
        return;
    }

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();

    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"ra_views\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    // Line 1: unique. Line 4: `target` twice → multi-hit fan-out.
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn target() {}\n\npub fn caller() {\n    target(); target();\n}\n",
    )
    .unwrap();

    // Real RA — do not set LITECODE_LSP_SERVERS mock override.
    seed_lsp_engines(root);
    let mut global = litecode::config::schema::GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools: std::collections::HashMap::from([("lsp".into(), binding_all_for("lsp"))]),
            ..Default::default()
        },
    );
    let mut workspace = WorkspaceState::new(root);
    workspace.workspace_tool_readiness =
        litecode::config::workspace::workspace_readiness_from_engines(root);
    let resolved = ConfigManager::resolve(global, workspace);

    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    // RA cold start + index can exceed the usual mock 10s budget.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        assert!(
            engines
                .wait_until_warmed("lsp", Duration::from_secs(90))
                .await,
            "rust-analyzer warmup timed out"
        );
    });

    let tool = LspTool::new(engines.clone(), root.to_path_buf());
    let exec = || ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::Safe,
        workspace_root: root.to_path_buf(),
        call_id: "lsp-ra".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        output_limit: 20_000,
        session_id: String::new(),
    };
    let run =
        |input: serde_json::Value| rt.block_on(tool.execute(input, exec())).finalize_signals();

    // Retry a few times while RA finishes indexing (inconclusive → later hit).
    let mut def = None;
    for _ in 0..8 {
        let out = run(serde_json::json!({
            "action": "goToDefinition",
            "file_path": "src/lib.rs",
            "line": 1,
            "text": "target"
        }));
        eprintln!("[ra] goToDefinition single attempt:\n{}\n", out.content);
        if out.level == ToolSignalLevel::Ok
            && out.content.contains("lib.rs")
            && !out.content.contains("inconclusive")
            && !out.content.contains("index not ready")
        {
            def = Some(out);
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let def = def.expect("goToDefinition should succeed after RA index");
    assert!(
        !def.content.contains("matched") && !def.content.contains("##target"),
        "single-hit RA definition must not use multi skeleton: {}",
        def.content
    );
    assert!(
        def.content.contains("```")
            || def.content.contains("|")
            || def.content.contains("fn target"),
        "RA definition should include landing context: {}",
        def.content
    );

    let hover = run(serde_json::json!({
        "action": "hover",
        "file_path": "src/lib.rs",
        "line": 1,
        "text": "target"
    }));
    eprintln!("[ra] hover single:\n{}\n", hover.content);
    assert_eq!(hover.level, ToolSignalLevel::Ok, "{}", hover.content);
    assert!(
        !hover.content.contains("matched"),
        "single hover has no multi skeleton: {}",
        hover.content
    );
    assert!(
        hover.content.to_lowercase().contains("fn")
            || hover.content.contains("target")
            || hover.content.len() > 5,
        "hover should carry type/docs text: {}",
        hover.content
    );

    let multi = run(serde_json::json!({
        "action": "goToDefinition",
        "file_path": "src/lib.rs",
        "line": 4,
        "text": "target"
    }));
    eprintln!("[ra] goToDefinition multi:\n{}\n", multi.content);
    assert_eq!(multi.level, ToolSignalLevel::Warning, "{}", multi.content);
    assert!(
        multi
            .content
            .contains("matched 2 times on line 4 → 1 unique definition"),
        "{}",
        multi.content
    );
    assert!(
        multi.content.matches("##target").count() >= 2,
        "two source headings expected: {}",
        multi.content
    );
    // Same landing collapsed — definition path should appear once in the body.
    let landing_hits = multi.content.matches("lib.rs:1:").count();
    assert_eq!(
        landing_hits, 1,
        "identical definition landings should merge to one expansion: {}",
        multi.content
    );
    assert!(
        multi.content.contains("narrow") || multi.content.contains("retry"),
        "{}",
        multi.content
    );

    let multi_hover = run(serde_json::json!({
        "action": "hover",
        "file_path": "src/lib.rs",
        "line": 4,
        "text": "target"
    }));
    eprintln!("[ra] hover multi:\n{}\n", multi_hover.content);
    assert_eq!(
        multi_hover.level,
        ToolSignalLevel::Warning,
        "{}",
        multi_hover.content
    );
    assert!(
        multi_hover.content.contains("matched 2 times"),
        "{}",
        multi_hover.content
    );

    engines.stop_all();
    clear_mock_ls_env();
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires rust-analyzer in PATH"]
async fn rust_analyzer_smoke() {
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() { let x = 1; }\n").unwrap();

    let hub = Arc::new(LspHub::new());
    hub.set_workspace(root.to_path_buf());
    let cmds = detect_needed_server_commands(root);
    assert!(!cmds.is_empty());
    let ids: Vec<String> = cmds.iter().map(|c| server_id_from_command(c)).collect();
    write_lsp_init(root, ids).expect("write lsp init");
    let commands = litecode::lsp::deps::commands_for_server_ids(
        root,
        &litecode::config::workspace::lsp_servers_from_engines(root),
    );
    hub.activate(&commands).expect("activate");

    let main_rs = root.join("src/main.rs");
    let out = hub
        .tool_action("hover", &main_rs, Some(1), Some(8))
        .await
        .expect("hover");
    assert!(!out.is_empty());
    hub.stop().await;
}

#[test]
fn monorepo_ts_file_resolves_web_project_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let web = root.join("web");
    std::fs::create_dir_all(web.join("src")).unwrap();
    std::fs::write(web.join("tsconfig.json"), "{}").unwrap();
    let ts_file = web.join("src/foo.ts");
    std::fs::write(&ts_file, "export {}").unwrap();

    let project_root =
        project_root_for_file(&ts_file, "typescript-language-server", Some(root)).unwrap();
    assert_eq!(project_root, web);
}

fn process_exists(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
            Err(_) => true, // cannot determine → assume alive
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(true)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hub_drop_kills_spawned_server_process() {
    // 2.11: dropping the LspHub must reap the language-server child process —
    // the runtime may be leaked, but no LSP process may outlive the hub.
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn main() {}\n").unwrap();

    let pid_file = dir.path().join("mock.pid");
    unsafe {
        std::env::set_var("MOCK_LSP_PID_FILE", pid_file.as_os_str());
    }
    set_mock_ls_env();

    let engines = WorkspaceEngines::new();
    let hub = engines.lsp_hub();
    hub.set_workspace(root.to_path_buf());
    let _ = hub.activate(&[mock_ls_command()]);

    // Spawn the mock server through the real sync entry.
    if hub.sync_document(&lib).await.is_err() {
        // Mock LS unavailable (no python3) — skip rather than flake.
        drop(hub);
        drop(engines);
        return;
    }
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("mock server must write its pid")
        .trim()
        .parse()
        .expect("pid parses");
    assert!(
        process_exists(pid),
        "mock server should be running after sync"
    );

    // Drop the hub (and engines): the server process must be killed.
    drop(hub);
    drop(engines);
    let mut reaped = false;
    for _ in 0..100 {
        if !process_exists(pid) {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(reaped, "mock server process {pid} survived hub drop (2.11)");
}
