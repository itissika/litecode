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

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
};
use crate::session::data::{SessionData, SessionMutation};
use crate::session::transcript_file::SearchableRow;
use crate::session::{MutationId, WorkspaceWriteLease};
use crate::types::{Item, assistant_text, user_text};

use super::sparse;

fn row(sid: &str, seq: i64, kind: &str, item: &Item) -> SearchableRow {
    let value = serde_json::to_value(item).expect("serialize item");
    let item_type = value
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown")
        .to_string();
    SearchableRow {
        session_id: sid.into(),
        seq,
        kind: kind.into(),
        item_type,
        body: Some(value.to_string()),
        body_ref: None,
    }
}

fn function_call(sid: &str, seq: i64, call_id: &str, name: &str, arguments: &str) -> SearchableRow {
    row(
        sid,
        seq,
        "item/tool_call",
        &Item::FunctionCall(FunctionToolCall {
            arguments: arguments.into(),
            call_id: call_id.into(),
            namespace: None,
            name: name.into(),
            id: None,
            status: None,
        }),
    )
}

fn function_result(sid: &str, seq: i64, call_id: &str, output: &str) -> SearchableRow {
    row(
        sid,
        seq,
        "item/tool_result",
        &Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.into(),
            output: FunctionCallOutput::Text(output.into()),
            id: None,
            status: None,
        }),
    )
}

/// Rows the index must treat specially: a compacted summary (dropped), a session
/// echo (dropped), CJK prose, and a row long enough to spill into chunks.
fn tricky_rows() -> Vec<SearchableRow> {
    let long: String = (0..300)
        .map(|i| format!("segment {i}: the quick brown fox jumps over the lazy dog.\n"))
        .collect();
    vec![
        row("P1", 0, "compacted", &user_text("COMPACTED_SUMMARY_UNIQUE")),
        function_call(
            "P1",
            1,
            "p1c",
            "read",
            r#"{"path":".litecode/sessions/OTHER.md"}"#,
        ),
        function_result("P1", 2, "p1c", "echoed page UNIQUE_ECHO"),
        row(
            "P2",
            0,
            "item/user",
            &user_text("这是一个中文段落，包含唯一标记：会话检索测试"),
        ),
        row("P2", 1, "item/user", &user_text(long)),
    ]
}

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

    fn stream(&self, id: &str, text: &str) {
        self.mutate(SessionMutation::BeginStreamItem {
            session_id: self.sid.clone(),
            expected_revision: self.rev(),
            operation_id: MutationId::new(),
            item: Item::Message(crate::authority::responses::MessageItem::Output(
                crate::authority::responses::OutputMessage {
                    id: id.into(),
                    role: crate::authority::responses::AssistantRole::Assistant,
                    content: vec![
                        crate::authority::responses::OutputMessageContent::OutputText(
                            crate::authority::responses::OutputTextContent {
                                text: text.into(),
                                annotations: vec![],
                                logprobs: None,
                            },
                        ),
                    ],
                    status: crate::authority::responses::OutputStatus::InProgress,
                    phase: None,
                },
            )),
            turn_id: "t-stream".into(),
        });
    }

    fn seal(&self) {
        self.mutate(SessionMutation::SealInProgress {
            session_id: self.sid.clone(),
            expected_revision: self.rev(),
            operation_id: MutationId::new(),
        });
    }

    fn index_root(&self) -> std::path::PathBuf {
        let root = self.dir.path().join("idx");
        std::fs::create_dir_all(&root).expect("index dir");
        root
    }

    /// A full build the way the search path does it.
    fn build(&self) {
        let root = self.index_root();
        let rows = self
            .data
            .reader()
            .searchable_rows_blocking(None)
            .expect("rows");
        sparse::build_index(&rows, &root).expect("build");
    }

    fn indexed_rows(&self) -> Vec<String> {
        snapshot(&self.index_root()).0
    }

    fn indexed_state(&self) -> Vec<String> {
        snapshot(&self.index_root()).1
    }
}

fn read_session_call(call_id: &str) -> Item {
    Item::FunctionCall(FunctionToolCall {
        arguments: r#"{"path":".litecode/sessions/OTHER.md"}"#.into(),
        call_id: call_id.into(),
        namespace: None,
        name: "read".into(),
        id: None,
        status: None,
    })
}

fn tool_result(call_id: &str, output: &str) -> Item {
    Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: call_id.into(),
        output: FunctionCallOutput::Text(output.into()),
        id: None,
        status: None,
    })
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

/// Real stored rows, so the parity check exercises blobs, echo detection and the
/// compaction pointer rather than only hand-written rows.
fn corpus(f: &Fixture) -> Vec<SearchableRow> {
    f.data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: f.sid.clone(),
            expected_revision: f.rev(),
            operation_id: MutationId::new(),
            items: vec![
                user_text("alpha beta gamma"),
                assistant_text("the AuthRefactorToken decision is final"),
                Item::FunctionCall(FunctionToolCall {
                    arguments: r#"{"cmd":"grep NEEDLE_TOOL"}"#.into(),
                    call_id: "c2".into(),
                    namespace: None,
                    name: "bash".into(),
                    id: None,
                    status: None,
                }),
                Item::FunctionCallOutput(FunctionCallOutputItemParam {
                    call_id: "c2".into(),
                    output: FunctionCallOutput::Text("normal output NEEDLE_TOOL".into()),
                    id: None,
                    status: None,
                }),
            ],
            turn_id: "t1".into(),
        })
        .expect("insert");
    let mut rows = f
        .data
        .reader()
        .searchable_rows_blocking(None)
        .expect("rows");
    rows.extend(tricky_rows());
    rows
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

fn build_fresh(data_root: &Path, rows: &[SearchableRow]) {
    sparse::build_index(rows, data_root).expect("build");
}

#[test]
fn incremental_derive_matches_a_full_build_field_by_field() {
    let f = open();
    let rows = corpus(&f);
    let data_root = f.dir.path().join("full");
    std::fs::create_dir_all(&data_root).expect("dir");

    build_fresh(&data_root, &rows);
    let expected = snapshot(&data_root);
    assert!(
        !expected.0.is_empty(),
        "the corpus must actually produce index rows"
    );

    // The incremental side: start from an index that has never seen these rows,
    // then apply them through `refresh_index` — the same reconcile the background
    // refresh runs for the search path.
    let incr_root = f.dir.path().join("incr");
    std::fs::create_dir_all(&incr_root).expect("dir");
    build_fresh(&incr_root, &[]);
    sparse::refresh_index(&rows, &incr_root).expect("refresh");
    let actual = snapshot(&incr_root);

    assert_eq!(
        actual.0, expected.0,
        "chunk rows differ between a full build and an incremental apply"
    );
    assert_eq!(
        actual.1, expected.1,
        "source_state differs between a full build and an incremental apply"
    );
}

#[test]
fn incremental_derive_from_a_partial_index_matches_a_full_build() {
    let f = open();
    let rows = corpus(&f);
    let (head, tail) = rows.split_at(rows.len() / 2);

    let full_root = f.dir.path().join("full");
    std::fs::create_dir_all(&full_root).expect("dir");
    build_fresh(&full_root, &rows);
    let expected = snapshot(&full_root);

    // Build a prefix, then feed the whole corpus: the unchanged half must be
    // skipped *because its hash matched*, and the new half must land exactly.
    let incr_root = f.dir.path().join("incr");
    std::fs::create_dir_all(&incr_root).expect("dir");
    build_fresh(&incr_root, head);
    sparse::refresh_index(&rows, &incr_root).expect("refresh");
    let actual = snapshot(&incr_root);

    assert_eq!(
        actual.0, expected.0,
        "rows diverged after a partial rebuild"
    );
    assert_eq!(
        actual.1, expected.1,
        "source_state diverged after a partial rebuild"
    );
    assert!(!tail.is_empty());
}

/// The equivalence gate, through the path production actually uses.
///
/// The promise is narrow and total: an index kept current by reconciliation must
/// be the index a clean rebuild would have produced. Not "the same rows have
/// something" — the same chunks, with the same derived metadata, in the same
/// order. Anything less means what search finds depends on how the index happened
/// to get there, which is exactly the class of bug this rewrite exists to remove.
///
/// The two indexes acquire their rows at different times and through different
/// entry points, so nothing about *when* a row was written is part of the
/// comparison. Everything the query path can see is.
#[test]
fn a_reconciled_index_equals_a_clean_rebuild() {
    let f = open();
    let root = f.index_root();
    let refresh = || sparse::refresh_from_source(&f.data.reader(), &root).expect("refresh");

    f.insert(vec![user_text("first turn UNIQUE_A")], "t1");
    f.build();

    f.insert(vec![user_text("second turn UNIQUE_B")], "t2");
    refresh();

    // A row that arrives in flight, then settles: it must not be visible in
    // between, and must appear exactly once afterwards.
    f.stream("asst_live", "half a");
    refresh();

    f.seal();
    refresh();

    // A session read and its result, in the same batch: the result is a copy of
    // another transcript and must never be searchable.
    f.insert(
        vec![
            read_session_call("c1"),
            tool_result("c1", "ECHOED UNIQUE_COPY"),
            user_text("third turn UNIQUE_C"),
        ],
        "t3",
    );
    refresh();

    // A revert reuses seqs: the rows that leave must leave, and the rows that
    // replace them must be derived from scratch.
    f.mutate(SessionMutation::Apply {
        session_id: f.sid.clone(),
        expected_revision: f.rev(),
        operation_id: MutationId::new(),
        op: crate::session::data::sqlite::session::SessionApply::Truncate { anchor_seq: 5 },
    });
    refresh();

    let reconciled = snapshot(&root);

    // The same corpus, built from nothing.
    let fresh_root = f.dir.path().join("fresh");
    std::fs::create_dir_all(&fresh_root).expect("dir");
    let rows = f
        .data
        .reader()
        .searchable_rows_blocking(None)
        .expect("rows");
    sparse::build_index(&rows, &fresh_root).expect("clean rebuild");
    let rebuilt = snapshot(&fresh_root);

    assert_eq!(
        reconciled.1, rebuilt.1,
        "derived state must match a clean rebuild"
    );
    assert_eq!(
        reconciled.0, rebuilt.0,
        "chunk rows must match a clean rebuild"
    );
    assert!(
        reconciled.0.len() > 1,
        "the corpus is not empty, so this is not vacuous: {:?}",
        reconciled.0
    );
    assert!(
        !reconciled.0.iter().any(|r| r.contains("UNIQUE_COPY")),
        "and the echo rule held through every step: {:?}",
        reconciled.0
    );
}
