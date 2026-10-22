//! Equivalence gate: **a reconciled index must equal a clean rebuild**.
//!
//! The index is never told what changed. Every prepare diffs the final source
//! key set against what the index holds and repairs the difference, and a clean
//! rebuild derives the same rows from the same corpus. The moment those two
//! paths can disagree, a missed diff silently corrupts the index and the only
//! way back is a full rebuild — so the equivalence is pinned here, field by
//! field, on the derived artifact itself (`rows` and `source_state`).
//!
//! Both sides go through the same `derive_rows`; what differs is only *how much*
//! of the corpus each call sees. That is the real risk: a row that is skipped as
//! "unchanged" must be skipped because its derived bytes matched, never because
//! the incremental path forgot it existed. Revert, delete-tail, settle, and
//! mid-flight rows all land in this file for that reason.

use std::path::Path;

use rusqlite::Connection;

use crate::session::data::{SessionData, SessionMutation};
use crate::session::{MutationId, WorkspaceWriteLease};
use crate::types::{Item, user_text};

use super::sparse;





struct Fixture {
    dir: tempfile::TempDir,
    data: std::sync::Arc<SessionData>,
    sid: String,
    _lease: WorkspaceWriteLease,
}

impl Fixture {
    fn mutate(&self, m: SessionMutation) {
        self.data.mutate_blocking(m).expect("mutation");
    }

    /// Mutations must declare the session's current revision; these tests do not
    /// care what it is.
    fn rev(&self) -> u64 {
        rusqlite::Connection::open(self.dir.path().join("sessions.db"))
            .expect("reopen")
            .query_row(
                "SELECT revision FROM sessions WHERE id = ?1",
                rusqlite::params![self.sid],
                |r| r.get::<_, i64>(0),
            )
            .expect("revision") as u64
    }

    fn insert(&self, items: Vec<Item>, turn: &str) {
        self.mutate(SessionMutation::InsertDetails {
            session_id: self.sid.clone(),
            expected_revision: self.rev(),
            operation_id: MutationId::new(),
            items,
            turn_id: turn.into(),
        });
    }



    fn index_root(&self) -> std::path::PathBuf {
        let root = self.dir.path().join("idx");
        std::fs::create_dir_all(&root).expect("index dir");
        root
    }


    fn indexed_rows(&self) -> Vec<String> {
        snapshot(&self.index_root()).0
    }

    fn indexed_state(&self) -> Vec<String> {
        snapshot(&self.index_root()).1
    }
}



fn open() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).expect("open");
    let sid = data
        .create_session("/p", "default", None)
        .expect("create session");
    Fixture {
        dir,
        data,
        sid,
        _lease: lease,
    }
}


fn dump(path: &Path, sql: &str) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }
    let conn = Connection::open(path).expect("open index");
    let mut stmt = conn.prepare(sql).expect("prepare");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect")
}

/// Every derived field, in a stable order — the comparison unit for both sides.
fn snapshot(data_root: &Path) -> (Vec<String>, Vec<String>) {
    let path = sparse::sparse_index_path(data_root);
    let rows = dump(
        &path,
        "SELECT session_id || '|' || seq || '|' || chunk || '|' || single || '|'
              || item_type || '|' || char_start || '|' || char_end || '|' || text_norm
              || '|' || text
         FROM rows ORDER BY session_id, seq, chunk",
    );
    let state = dump(
        &path,
        "SELECT session_id || '|' || seq || '|' || kind || '|' || item_type || '|'
              || ifnull(call_id,'-') || '|'
              || ifnull(session_read_call,'-') || '|' || in_chunks
         FROM source_state ORDER BY session_id, seq",
    );
    (rows, state)
}




/// A rebuild makes the index equal the corpus. It is not a merge: a row that has
/// left the settled set is gone afterwards, and a file left behind by an older
/// schema is replaced rather than written into.
#[test]
fn a_rebuild_replaces_the_whole_index() {
    let f = open();
    let root = f.index_root();
    f.insert(
        vec![
            user_text("first UNIQUE_ONE"),
            user_text("second UNIQUE_TWO"),
        ],
        "t1",
    );
    let rows = f
        .data
        .reader()
        .searchable_rows_blocking(None)
        .expect("rows");
    sparse::build_index(&rows, &root).expect("build");
    assert!(f.indexed_rows().iter().any(|r| r.contains("UNIQUE_TWO")));

    // Plant evidence of a build that this corpus does not contain, and claim an
    // older schema: a rebuild must not leave either of them behind.
    let conn = Connection::open(sparse::sparse_index_path(&root)).expect("index");
    conn.execute(
        "INSERT INTO rows(session_id, seq, chunk, single, item_type,
                          char_start, char_end, text_norm, text)
         VALUES ('ghost', 0, 0, 1, 'message', 0, 6, 'ghosted', 'ghosted')",
        [],
    )
    .expect("plant chunk");
    conn.execute(
        "INSERT INTO source_state(session_id, seq, kind, item_type, in_chunks)
         VALUES ('ghost', 0, 'item/user', 'message', 1)",
        [],
    )
    .expect("plant state");
    conn.execute("UPDATE meta SET value = '1' WHERE key = 'schema'", [])
        .expect("lie about schema");
    drop(conn);

    // Rebuild from only the first row.
    sparse::build_index(&rows[..1], &root).expect("rebuild");

    let rows_after = f.indexed_rows();
    assert!(rows_after.iter().any(|r| r.contains("UNIQUE_ONE")));
    assert!(
        !rows_after.iter().any(|r| r.contains("UNIQUE_TWO")),
        "a row outside the new corpus must be gone: {rows_after:?}"
    );
    assert!(
        !rows_after.iter().any(|r| r.contains("ghosted")),
        "and so must anything planted by another build: {rows_after:?}"
    );
    assert!(
        f.indexed_state().iter().all(|r| !r.contains("ghost")),
        "including its tracked state"
    );
}
