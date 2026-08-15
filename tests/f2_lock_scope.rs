//! Phase F acceptance tests:
//! - 2.16 (REV-6): concurrent commit_partial must not lose updates.
//! - 2.15 (REV-5): with_entry_store runs the closure OUTSIDE the records lock
//!   (re-entering it inside the closure must not deadlock).

use std::sync::Arc;
use std::time::Duration;

use litecode::config::TurnGuard;
use litecode::config::schema::{ProviderConnectionConfig, ProviderDefinition};
use litecode::session::manager::SessionManager;
use litecode::session::store::Session;
use tempfile::TempDir;

mod common;

use common::{default_test_global, seed_global_db, test_serve_settings_with_db};

#[test]
fn concurrent_commit_partial_does_not_lose_updates() {
    let db_dir = TempDir::new().expect("db dir");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path, &default_test_global());
    let turn_guard = Arc::new(TurnGuard::new());
    let (writer, _engines) = test_serve_settings_with_db(turn_guard, &db_path);
    let writer = Arc::new(writer);

    // 8 threads each commit a DISTINCT provider. Without the commit_partial
    // process mutex (REV-6) the read-modify-write races would drop all but one.
    let mut handles = Vec::new();
    for i in 0..8u32 {
        let w = Arc::clone(&writer);
        handles.push(std::thread::spawn(move || {
            w.write_provider(ProviderDefinition {
                id: format!("provider-{i}"),
                adapter_id: litecode::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
                label: format!("P{i}"),
                config: ProviderConnectionConfig {
                    endpoint: format!("http://example-{i}/v1"),
                    api_key: format!("sk-{i}"),
                    auth: litecode::config::schema::ProviderAuth::Bearer,
                },
            })
            .expect("write provider");
        }));
    }
    for h in handles {
        h.join().expect("thread join");
    }

    let settings = writer.load_settings().expect("load settings");
    for i in 0..8u32 {
        assert!(
            settings.providers.contains_key(&format!("provider-{i}")),
            "provider-{i} update lost under concurrent commit (REV-6)"
        );
    }
}

#[test]
fn with_entry_store_runs_closure_outside_records_lock() {
    let dir = TempDir::new().expect("dir");
    let db_path = dir.path().join("sessions.db");
    let db_str = db_path.to_str().expect("db path str").to_string();
    let session = Session::open(&db_str, "/proj", "default", Some("m")).expect("open session");
    let sid = session.id.clone();
    let manager = Arc::new(SessionManager::new(Arc::new(TurnGuard::new()), db_str));
    manager.register_for_test(session);

    // Prove the records lock is released before the closure runs (2.15 REV-5):
    // the probe try-locks the records mutex — if with_entry_store still held it,
    // try_lock fails. Single-threaded, so the only potential holder would be
    // with_entry_store itself.
    let m2 = Arc::clone(&manager);
    let sid2 = sid.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let result = m2.with_entry_store(&sid2, |_s| {
            // The records lock must be free here (REV-5: no DB I/O under it).
            Ok(if m2.records_lock_free() { 1 } else { 0 })
        });
        let _ = tx.send(result.ok().unwrap_or(-1));
    });

    let probe = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("with_entry_store closure did not run (records lock still held)");
    assert_eq!(
        probe, 1,
        "records lock was still held while the closure ran (REV-5)"
    );
    drop(handle); // joined implicitly; if the test failed the process exit reaps it
}
