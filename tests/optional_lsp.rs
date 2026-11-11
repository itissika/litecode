//! Stage 5 integration tests for lsp (mock LS; optional rust-analyzer smoke).

use std::sync::Arc;

use litecode::config::TurnGuard;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use litecode::config::schema::{AgentProfile, AgentToolBinding, ToolPreset};
use litecode::config::workspace::write_lsp_init;
use litecode::config::{ConfigManager, WorkspaceState, init_workspace};
use litecode::engines::WorkspaceEngines;
use litecode::lsp::deps::server_id_from_command;
use litecode::lsp::{LspDiagFeedback, detect_needed_server_commands, file_to_uri};
use litecode::optional::EngineManager;
use litecode::session::manager::SessionManager;
use litecode::tool::registry::build_tool_list;
use litecode::tools::edit::EditTool;
use litecode::tools::write::WriteTool;
use tempfile::TempDir;

mod common;

use common::bindings::binding_all_for;

static LSP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Write/edit diagnostics live in the result data, not in a side channel:
/// `run_code` never reads `ToolCallResult::diagnostics`.
fn lsp_error_text(result: &litecode::types::ToolCallResult) -> String {
    result
        .data
        .as_ref()
        .and_then(|data| data.get("diagnostics"))
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item["code"] == "lsp_error")
                .filter_map(|item| item["details"]["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
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
        std::env::remove_var("MOCK_LSP_COLD_MS");
        std::env::remove_var("MOCK_LSP_DIE_ON_FIRST_SYMBOL");
        std::env::remove_var("LITECODE_LSP_INDEX_WAIT_MS");
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



#[cfg(windows)]
fn kill_pid(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status();
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
}

/// The reported repro: kill the server, then call once with a path. That call
/// must start a fresh server and answer; the process that answers is not the
/// one that was killed.
#[tokio::test(flavor = "current_thread")]
async fn the_first_call_after_a_kill_starts_a_fresh_server() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let pid_file = root.join("mock-ls.pid");
    unsafe {
        std::env::set_var("MOCK_LSP_PID_FILE", pid_file.to_string_lossy().to_string());
    }
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();
    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;
    let hub = engines.lsp_hub();

    let first = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect("first call starts the server");
    assert!(first.text.contains("MockSymbol"), "{}", first.text);
    let killed: u32 = std::fs::read_to_string(&pid_file)
        .expect("mock pid file")
        .trim()
        .parse()
        .expect("pid");
    kill_pid(killed);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect("the call after a kill starts a fresh server and answers");
    assert!(after.text.contains("MockSymbol"), "{}", after.text);
    let respawned: u32 = std::fs::read_to_string(&pid_file)
        .expect("mock pid file")
        .trim()
        .parse()
        .expect("pid");
    assert_ne!(killed, respawned, "a fresh process must serve the call");

    engines.stop_all();
    clear_mock_ls_env();
}
/// A cold start that settles inside the short grace is still answered by the
/// call that started the server: no empty result and no I/O failure leaks out
/// while the index is being built.
#[tokio::test(flavor = "current_thread")]
async fn the_call_that_starts_a_cold_server_also_answers_it() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    // Longer than any single retry step, shorter than the grace: the wait for a
    // reported index is bounded, so only a window this short fits in one call.
    unsafe {
        std::env::set_var("MOCK_LSP_COLD_MS", "2500");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();
    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;
    let hub = engines.lsp_hub();

    let out = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect("the starting call must also answer");
    assert!(out.text.contains("MockSymbol"), "{}", out.text);

    // The same instance now answers without a cold wait.
    let again = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect("second call");
    assert!(again.text.contains("MockSymbol"), "{}", again.text);

    engines.stop_all();
    clear_mock_ls_env();
}


/// An explicit "indexing" is not worth the whole budget: the call comes back in
/// a few seconds so the model can do something else and try again later.
#[tokio::test(flavor = "current_thread")]
async fn an_indexing_server_gets_a_short_grace_not_the_whole_budget() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_COLD_MS", "9000");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();
    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;
    let hub = engines.lsp_hub();

    // The first call that meets a reported index pays the grace once.
    let started = std::time::Instant::now();
    let error = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect_err("a 9s index window cannot be answered by one call");
    let paid = started.elapsed();
    assert!(error.to_string().contains("still starting"), "{error}");
    assert!(
        paid >= Duration::from_secs(2),
        "the first call must pay the grace, not return at once: {paid:?}"
    );
    println!("first call paid {paid:?} of grace");

    // Every later call in the same epoch answers at once: the model spends its
    // time on another tool instead of being held for the same seconds again.
    let mut calls = 1u32;
    let mut probes: Vec<Duration> = Vec::new();
    loop {
        calls += 1;
        assert!(calls < 12, "the 9s index window never ended");
        tokio::time::sleep(Duration::from_secs(1)).await;
        let started = std::time::Instant::now();
        let outcome = hub.workspace_symbol_outcome("Mock", Some(&lib)).await;
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(1500),
            "call {calls} was held for {elapsed:?} on an epoch already paid"
        );
        probes.push(elapsed);
        match outcome {
            Ok(out) => {
                assert!(out.text.contains("MockSymbol"), "{}", out.text);
                println!("call {calls}: data after {elapsed:?}");
                break;
            }
            Err(error) => {
                assert!(error.to_string().contains("still starting"), "{error}");
                println!("call {calls}: still starting after {elapsed:?}");
            }
        }
    }
    println!("calls {calls}; probes {probes:?}");
    assert!(calls >= 3, "a 9s index window took only {calls} calls");

    engines.stop_all();
    clear_mock_ls_env();
}
/// When the index does not settle inside this call's budget, the call says the
/// server is still starting — never that the query found nothing.
#[tokio::test(flavor = "current_thread")]
async fn a_bounded_wait_reports_still_starting_not_a_result() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    unsafe {
        std::env::set_var("MOCK_LSP_COLD_MS", "2000");
        std::env::set_var("LITECODE_LSP_INDEX_WAIT_MS", "600");
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();
    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;
    let hub = engines.lsp_hub();

    let error = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect_err("an unbuilt index must not produce a result");
    let message = error.to_string();
    assert!(message.contains("still starting"), "{message}");
    assert!(message.contains("index not ready"), "{message}");

    // Once the window elapses, the next call gets the data.
    tokio::time::sleep(Duration::from_millis(1800)).await;
    let out = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect("after the cold window");
    assert!(out.text.contains("MockSymbol"), "{}", out.text);

    engines.stop_all();
    clear_mock_ls_env();
}

/// A kill between the liveness check and the write used to surface as
/// "language server I/O failed". One more pass resolves a fresh server and the
/// call still answers.
#[tokio::test(flavor = "current_thread")]
async fn a_dead_pipe_mid_call_is_retried_once() {
    let _guard = LSP_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mock_ls_env();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let flag = root.join("died-once");
    unsafe {
        std::env::set_var("MOCK_LSP_DIE_ON_FIRST_SYMBOL", flag.to_string_lossy().to_string());
    }
    let lib = root.join("lib.rs");
    std::fs::write(&lib, "fn shared() {}\n").unwrap();
    let resolved = workspace_with_lsp(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    wait_lsp_warm(&engines).await;
    let hub = engines.lsp_hub();

    let out = hub
        .workspace_symbol_outcome("Mock", Some(&lib))
        .await
        .expect("the retried call must answer");
    assert!(out.text.contains("MockSymbol"), "{}", out.text);
    assert!(flag.exists(), "the first pass really hit the dying server");

    engines.stop_all();
    clear_mock_ls_env();
}
#[test]
fn engines_json_off_keeps_lsp_tools() {
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
    let permission =
        litecode::permission::PermissionEngine::resolver(resolved.clone(), "default", 0);
    let context = litecode::context_pipeline::Context {
        cwd: root.to_path_buf(),
        workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    };
    let host = Arc::new(litecode::tools::code::CodeToolHost::new(
        permission,
        context,
        "default",
        litecode::permission::deny_permission_sink(),
        root.join(".litecode"),
        litecode::tool::write_lock::process_write_lock(),
        None,
    ));
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
    let tools = rt
        .block_on(build_tool_list(
            &runtime,
            "default",
            0,
            tokio_util::sync::CancellationToken::new(),
            "test-parent-session",
            Arc::new(SessionManager::new_for_test(
                Arc::new(TurnGuard::new()),
                String::new(),
            )),
            Some(host),
        ))
        .expect("build tool list");
    let transport = tools
        .iter()
        .find(|tool| tool.name() == "run_code")
        .expect("LSP card must bundle into run_code even when the engine is off");
    let bindings = transport.code_bindings().expect("code bindings");
    assert!(bindings.iter().any(|name| name == "go_to_definition"));
    assert!(bindings.iter().any(|name| name == "hover"));
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
        "expected lsp_error diagnostic in data: {:?}",
        result.data
    );
    assert!(lsp.contains("mock error"), "{lsp}");
    // Write keeps the diagnostics as data; there is no flattened message.
    assert!(
        result.data.as_ref().unwrap().get("message").is_none(),
        "{:?}",
        result.data
    );

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
    // The diagnostics field is the only carrier: no message tail, no side channel.
    let broken_lsp = lsp_error_text(&broken);
    assert!(
        !broken_lsp.is_empty(),
        "first edit should surface the live error: {broken:?}",
    );
    assert!(
        broken_lsp.contains("mock broken marker"),
        "first edit should include the current diagnostic: {broken_lsp}"
    );
    let broken_data = broken.data.as_ref().expect("edit data");
    assert!(broken_data.get("message").is_none(), "{broken_data}");

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
    assert!(
        !fixed_lsp.contains("mock broken marker"),
        "fixed edit must not echo the previous LSP error: {fixed_lsp}"
    );

    engines.stop_all();
    clear_mock_ls_env();
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
