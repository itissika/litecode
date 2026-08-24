use std::path::PathBuf;

use litecode::config::ConfigManager;
use rusqlite::Connection;

fn fresh_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("litecode.db");
    (dir, db)
}

#[test]
fn config_global_db_migration_and_seed() {
    let (_dir, db) = fresh_db();
    let settings = ConfigManager::load_global_from(&db).expect("load seeds fresh db");

    assert!(
        settings.providers.is_empty(),
        "seed must not plant providers"
    );
    assert!(settings.models.is_empty(), "seed must not plant models");

    assert!(settings.agents.contains_key("default"));
    assert!(settings.agents.contains_key("compaction"));
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
