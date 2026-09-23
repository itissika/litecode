//! Phase F acceptance tests:
//! - 2.16 (REV-6): concurrent commit_partial must not lose updates.
//! - Session mutations go through SessionData and must not hold the manager
//!   records lock.

use std::sync::Arc;
use std::time::Duration;

use litecode::config::TurnGuard;
use litecode::config::resolved::WorkspaceState;
use litecode::config::schema::AgentProfile;
use litecode::session::manager::SessionManager;
use tempfile::TempDir;

mod common;

use common::{default_test_global, seed_global_db, test_serve_settings_with_db};

#[test]
fn concurrent_commit_partial_does_not_lose_updates() {
    let db_dir = TempDir::new().expect("db dir");
    let db_path = db_dir.path().join("litecode.db");
    // commit_partial projects the summary through the catalog, so seed the
    // fixture catalog next to this DB before the first load.
    common::seed_test_catalog(&db_path, common::TEST_CATALOG_ENDPOINT, 128_000);
    seed_global_db(&db_path, &default_test_global());
    let turn_guard = Arc::new(TurnGuard::new());
    let (writer, _engines) = test_serve_settings_with_db(turn_guard, &db_path);
    let writer = Arc::new(writer);

    // 8 threads each commit a DISTINCT settings document entry. Without the
    // commit_partial process mutex (REV-6) the read-modify-write races would
    // drop all but one. (Provider definitions moved to the catalog, so the
    // per-id settings document is now the agent map.)
    let workspace = WorkspaceState::new("/tmp/f2-lock-scope");
    let mut handles = Vec::new();
    for i in 0..8u32 {
        let w = Arc::clone(&writer);
        let workspace = workspace.clone();
        handles.push(std::thread::spawn(move || {
            w.write_agent(
                &format!("agent_{i}"),
                AgentProfile {
                    description: format!("concurrent {i}"),
                    ..Default::default()
                },
                &workspace,
            )
            .expect("write agent");
        }));
    }
    for h in handles {
        h.join().expect("thread join");
    }

    let settings = writer.load_settings().expect("load settings");
    for i in 0..8u32 {
        assert!(
            settings.agents.contains_key(&format!("agent_{i}")),
            "agent_{i} update lost under concurrent commit (REV-6)"
        );
    }
}

#[test]
fn session_mutation_does_not_hold_records_lock() {
    let dir = TempDir::new().expect("dir");
    let db_path = dir.path().join("sessions.db");
    let db_str = db_path.to_str().expect("db path str").to_string();
    let manager = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_str,
    ));
    let sid = manager
        .open_session_sync("/proj", "default", Some("test/test-primary-model"))
        .expect("open session");

    manager
        .insert_detail_rows(&sid, &[litecode::types::user_text("x")])
        .expect("insert");
    assert!(
        manager.records_lock_free(),
        "records lock must be free after SessionData mutation"
    );
}
