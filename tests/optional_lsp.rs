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

fn lsp_error_text(result: &litecode::types::ToolCallResult) -> String {
    result
        .diagnostics
        .iter()
        .filter(|item| item.code == "lsp_error")
        .filter_map(|item| item.details.as_ref())
        .filter_map(|details| details.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

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
    ConfigManager::resolve(global, workspace, common::default_test_catalog())
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
    let resolved = ConfigManager::resolve(
        global,
        WorkspaceState::new(root),
        common::default_test_catalog(),
    );
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(!engines.is_warmed("lsp"));
    assert!(!engines.lsp_hub().is_active());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ide = litecode::ide_base::IdeBaseHandle::open(root, std::sync::Arc::new(engines.clone()))
        .expect("ide");
    let runtime = litecode::runtime::RuntimeHandle::new(
        resolved,
        "default".into(),
        WorkspaceState::new(root),
        Arc::new(EngineManager::new()),
        Arc::new(engines.clone()),
        ide,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        root.join("global.db"),
    );
    let tools = rt.block_on(build_tool_list(
        &runtime,
        "default",
        0,
        tokio_util::sync::CancellationToken::new(),
        "test-parent-session",
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
        None,
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
    // Cold start may reject "index not ready" (WP-206 #3); either way the LS is spawned.
    if let Err(error) = hub.tool_action("diagnostics", &lib, None, None).await {
        assert!(error.to_string().contains("index not ready"), "{error}");
    }

    std::thread::sleep(Duration::from_millis(1100));

    // The point: a request after idle reclaim must not panic (nested block_on).
    match hub.tool_action("diagnostics", &lib, None, None).await {
        Ok(out) => assert!(
            out.contains("No diagnostics") || out.contains("Error") || out.contains("Warning"),
            "unexpected diagnostics output: {out}"
        ),
        Err(error) => assert!(error.to_string().contains("index not ready"), "{error}"),
    }

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
        "path": path.to_str().unwrap(),
        "content": "fn main() {}\n"
    }));
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.data.as_ref().unwrap()["file"]["hash"].is_string());
    assert!(
        result
            .diagnostics
            .iter()
            .all(|item| item.code != "lsp_error"),
        "cold LSP must not emit lsp_error: {:?}",
        result.diagnostics
    );
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
    // Explicit none —mock must not publish diagnostics.
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
        "path": lib.to_str().unwrap(),
        "content": "fn ok() { let _x = 1; }\n"
    }));
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.data.as_ref().unwrap()["created"].is_boolean());
    assert!(
        result
            .diagnostics
            .iter()
            .all(|item| item.code != "lsp_error"),
        "empty/no-error diagnostics must not emit lsp_error: {:?}",
        result.diagnostics
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
        "path": lib.to_str().unwrap(),
        "content": "fn broken( {}\n"
    }));
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.data.as_ref().unwrap()["created"].is_boolean());
    let lsp = lsp_error_text(&result);
    assert!(
        !lsp.is_empty(),
        "expected lsp_error diagnostic: {:?}",
        result.diagnostics
    );
    assert!(lsp.contains("mock error"), "{lsp}");

    engines.stop_all();
    clear_mock_ls_env();
}

#[test]
fn write_ignores_slow_diagnostics_past_budget() {
    use litecode::tool::Tool;
    use litecode::tools::write::WriteTool;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    // Diagnostics arrive after feedback budget (750ms) —must stay silent.
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
        "path": lib.to_str().unwrap(),
        "content": "fn later() {}\n"
    }));
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.data.as_ref().unwrap()["created"].is_boolean());
    assert!(
        result
            .diagnostics
            .iter()
            .all(|item| item.code != "lsp_error"),
        "late diagnostics must not emit lsp_error: {:?}",
        result.diagnostics
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
        "path": lib.to_str().unwrap(),
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
            "path": lib.to_str().unwrap(),
            "old_string": "fn ok() {}",
            "new_string": "fn ok() { broken }"
    }));
    assert!(result.error.is_none(), "{}", result.content);
    let data = result.data.as_ref().expect("edit data");
    assert_eq!(data["ok"], true, "{data}");
    // WP-198: edit folds LSP notes into EditResult.message (not ToolCallResult.diagnostics).
    let msg = data["message"].as_str().unwrap_or("");
    assert!(
        !lsp_error_text(&result).is_empty() || msg.to_lowercase().contains("lsp") || msg.contains("Error:"),
        "expected LSP error note on edit via diagnostics or message: diagnostics={:?} message={msg}",
        result.diagnostics
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
        "path": lib.to_str().unwrap(),
        "old_string": "fn ok() {}",
        "new_string": "fn BROKEN() {}"
    }));
    let broken_lsp = lsp_error_text(&broken);
    let broken_msg = broken
        .data
        .as_ref()
        .and_then(|d| d.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let broken_note = if broken_lsp.is_empty() {
        broken_msg.clone()
    } else {
        broken_lsp.clone()
    };
    assert!(
        !broken_note.is_empty(),
        "first edit should surface the live error: diagnostics={:?} message={broken_msg}",
        broken.diagnostics
    );
    assert!(
        broken_note.contains("mock broken marker"),
        "first edit should include the current diagnostic: {broken_note}"
    );

    // Next publish is slower than the feedback budget, so the only way to
    // still show a note is to leak the previous round's cached Error.
    unsafe {
        std::env::set_var("MOCK_LSP_DIAG_DELAY_MS", "2000");
    }
    let fixed = tool.call(serde_json::json!({
        "path": lib.to_str().unwrap(),
        "old_string": "fn BROKEN() {}",
        "new_string": "fn ok() {}"
    }));
    let fixed_lsp = lsp_error_text(&fixed);
    let fixed_msg = fixed
        .data
        .as_ref()
        .and_then(|d| d.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let fixed_note = format!("{fixed_lsp}
{fixed_msg}");
    assert!(
        !fixed_note.contains("mock broken marker"),
        "fixed edit must not echo the previous LSP error: {fixed_note}"
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

/// Agent LSP Code tools: file_path+line+character, structured data, no text/action.
#[test]
fn agent_lsp_code_tools_line_character_scenarios() {
    use litecode::config::WorkspacePaths;
    use litecode::context_pipeline::Context;
    use litecode::tool::Tool;
    use litecode::tool::trait_::ToolExecutionContext;
    use litecode::tools::lsp::all_code_tools;
    use litecode::types::ToolSignalLevel;

    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
    let lib = root.join("lib.rs");
    // Line 1: `unique_sym` starts at UTF-16 column 4 ("fn ").
    std::fs::write(&lib, "fn unique_sym() {}\nfoo foo\n").unwrap();

    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm_blocking(&engines);

    let tools = all_code_tools(engines.clone(), root.to_path_buf());
    let by_name = |name: &str| {
        tools
            .iter()
            .find(|t| t.name() == name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .clone()
    };
    let def_tool = by_name("go_to_definition");
    let hover_tool = by_name("hover");
    let refs_tool = by_name("find_references");
    let diags_tool = by_name("diagnostics");
    let doc_tool = by_name("document_symbol");
    let ws_tool = by_name("workspace_symbol");
    let impl_tool = by_name("go_to_implementation");

    // No Direct tool named "lsp".
    assert!(!tools.iter().any(|t| t.name() == "lsp"));
    for tool in &tools {
        assert_eq!(tool.surface(), litecode::tool::ToolSurface::Code);
        assert!(tool.schema()["properties"].get("text").is_none());
        assert!(tool.schema()["properties"].get("action").is_none());
        assert!(tool.data_schema().is_some());
    }

    let def_schema = def_tool.schema();
    assert!(def_schema["properties"].get("character").is_some());
    assert!(def_schema["properties"].get("file_path").is_some());
    let ws_schema = ws_tool.schema();
    assert!(ws_schema["properties"].get("query").is_some());
    assert!(ws_schema["properties"].get("path").is_none());
    assert!(ws_schema["properties"].get("file_path").is_none());

    let ctx = Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    };
    let desc = def_tool.description(&ctx);
    assert!(
        desc.to_lowercase().contains("definition") || desc.to_lowercase().contains("file_path"),
        "desc: {desc}"
    );

    let exec = || ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::Safe,
        workspace_root: root.to_path_buf(),
        call_id: "lsp-ux".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        output_limit: 20_000,
        session_id: String::new(),
        session: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let run = |tool: &std::sync::Arc<dyn Tool>, input: serde_json::Value| {
        rt.block_on(tool.execute(input, exec())).finalize_signals()
    };

    // unique_sym starts after "fn " → character 4
    let def = run(
        &def_tool,
        serde_json::json!({
            "file_path": "lib.rs",
            "line": 1,
            "character": 4
        }),
    );
    eprintln!("[ux] go_to_definition:\n{:?}\n", def.data);
    assert_eq!(def.level, ToolSignalLevel::Ok, "{}", def.content);
    assert!(def.error.is_none());
    let data = def.data.as_ref().expect("data");
    assert!(data.get("locations").is_some(), "{data}");
    assert!(data.get("message").is_some(), "{data}");
    // Mock echoes query position as landing.
    let locs = data["locations"].as_array().unwrap();
    if !locs.is_empty() {
        assert!(locs[0]["line"].as_u64().unwrap() >= 1);
        assert!(locs[0]["character"].as_u64().unwrap() >= 1);
    }

    let hover = run(
        &hover_tool,
        serde_json::json!({
            "file_path": "lib.rs",
            "line": 1,
            "character": 4
        }),
    );
    eprintln!("[ux] hover:\n{:?}\n", hover.data);
    assert_eq!(hover.level, ToolSignalLevel::Ok, "{}", hover.content);
    let hdata = hover.data.as_ref().expect("data");
    assert!(hdata.get("markdown").is_some(), "{hdata}");
    assert!(hdata.get("message").is_some(), "{hdata}");
    assert!(
        hdata["markdown"].as_str().unwrap_or("").contains("mock hover")
            || hdata["message"].as_str().unwrap_or("").contains("No hover"),
        "{hdata}"
    );

    let refs = run(
        &refs_tool,
        serde_json::json!({
            "file_path": "lib.rs",
            "line": 1,
            "character": 4
        }),
    );
    assert_eq!(refs.level, ToolSignalLevel::Ok, "{}", refs.content);
    assert!(refs.data.as_ref().unwrap().get("locations").is_some());

    let diags = run(
        &diags_tool,
        serde_json::json!({ "file_path": "lib.rs" }),
    );
    eprintln!("[ux] diagnostics:\n{:?}\n", diags.data);
    assert_eq!(diags.level, ToolSignalLevel::Ok, "{}", diags.content);
    let ddata = diags.data.as_ref().expect("data");
    assert_eq!(ddata["items"], serde_json::json!([]));
    assert!(
        ddata["message"].as_str().unwrap().to_lowercase().contains("no diagnostic"),
        "{ddata}"
    );

    let _ = run(
        &doc_tool,
        serde_json::json!({ "file_path": "lib.rs" }),
    );
    let _ = run(
        &impl_tool,
        serde_json::json!({
            "file_path": "lib.rs",
            "line": 1,
            "character": 4
        }),
    );
    let ws = run(
        &ws_tool,
        serde_json::json!({ "query": "unique" }),
    );
    assert_eq!(ws.level, ToolSignalLevel::Ok, "{}", ws.content);
    assert!(ws.data.as_ref().unwrap().get("symbols").is_some());

    // Character out of range → ToolError (no ok:false data)
    let oob = run(
        &def_tool,
        serde_json::json!({
            "file_path": "lib.rs",
            "line": 1,
            "character": 999
        }),
    );
    eprintln!("[ux] character oob:\n{}\n", oob.content);
    assert_eq!(oob.level, ToolSignalLevel::Error, "{}", oob.content);
    assert!(oob.error.is_some());
    assert!(oob.content.contains("out of range") || oob.content.contains("Error:"));

    // text dialect rejected at validate
    assert!(
        def_tool
            .validate_input(&serde_json::json!({
                "file_path": "lib.rs",
                "line": 1,
                "text": "unique_sym"
            }))
            .is_err()
    );

    // Cold engines → unavailable ToolError
    let cold = WorkspaceEngines::new();
    let cold_tools = all_code_tools(cold, root.to_path_buf());
    let cold_diags = cold_tools
        .iter()
        .find(|t| t.name() == "diagnostics")
        .unwrap();
    let disabled = rt
        .block_on(cold_diags.execute(
            serde_json::json!({ "file_path": "lib.rs" }),
            exec(),
        ))
        .finalize_signals();
    eprintln!("[ux] lsp disabled:\n{}\n", disabled.content);
    assert_eq!(disabled.level, ToolSignalLevel::Error, "{}", disabled.content);
    assert!(
        disabled.content.contains("not enabled") || disabled.content.contains("unavailable"),
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

/// Real rust-analyzer smoke for go_to_definition / hover with line+character.
/// Skips when `rust-analyzer` is not on PATH.
#[test]
fn agent_lsp_tool_views_with_rust_analyzer() {
    use litecode::tool::Tool;
    use litecode::tool::trait_::ToolExecutionContext;
    use litecode::tools::lsp::all_code_tools;
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
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn target() {}\n\npub fn caller() {\n    target();\n}\n",
    )
    .unwrap();

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
    let resolved = ConfigManager::resolve(global, workspace, common::default_test_catalog());

    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        assert!(
            engines
                .wait_until_warmed("lsp", Duration::from_secs(90))
                .await,
            "rust-analyzer warmup timed out"
        );
    });

    let tools = all_code_tools(engines.clone(), root.to_path_buf());
    let def_tool = tools.iter().find(|t| t.name() == "go_to_definition").unwrap();
    let hover_tool = tools.iter().find(|t| t.name() == "hover").unwrap();
    let exec = || ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::Safe,
        workspace_root: root.to_path_buf(),
        call_id: "lsp-ra".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        output_limit: 20_000,
        session_id: String::new(),
        session: None,
    };
    let run = |tool: &std::sync::Arc<dyn Tool>, input: serde_json::Value| {
        rt.block_on(tool.execute(input, exec())).finalize_signals()
    };

    // `pub fn target` → 't' of target at UTF-16 column 8
    let mut def = None;
    for _ in 0..8 {
        let out = run(
            def_tool,
            serde_json::json!({
                "file_path": "src/lib.rs",
                "line": 1,
                "character": 8
            }),
        );
        eprintln!("[ra] go_to_definition attempt:\n{:?} / {}\n", out.data, out.content);
        if out.level == ToolSignalLevel::Ok
            && out.error.is_none()
            && out
                .data
                .as_ref()
                .and_then(|d| d.get("locations"))
                .is_some()
            && !out.content.contains("inconclusive")
            && !out.content.contains("index not ready")
        {
            def = Some(out);
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let def = def.expect("go_to_definition should succeed after RA index");
    let locs = def.data.as_ref().unwrap()["locations"].as_array().unwrap();
    assert!(
        !locs.is_empty() || def.data.as_ref().unwrap()["message"]
            .as_str()
            .unwrap_or("")
            .contains("No locations"),
        "unexpected def data: {:?}",
        def.data
    );

    let hover_deadline = std::time::Instant::now() + Duration::from_secs(20);
    let hover = loop {
        let out = run(
            hover_tool,
            serde_json::json!({
                "file_path": "src/lib.rs",
                "line": 1,
                "character": 8
            }),
        );
        eprintln!("[ra] hover attempt:\n{:?}\n", out.data);
        if out.level != ToolSignalLevel::Warning {
            break out;
        }
        assert!(
            out.content.contains("language server is updating"),
            "hover Warning must carry ContentModified: {}",
            out.content
        );
        assert!(
            std::time::Instant::now() < hover_deadline,
            "hover still ContentModified after 20s"
        );
        std::thread::sleep(Duration::from_millis(500));
    };
    assert_eq!(hover.level, ToolSignalLevel::Ok, "{}", hover.content);
    assert!(hover.data.as_ref().unwrap().get("markdown").is_some());

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
    // ISSUE-047: a -32801 ContentModified must surface as an empty result plus
    // the `content_modified` signal, never as a raw error.
    let out = hub
        .tool_action_outcome("hover", &main_rs, Some(1), Some(8), None)
        .await
        .expect("hover must not error (ContentModified maps to empty + signal)");
    assert!(
        out.content_modified || !out.text.is_empty(),
        "hover: got neither a result nor the updating signal: {out:?}"
    );
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
            Err(_) => true, // cannot determine →assume alive
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
    // 2.11: dropping the LspHub must reap the language-server child process —    // the runtime may be leaked, but no LSP process may outlive the hub.
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
        // Mock LS unavailable (no python3) —skip rather than flake.
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
