//! Behavioral contract tests for subagent spawn: the child session must be a
//! first-class citizen — spawned through the same turn machinery as a main
//! session, reading live configuration, and bound to its own agent's provider.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::config::SettingsWriter;
use crate::config::TurnGuard;
use crate::config::global_db;
use crate::config::resolved::{WorkspaceState, resolve};
use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
use crate::provider_catalog;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::optional::EngineManager;
use crate::runtime::RuntimeHandle;
use crate::session::manager::SessionManager;
use crate::session::model::TurnResult;
use crate::tools::subagent::{LaunchSpec, SpawnDeps, spawn_child_job};

/// Two providers on closed local ports: any LLM call fails fast, and the error
/// text names the port — which lets tests assert WHICH provider was called.
const CATALOG: &str = r#"version = 1

[[providers]]
id = "p1"
name = "P1"
endpoint = "http://127.0.0.1:60001/v1"
endpoint_type = "responses"

[[providers]]
id = "p2"
name = "P2"
endpoint = "http://127.0.0.1:60002/v1"
endpoint_type = "responses"

[[models]]
id = "m1"
provider_id = "p1"
context_window = 8000
max_output = 1024

[[models]]
id = "m2"
provider_id = "p2"
context_window = 8000
max_output = 1024

[[models]]
id = "m3"
provider_id = "p2"
context_window = 8000
max_output = 1024
"#;

fn settings_with_explore_model(explore_model: &str) -> GlobalSettings {
    GlobalSettings {
        provider_credentials: HashMap::from([
            ("p1".into(), "sk-test".into()),
            ("p2".into(), "sk-test".into()),
        ]),
        agents: HashMap::from([
            (
                "default".into(),
                AgentProfile {
                    role: AgentRole::Primary,
                    model_ref: "p1/m1".into(),
                    allowed_subagents: vec!["explore".into()],
                    ..Default::default()
                },
            ),
            (
                "explore".into(),
                AgentProfile {
                    role: AgentRole::Subagent,
                    model_ref: explore_model.into(),
                    ..Default::default()
                },
            ),
            (
                "compaction".into(),
                AgentProfile {
                    role: AgentRole::Hidden,
                    model_ref: "p1/m1".into(),
                    ..Default::default()
                },
            ),
        ]),
        ..Default::default()
    }
}

struct SpawnEnv {
    _dir: tempfile::TempDir,
    sessions: Arc<SessionManager>,
    parent: String,
    writer: SettingsWriter,
    revision: Arc<AtomicU64>,
    /// Parent-side runtime handle whose `resolved` is the snapshot a parent
    /// tool list would have been built against at construction time.
    runtime: RuntimeHandle,
}

/// Build a spawn environment whose global DB currently holds `explore_model`
/// as the explore agent's model_ref. The returned handle's `resolved` is a
/// snapshot of that state (what a parent tool list would have been built
/// against).
async fn spawn_env(explore_model: &str) -> SpawnEnv {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("litecode.db");

    let global = settings_with_explore_model(explore_model);
    std::fs::write(provider_catalog::catalog_path_for_db(&db_path), CATALOG).unwrap();
    {
        let conn = global_db::open(&db_path).unwrap();
        global_db::store::replace_all(&conn, &global).unwrap();
    }

    let workspace = WorkspaceState::new(dir.path());
    let catalog = provider_catalog::shared_for_db(&db_path).unwrap();
    let resolved = resolve(global, workspace.clone(), catalog);

    let turn_guard = Arc::new(TurnGuard::new());
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::clone(&turn_guard),
        dir.path().join("sessions.db").to_string_lossy().to_string(),
    ));
    let parent = sessions
        .open_session("/proj", "default", None)
        .await
        .expect("parent session");

    let writer = SettingsWriter::with_path(db_path.clone(), turn_guard);
    let revision = writer.revision_handle();

    let engines = WorkspaceEngines::new();
    let ide = IdeBaseHandle::open(
        workspace.workspace_root.as_path(),
        Arc::new(engines.clone()),
    )
    .expect("ide base");

    let runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        workspace,
        Arc::new(EngineManager::new()),
        Arc::new(engines),
        ide,
        Arc::clone(&revision),
        db_path,
    );
    runtime.subagent_hub.attach_sessions(Arc::clone(&sessions));

    SpawnEnv {
        _dir: dir,
        sessions,
        parent,
        writer,
        revision,
        runtime,
    }
}

fn make_deps(env: &SpawnEnv) -> SpawnDeps {
    SpawnDeps {
        runtime: env.runtime.clone(),
        depth: 0,
        sessions: Arc::clone(&env.sessions),
    }
}

async fn spawn_and_wait(env: &SpawnEnv, deps: SpawnDeps) -> (String, TurnResult) {
    let spec = LaunchSpec {
        agent_name: "explore".into(),
        responsibility: "research".into(),
        prompt: "report back".into(),
    };
    let (child, turn_id) = spawn_child_job(&deps, &env.parent, "call-contract", spec)
        .await
        .expect("spawn");
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(result) = env.sessions.data().turn_result_blocking(&child, &turn_id) {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child turn must settle");
    (child, result)
}

/// First-class contract: a subagent launched after a settings write must read
/// the LIVE agent config (fresh model_ref), not the `ResolvedConfig` snapshot
/// the parent's tool list was built against.
#[tokio::test]
async fn spawn_reads_fresh_agent_model_ref_from_live_config() {
    let env = spawn_env("p2/m2").await;

    // Simulate: parent tool list built against v1, then the human edits the
    // explore agent's model in Settings (same global DB, revision advances).
    env.writer
        .write_agent(
            "explore",
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "p2/m3".into(),
                ..Default::default()
            },
            &WorkspaceState::new(std::path::Path::new("/tmp")),
        )
        .expect("settings write");
    assert!(env.revision.load(Ordering::Acquire) >= 1);

    let deps = make_deps(&env);
    let (child, _result) = spawn_and_wait(&env, deps).await;
    assert_eq!(
        env.sessions.session_model_id(&child).as_deref(),
        Some("p2/m3"),
        "child session must be seeded from the live config, not the parent snapshot"
    );
}

/// First-class contract: the child's LLM traffic goes to the provider resolved
/// from the AGENT's model_ref — never inherited from the parent session's
/// provider. Locked as a regression test for the parent-provider clobber bug.
#[tokio::test]
async fn child_calls_agent_provider_endpoint_not_parent_provider() {
    // Parent runs p1/m1 on p1 (port 60001); explore is configured with p2/m2 on
    // p2 (port 60002). The child's failing request must name p2's port.
    let env = spawn_env("p2/m2").await;

    let deps = make_deps(&env);
    let (_child, result) = spawn_and_wait(&env, deps).await;
    assert!(
        result.reason != "completed",
        "dummy endpoints must fail; unexpected success: {}",
        result.output
    );
    assert!(
        result.output.contains("60002"),
        "child must call the agent's provider (p2), got: {}",
        result.output
    );
    assert!(
        !result.output.contains("60001"),
        "child must NOT call the parent's provider (p1), got: {}",
        result.output
    );
}
