use std::path::PathBuf;

use litecode::config::ConfigManager;
use litecode::config::global_db;
use litecode::config::schema::AgentToolBinding;
use litecode::permission::{BindingPathMode, ToolPolicy};
use rusqlite::Connection;

fn fresh_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("litecode.db");
    (dir, db)
}

/// Rows persisted by older builds must not block boot: load repairs them so the
/// document validates again (subagent may not bind plan/todo or carry a subagent allowlist).
#[test]
fn legacy_role_rule_rows_are_repaired_on_load() {
    let (_dir, db) = fresh_db();
    let mut settings = ConfigManager::load_global_from(&db).expect("load seeds fresh db");

    let binding = AgentToolBinding {
        enabled: false,
        policy: ToolPolicy::allow_all(),
        path_mode: BindingPathMode::default(),
        last_applied_preset: None,
        allowed_tools: None,
    };
    let explore = settings.agents.get_mut("explore").expect("explore agent");
    explore.tools.insert("plan".into(), binding.clone());
    explore.tools.insert("todo".into(), binding);
    explore.allowed_subagents = vec!["explore".into()];

    assert!(
        ConfigManager::validate(&settings).is_err(),
        "the legacy state must be what validation rejects"
    );

    global_db::save_global(&db, &settings).expect("write legacy rows");

    let repaired = ConfigManager::load_global_from(&db).expect("load must repair, not fail");
    ConfigManager::validate(&repaired).expect("repaired settings must validate");

    let explore = repaired.agents.get("explore").expect("explore agent");
    assert!(!explore.tools.contains_key("plan"));
    assert!(!explore.tools.contains_key("todo"));
    assert!(explore.allowed_subagents.is_empty());
    assert!(
        explore.tools.contains_key("read"),
        "legit bindings must survive the repair"
    );
}

#[test]
fn config_global_db_migration_and_seed() {
    let (_dir, db) = fresh_db();
    let settings = ConfigManager::load_global_from(&db).expect("load seeds fresh db");

    assert!(
        settings.provider_credentials.is_empty(),
        "seed must not plant provider credentials"
    );

    assert!(settings.agents.contains_key("default"));
    assert!(settings.agents.contains_key("compaction"));
    assert!(settings.agents.contains_key("explore"));
    assert!(settings.agents.contains_key("general"));
    assert!(settings.agents.contains_key("orchestrator"));
    assert_eq!(
        settings.agents.get("default").unwrap().system_prompt,
        "builtin:general"
    );
    assert_eq!(
        settings.agents.get("orchestrator").unwrap().system_prompt,
        "builtin:orchestrator"
    );
    assert_eq!(
        settings.agents.get("explore").unwrap().system_prompt,
        "builtin:explore"
    );
    assert_eq!(
        settings.agents.get("general").unwrap().system_prompt,
        "builtin:general"
    );
    assert_eq!(
        settings.agents.get("default").unwrap().allowed_subagents,
        vec!["explore".to_string()]
    );
    assert_eq!(
        settings
            .agents
            .get("orchestrator")
            .unwrap()
            .allowed_subagents,
        vec!["explore".to_string(), "general".to_string()]
    );
    assert!(settings.agents.get("default").unwrap().model_ref.is_empty());
    assert!(
        settings
            .agents
            .get("compaction")
            .unwrap()
            .model_ref
            .is_empty()
    );

    assert!(
        settings
            .agents
            .get("default")
            .unwrap()
            .tools
            .contains_key("read")
    );
    assert!(
        settings
            .agents
            .get("default")
            .unwrap()
            .tools
            .contains_key("wait_shell")
    );

    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
            .unwrap(),
        CURRENT_USER_VERSION
    );
}

// The codebase is the source of truth — this test asserts the seeded DB
// matches the actual migration version (previously a stale hardcoded 3).
const CURRENT_USER_VERSION: i32 = litecode::config::global_db::current_user_version();
