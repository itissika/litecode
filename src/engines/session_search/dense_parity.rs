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

use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
};
use crate::engines::code_search::{Embedder, HashEmbedder};
use crate::session::data::command::{MutationId, SessionMutation};
use crate::session::data::sqlite::session::SessionApply;
use crate::session::{SessionData, SessionDataReader, WorkspaceWriteLease};
use crate::types::{Item, Result, user_text};

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
        (
            read_lines(&self.index_dir().join("chunks.jsonl"), true),
            read_lines(&self.index_dir().join("source_state.jsonl"), false),
        )
    }
}

/// One artifact file as sorted lines. `strip_id` removes the vector id, which
/// differs between two equally correct builds (it counts allocations, not rows).
fn read_lines(path: &Path, strip_id: bool) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut lines: Vec<String> = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            if !strip_id {
                return line.to_string();
            }
            let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
            value.as_object_mut().unwrap().remove("id");
            serde_json::to_string(&value).unwrap()
        })
        .collect();
    lines.sort();
    lines
}

fn session_search_call(call_id: &str) -> Item {
    Item::FunctionCall(FunctionToolCall {
        arguments: r#"{"query":"auth"}"#.into(),
        call_id: call_id.into(),
        namespace: None,
        name: "session_search".into(),
        id: None,
        status: None,
    })
}

fn call_result(call_id: &str, output: &str) -> Item {
    Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: call_id.into(),
        output: FunctionCallOutput::Text(output.into()),
        id: None,
        status: None,
    })
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

#[test]
fn a_reverted_dense_index_equals_a_clean_rebuild() {
    let f = open();
    f.insert(&[
        user_text("first turn"),
        user_text("second turn"),
        user_text("third turn"),
    ]);
    f.reconcile();

    let revision = f.data.revision_blocking(&f.sid).unwrap();
    f.data
        .mutate_blocking(SessionMutation::Apply {
            session_id: f.sid.clone(),
            expected_revision: revision,
            operation_id: MutationId::new(),
            op: SessionApply::Truncate { user_k: 1 },
        })
        .unwrap();
    let reverted = f.reconcile();
    assert_eq!(reverted.len(), 1, "the revert left one row");

    let (chunks, settled) = f.artifacts();
    assert_eq!(settled.len(), 1, "the removed rows left the settled state");
    assert_eq!(f.rebuild(), (chunks, settled));
}

/// The echo closure has to survive a batch boundary: the result arrives after
/// its call was settled, so only the persisted linkage can exclude it.
#[test]
fn a_late_echo_result_is_excluded_by_the_settled_seed() {
    let f = open();
    f.insert(&[user_text("hello"), session_search_call("call_1")]);
    f.reconcile();

    f.insert(&[call_result("call_1", "a copy of another transcript")]);
    let incremental = f.reconcile();
    assert_eq!(incremental.len(), 2, "the call is content, the copy is not");

    let (chunks, settled) = f.artifacts();
    assert_eq!(chunks.len(), 2, "the late result produced no document");
    assert!(
        !chunks.iter().any(|line| line.contains("a copy of another")),
        "an echo copy must never be searchable: {chunks:?}"
    );
    assert_eq!(f.rebuild(), (chunks, settled));
}

/// A hash embedder that counts what was actually embedded.
#[derive(Default)]
struct Counting {
    embedded: usize,
}

impl Embedder for Counting {
    fn embedder_id(&self) -> &'static str {
        HashEmbedder.embedder_id()
    }

    fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embedded += texts.len();
        HashEmbedder.embed_batch(texts)
    }
}

/// An index written before the settled state existed has no diff basis: its
/// first pass reads the corpus once, and it must reuse every vector it already
/// holds — an upgrade that re-embeds the corpus is an upgrade nobody can afford.
#[test]
fn a_legacy_index_migrates_in_one_pass_without_re_embedding() {
    let f = open();
    f.insert(&[user_text("alpha row"), user_text("beta row")]);
    f.reconcile();

    // Drop the state, leaving exactly what the previous version wrote.
    std::fs::remove_file(f.index_dir().join("source_state.jsonl")).unwrap();
    f.insert(&[user_text("gamma row")]);

    let mut emb = Counting::default();
    let index = ensure_session_index(f.root.path(), &f.reader(), &mut emb).unwrap();
    assert_eq!(index.len(), 3);
    assert_eq!(
        emb.embedded, 1,
        "only the new row may be embedded; the corpus is reused"
    );

    let (chunks, settled) = f.artifacts();
    assert_eq!(settled.len(), 3, "the migrated state covers every row");
    assert_eq!(f.rebuild(), (chunks, settled));
}
