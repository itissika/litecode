//! Dense-lane equivalence gate: an incrementally reconciled index must equal a
//! clean rebuild of the same final corpus.
//!
//! `parity.rs` pins this property for the sparse lane by comparing its derived
//! tables. The dense lane's artifact is `chunks.jsonl` plus the settled state
//! beside it, so those are what is compared here — normalized for the vector id,
//! which is an allocation counter and not something the corpus derived.
//!
//! The risk this is aimed at: the incremental path may skip a row only because
//! that row was already settled, never because the diff forgot it existed.
//! Append, revert, and a late echo result all land here for that reason.

use std::sync::Arc;

use tempfile::TempDir;

use crate::engines::code_search::HashEmbedder;
use crate::session::{SessionData, SessionDataReader, WorkspaceWriteLease};
use crate::types::{Item, user_text};

use super::{SessionSemanticIndex, ensure_session_index};

struct Fixture {
    root: TempDir,
    data: Arc<SessionData>,
    sid: String,
}

fn open() -> Fixture {
    let root = TempDir::new().unwrap();
    let db = root.path().join(".litecode").join("sessions.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
    let data = SessionData::open(&lease, &db).unwrap();
    let sid = data.create_session("/proj", "default", None).unwrap();
    Fixture { root, data, sid }
}

impl Fixture {
    fn reader(&self) -> SessionDataReader {
        SessionDataReader::open(&self.root.path().join(".litecode").join("sessions.db"))
    }

    fn index_dir(&self) -> std::path::PathBuf {
        self.root.path().join(".litecode").join("session-index")
    }

    fn insert(&self, items: &[Item]) {
        self.data.insert_items(&self.sid, items).unwrap();
    }

    /// Load + reconcile, exactly as the worker does.
    fn reconcile(&self) -> SessionSemanticIndex {
        let mut emb = HashEmbedder;
        ensure_session_index(self.root.path(), &self.reader(), &mut emb).unwrap()
    }

    /// Throw the index away and derive it again from nothing.
    fn rebuild(&self) -> (Vec<String>, Vec<String>) {
        let _ = std::fs::remove_dir_all(self.index_dir());
        self.reconcile();
        self.artifacts()
    }

    fn artifacts(&self) -> (Vec<String>, Vec<String>) {
        // WP-144: the logs are append-only, so compare what the index holds
        // (live chunks + settled rows), not the raw files.
        let (chunks, settled) = SessionSemanticIndex::live_artifact_lines(self.root.path());
        (normalize(chunks, true), normalize(settled, false))
    }
}

/// Sorted lines; `strip_id` removes the vector id, which differs between two
/// equally correct builds (it counts allocations, not rows).
fn normalize(lines: Vec<String>, strip_id: bool) -> Vec<String> {
    let mut lines: Vec<String> = lines
        .into_iter()
        .map(|line| {
            if !strip_id {
                return line;
            }
            let mut value: serde_json::Value = serde_json::from_str(&line).unwrap();
            value.as_object_mut().unwrap().remove("id");
            serde_json::to_string(&value).unwrap()
        })
        .collect();
    lines.sort();
    lines
}

#[test]
fn an_appended_dense_index_equals_a_clean_rebuild() {
    let f = open();
    f.insert(&[user_text("alpha row")]);
    f.reconcile();

    f.insert(&[user_text("beta row"), user_text("gamma row")]);
    let incremental = f.reconcile();
    assert_eq!(incremental.len(), 3, "the delta was derived");

    let (chunks, settled) = f.artifacts();
    assert_eq!(chunks.len(), 3);
    assert_eq!(f.rebuild(), (chunks, settled));
}


