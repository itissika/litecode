//! WP-155 harness: replays `serve::run` (src/serve/run.rs:51) up to the bind, step by step, with a
//! monotonic `Instant` around each public call, then decomposes `ServeState::new` (src/serve/state.rs)
//! by calling its public pieces standalone. Read-only replica: no product code is changed.
//!
//! usage: wp155_open_harness <workspace> <mode: serve_order|decompose> [--wait-warm-ms N]
//! prints one JSON line: {"mode", "steps":[{"step","src","ms"}], "cs_warm_ms", "lsp_state"}
use std::sync::Arc;
use std::time::{Duration, Instant};

use litecode::config::{ConfigManager, SettingsWriter, TurnGuard};
use litecode::engines::WorkspaceEngines;
use litecode::optional::EngineManager;

struct Steps(Vec<serde_json::Value>, Instant);
impl Steps {
    fn time<T>(&mut self, step: &str, src: &str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.0.push(serde_json::json!({
            "step": step, "src": src,
            "start_ms": (t - self.1).as_secs_f64() * 1e3,
            "ms": t.elapsed().as_secs_f64() * 1e3,
        }));
        out
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[1]);
    let mode = args.get(2).map(String::as_str).unwrap_or("serve_order");
    let wait_warm = args
        .iter()
        .position(|a| a == "--wait-warm-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(600_000u64);
    let mut s = Steps(Vec::new(), Instant::now());

    let resolved = s.time("load_runtime_bundle", "src/cli/cli.rs:93", || {
        ConfigManager::load_runtime_bundle(Some(&root))
    })?;
    let workspace = resolved.workspace().clone();

    match mode {
        "serve_order" => {
            s.time("resolve_web_dist", "src/serve/run.rs:70", litecode::serve::web_dist::resolve_web_dist)?;
            s.time("sweep_orphan_lsp_processes", "src/serve/run.rs:75 → src/proc_lifetime.rs:48",
                litecode::proc_lifetime::sweep_orphan_lsp_processes);
            let turn_guard = Arc::new(TurnGuard::new());
            let engine_manager = Arc::new(EngineManager::new());
            let workspace_engines = Arc::new(WorkspaceEngines::new());
            let mut writer = SettingsWriter::new(turn_guard.clone());
            writer.set_engine_manager(Arc::clone(&engine_manager));
            let writer = Arc::new(writer);
            s.time("EngineManager::reconcile", "src/serve/run.rs:84", || engine_manager.reconcile(&resolved));
            s.time("WorkspaceEngines::reconcile (spawns warmup threads)", "src/serve/run.rs:85 → src/engines/mod.rs:773/1105",
                || workspace_engines.reconcile(&resolved));
            s.time("path_index.attach", "src/serve/run.rs:86 → src/engines/path_index/mod.rs:83",
                || workspace_engines.path_index().attach(resolved.workspace_root()));
            let state = s.time("ServeState::new", "src/serve/run.rs:91 → src/serve/state.rs:37", || {
                litecode::serve::state::ServeState::new(
                    resolved.clone(), "default".into(), workspace, Arc::clone(&engine_manager),
                    Arc::clone(&workspace_engines), None, None, turn_guard, writer,
                )
            })?;
            let rt = s.time("tokio runtime build", "src/serve/run.rs:110", || {
                tokio::runtime::Builder::new_multi_thread().enable_all().build()
            })?;
            rt.block_on(async {
                let t = Instant::now();
                litecode::workspace::restart_watcher(&state.watcher, state.workspace.clone()).await?;
                s.0.push(serde_json::json!({"step": "restart_watcher (awaited before bind)",
                    "src": "src/serve/router.rs:112 → src/workspace/watcher.rs:270",
                    "start_ms": (t - s.1).as_secs_f64() * 1e3, "ms": t.elapsed().as_secs_f64() * 1e3}));
                let t = Instant::now();
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                s.0.push(serde_json::json!({"step": "TcpListener::bind", "src": "src/serve/router.rs:253",
                    "start_ms": (t - s.1).as_secs_f64() * 1e3, "ms": t.elapsed().as_secs_f64() * 1e3}));
                drop(l);
                anyhow::Ok(())
            })?;
            // Background: code_search warm (worker spawn + IPC + index load), polled every 10 ms.
            let t = Instant::now();
            let mut cs_warm = None;
            while t.elapsed() < Duration::from_millis(wait_warm) {
                match workspace_engines.state("code_search") {
                    Some(litecode::engines::EngineState::Warm) => {
                        cs_warm = Some((Instant::now() - s.1).as_secs_f64() * 1e3);
                        break;
                    }
                    Some(litecode::engines::EngineState::Failed) | None => break,
                    _ => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            println!("{}", serde_json::json!({"mode": mode, "steps": s.0, "cs_warm_ms": cs_warm,
                "cs_state": format!("{:?}", workspace_engines.state("code_search")),
                "lsp_state": format!("{:?}", workspace_engines.state("lsp"))}));
            workspace_engines.stop_all();
            engine_manager.stop_all();
        }
        "decompose" => {
            // The public pieces of ServeState::with_project, in its order, standalone.
            let litecode_dir = workspace.paths.sessions_db.parent().unwrap().to_path_buf();
            let lease = s.time("WorkspaceWriteLease::acquire", "src/serve/state.rs:~85", || {
                litecode::session::workspace_lock::WorkspaceWriteLease::acquire(&litecode_dir)
            })?;
            let data = s.time("SessionData::open (sessions.db)", "src/serve/state.rs:90", || {
                litecode::session::data::SessionData::open(&lease, &workspace.paths.sessions_db)
            })?;
            let ids = s.time("list_session_ids", "src/serve/state.rs:99", || {
                data.list_session_ids_blocking().unwrap_or_default()
            });
            let _ = s.time("maintain_snapshots", "src/serve/state.rs:97", || {
                litecode::session::snapshot::maintain_snapshots(&workspace.paths.snapshots_dir, &ids)
            });
            let _ws = s.time("WorkspaceService::new", "src/serve/state.rs:111 → src/workspace/service.rs:97", || {
                litecode::workspace::WorkspaceService::new(root.clone())
            })?;
            let _hub = s.time("TerminalHub::new", "src/serve/state.rs:112", litecode::terminal::TerminalHub::new);
            println!("{}", serde_json::json!({"mode": mode, "steps": s.0, "session_count": ids.len()}));
        }
        other => anyhow::bail!("unknown mode {other}"),
    }
    Ok(())
}
