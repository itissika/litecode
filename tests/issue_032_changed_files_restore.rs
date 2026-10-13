//! WP-054 / ISSUE-032 — changed-files restore after seam failure.
//!
//! Session-level contract tests for the API `exec.rs` calls on spine Err and
//! FilesChanged-not-written paths: `restore_changed_files` → `mark_dirty`.
//! Full AgentLoop seam inject is out of scope (no Err inject fixture); the
//! Runtime restore call sites are covered by code review + these contracts.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use common::SessionDataFixture;
use litecode::config::TurnGuard;
use litecode::session::manager::SessionManager;

fn manager_with_open_session(fx: &SessionDataFixture) -> (SessionManager, String) {
    let mgr = SessionManager::from_data(Arc::new(TurnGuard::new()), Arc::clone(&fx.data));
    let sid = mgr
        .open_session_sync("proj-032", "agent", Some("test-model"))
        .expect("open session");
    (mgr, sid)
}

fn scratch_file(fx: &SessionDataFixture, name: &str, body: &str) -> PathBuf {
    let dir = fx.db_path().parent().expect("fixture dir").join("issue032");
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(name);
    fs::write(&path, body).expect("write");
    path
}

/// T1 — models spine_reminder_view Err after take: restore so next take
/// still reports the same paths (exec.rs spine Err branch).
#[test]
fn t1_spine_failure_restores_changed_files_for_next_seam() {
    let fx = SessionDataFixture::new();
    let (mgr, sid) = manager_with_open_session(&fx);
    let path = scratch_file(&fx, "t1.txt", "v1");
    mgr.note_touched_files(&sid, &[path.clone()]);
    fs::write(&path, "external").expect("mutate");

    let changed = mgr.take_changed_files(&sid);
    assert_eq!(
        changed,
        vec![path.clone()],
        "precondition: take sees change"
    );

    // Simulate spine Err → exec calls restore_changed_files(&changed).
    mgr.restore_changed_files(&sid, &changed);
    assert_eq!(
        mgr.take_changed_files(&sid),
        vec![path],
        "after restore, next seam take must re-report"
    );
}

/// T2 — models FilesChanged append soft-fail: restore when not written;
/// seam itself stays soft (caller does not escalate to hard Err).
#[test]
fn t2_files_changed_append_failure_soft_restores() {
    let fx = SessionDataFixture::new();
    let (mgr, sid) = manager_with_open_session(&fx);
    let path = scratch_file(&fx, "t2.txt", "v1");
    mgr.note_touched_files(&sid, &[path.clone()]);
    fs::write(&path, "external").expect("mutate");

    let changed = mgr.take_changed_files(&sid);
    assert_eq!(changed, vec![path.clone()]);

    // Simulate append warn path: files_changed_written=false → restore.
    let files_changed_written = false;
    if !changed.is_empty() && !files_changed_written {
        mgr.restore_changed_files(&sid, &changed);
    }
    assert_eq!(
        mgr.take_changed_files(&sid),
        vec![path],
        "soft restore must preserve notification for next seam"
    );
}

/// T3 — happy path: FilesChanged written → no restore; subsequent take empty.
#[test]
fn t3_successful_files_changed_does_not_restore() {
    let fx = SessionDataFixture::new();
    let (mgr, sid) = manager_with_open_session(&fx);
    let path = scratch_file(&fx, "t3.txt", "v1");
    mgr.note_touched_files(&sid, &[path.clone()]);
    fs::write(&path, "external").expect("mutate");

    let changed = mgr.take_changed_files(&sid);
    assert_eq!(changed, vec![path]);
    let files_changed_written = true; // append Ok(FilesChanged)
    if !changed.is_empty() && !files_changed_written {
        mgr.restore_changed_files(&sid, &changed);
    }
    assert!(
        mgr.take_changed_files(&sid).is_empty(),
        "success path must NOT restore"
    );
}

/// T4 — empty changed: restore no-op; no behavior change.
#[test]
fn t4_empty_changed_skips_restore() {
    let fx = SessionDataFixture::new();
    let (mgr, sid) = manager_with_open_session(&fx);
    let changed = mgr.take_changed_files(&sid);
    assert!(changed.is_empty());
    mgr.restore_changed_files(&sid, &changed);
    assert!(mgr.take_changed_files(&sid).is_empty());
}
