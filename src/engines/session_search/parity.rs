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
            r#"{"file_path":".litecode/sessions/OTHER.md"}"#,
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
        self.mutate(SessionMutation::PersistItem {
            session_id: self.sid.clone(),
            expected_revision: self.rev(),
            operation_id: MutationId::new(),
            item: Item::Message(crate::authority::responses::MessageItem::Output(
                crate::authority::responses::OutputMessage {
                    id: id.into(),
                    role: crate::authority::responses::AssistantRole::Assistant,
                    content: vec![crate::authority::responses::OutputMessageContent::OutputText(
                        crate::authority::responses::OutputTextContent {
                            text: text.into(),
                            annotations: vec![],
                            logprobs: None,
                        },
                    )],
                    status: crate::authority::responses::OutputStatus::InProgress,
                    phase: None,
                },
            )),
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
        arguments: r#"{"file_path":".litecode/sessions/OTHER.md"}"#.into(),
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
    let sid = data.create_session("/p", "default", None).expect("create session");
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
    // then apply them through `refresh_index` — the same reconcile the search path
    // uses.
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

    assert_eq!(actual.0, expected.0, "rows diverged after a partial rebuild");
    assert_eq!(
        actual.1, expected.1,
        "source_state diverged after a partial rebuild"
    );
    assert!(!tail.is_empty());
}

/// The number the whole design hangs on, as an assertion rather than a printout.
///
/// Reconciliation was rebuilt so its cost follows the *change* and not the
/// corpus: a refresh reads key coordinates, diffs them, and derives only what the
/// diff names, while a full rebuild derives and writes every row. On a settled
/// corpus the two are therefore separated by a wide margin, and the margin is the
/// thing worth guarding — because the failure mode is silent. A refresh that
/// quietly went back to deriving the whole corpus would still produce exactly the
/// right index, just slowly, and every other test here would still pass.
///
/// It is a ratio and not a wall-clock budget on purpose. This runs beside the rest
/// of the suite, where absolute milliseconds are noise; the ratio between two
/// operations measured moments apart on the same machine in the same run is not.
/// The margin is generous enough to be safe under load and still settles the
/// question: at forty thousand rows a rebuild measures seconds and a settled
/// refresh measures a fifth of a second.
#[test]
fn a_settled_reconcile_costs_a_fraction_of_a_rebuild() {
    use std::time::Instant;

    let f = open();
    seed_direct(&f, 4_000);
    let root = f.index_root();

    let rows = f
        .data
        .reader()
        .searchable_rows_blocking(None)
        .expect("rows");
    let t = Instant::now();
    sparse::build_index(&rows, &root).expect("build");
    let build_ms = t.elapsed().as_millis().max(1);
    drop(rows);

    // Nothing changed: the reconcile reads keys and derives nothing at all.
    let t = Instant::now();
    let changed = sparse::refresh_from_source(&f.data.reader(), &root).expect("idle refresh");
    let idle_ms = t.elapsed().as_millis().max(1);
    assert_eq!(changed, 0, "an idle refresh changes nothing");

    // One new row: the cost is one row, not the corpus.
    append_one(&f, "a new row entirely UNIQUE_DELTA");
    let t = Instant::now();
    let changed = sparse::refresh_from_source(&f.data.reader(), &root).expect("delta refresh");
    let delta_ms = t.elapsed().as_millis().max(1);
    assert_eq!(changed, 1, "exactly the new row moved");

    for (label, ms) in [("idle", idle_ms), ("delta", delta_ms)] {
        assert!(
            ms * 8 <= build_ms,
            "a {label} reconcile must be a fraction of a rebuild, not a rebuild:              {label}={ms}ms build={build_ms}ms — the delta path has regressed into              touching the whole corpus"
        );
    }
}

/// Seed settled rows straight into the store. This is a cost measurement, not a
/// correctness test, so it does not need the write path.
fn seed_direct(f: &Fixture, rows: usize) {
    let conn = rusqlite::Connection::open(f.dir.path().join("sessions.db")).expect("db");
    let tx = conn.unchecked_transaction().expect("tx");
    {
        let mut ins = tx
            .prepare(
                "INSERT INTO transcript_items
                    (session_id, seq, turn_id, turn_seq, item_type, kind, body,
                     token_estimate, created_at, event_type, surface_op, state)
                 VALUES (?1, ?2, 't', ?2, 'message', 'item/user', ?3, 1, 1, 'message', 'append', 'final')",
            )
            .expect("prepare");
        for seq in 0..rows {
            let item = crate::types::user_text(&format!(
                "seeded prose number {seq} with a few words for the tokenizer to chew on"
            ));
            let body = serde_json::to_string(&item).expect("serialize");
            ins.execute(rusqlite::params![f.sid, seq as i64, body])
                .expect("insert");
        }
    }
    tx.commit().expect("commit");
}

fn append_one(f: &Fixture, text: &str) {
    let conn = rusqlite::Connection::open(f.dir.path().join("sessions.db")).expect("db");
    let seq: i64 = conn
        .query_row(
            "SELECT ifnull(MAX(seq) + 1, 0) FROM transcript_items WHERE session_id = ?1",
            rusqlite::params![f.sid],
            |r| r.get(0),
        )
        .expect("next seq");
    let body = serde_json::to_string(&user_text(text)).expect("serialize");
    conn.execute(
        "INSERT INTO transcript_items
            (session_id, seq, turn_id, turn_seq, item_type, kind, body,
             token_estimate, created_at, event_type, surface_op, state)
         VALUES (?1, ?2, 't', ?2, 'message', 'item/user', ?3, 1, 1, 'message', 'append', 'final')",
        rusqlite::params![f.sid, seq, body],
    )
    .expect("append");
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
        op: crate::session::data::sqlite::session::SessionApply::Truncate { user_k: 2 },
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

/// A refresh that fails leaves the index exactly as it was.
///
/// The rows are written in a single transaction, so a failure rolls back whole.
/// The observable form is what matters: a consumer that half-applied a diff would
/// declare rows accounted for while never projecting them.
#[test]
fn a_failed_refresh_leaves_the_index_unchanged() {
    let f = open();
    let root = f.index_root();
    f.insert(vec![user_text("good content UNIQUE_A")], "t1");
    f.build();
    let before_rows = f.indexed_rows();

    // A final row whose body cannot be read: it is searchable in shape, so the
    // reconcile schedules it, and then the derivation refuses to produce it.
    let conn = Connection::open(f.dir.path().join("sessions.db")).expect("db");
    conn.execute(
        "INSERT INTO transcript_items
            (session_id, seq, turn_id, turn_seq, item_type, kind, body,
             token_estimate, created_at, event_type, surface_op, state)
         VALUES (?1, 99, 't', 0, 'message', 'item/user', '{{{ not an item',
                 1, 1, 'message', 'append', 'final')",
        rusqlite::params![f.sid],
    )
    .expect("plant an unreadable final row");
    drop(conn);

    assert!(
        sparse::refresh_from_source(&f.data.reader(), &root).is_err(),
        "an unreadable row must fail the refresh"
    );
    assert_eq!(
        f.indexed_rows(),
        before_rows,
        "and change nothing on the way out"
    );
}

/// A rebuild is published whole or not at all.
///
/// The old builder deleted the file and rebuilt into the gap, so a failure left
/// no index at all — and a rebuild is exactly when somebody is likely to be
/// searching. Now the corpus is derived before the file is touched and the write
/// is a single transaction, so a failure leaves the previous index live.
#[test]
fn a_failed_rebuild_leaves_the_previous_index_live() {
    let f = open();
    let root = f.index_root();
    f.insert(vec![user_text("the good corpus UNIQUE_KEEP")], "t1");
    f.build();
    let before = f.indexed_rows();
    assert!(before.iter().any(|r| r.contains("UNIQUE_KEEP")));

    // One row that cannot be read is enough to fail the whole build.
    let poisoned = vec![SearchableRow {
        session_id: f.sid.clone(),
        seq: 99,
        kind: "item/user".into(),
        item_type: "message".into(),
        body: Some("{{{ this is not an item".into()),
        body_ref: None,
    }];
    assert!(
        sparse::build_index(&poisoned, &root).is_err(),
        "an unreadable row must fail the rebuild"
    );

    assert_eq!(
        f.indexed_rows(),
        before,
        "the previous index is untouched, down to the row"
    );
    assert!(
        !sparse::needs_rebuild(&sparse::sparse_index_path(&root)).expect("probe"),
        "and what is on disk is still a finished index, not a half-written file"
    );
}

/// A rebuild makes the index equal the corpus. It is not a merge: a row that has
/// left the settled set is gone afterwards, and a file left behind by an older
/// schema is replaced rather than written into.
#[test]
fn a_rebuild_replaces_the_whole_index() {
    let f = open();
    let root = f.index_root();
    f.insert(
        vec![user_text("first UNIQUE_ONE"), user_text("second UNIQUE_TWO")],
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

/// A refresh over a session with an in-progress row indexes the final rows and
/// leaves only the in-progress row out.
#[test]
fn a_refresh_indexes_final_rows_even_beside_an_in_flight_one() {
    let f = open();
    f.insert(vec![user_text("settled history")], "t1");
    f.stream("asst_live", "stuck");
    f.insert(vec![user_text("later final turn")], "t2");
    f.build();

    let changed = sparse::refresh_from_source(&f.data.reader(), &f.index_root())
        .expect("refresh over an in-progress row still succeeds");
    assert_eq!(changed, 0, "the final rows were already indexed");

    let rows = f.indexed_rows();
    assert!(rows.iter().any(|r| r.contains("settled history")));
    assert!(rows.iter().any(|r| r.contains("later final turn")));
    assert!(!rows.iter().any(|r| r.contains("stuck")));
}

/// Eligibility is per row, not per prefix: an in-progress row is not searchable,
/// but a final row after it is.
///
/// The session layer writes a row final only when its content can no longer
/// change, so a later final row does not have to wait for an earlier in-progress
/// one to settle.
#[test]
fn an_in_flight_row_does_not_hold_back_a_later_final_row() {
    let f = open();
    f.insert(vec![user_text("settled history")], "t1");

    f.stream("asst_live", "half a thought");
    // Final, and *after* the in-progress row: searchable on its own.
    f.insert(vec![user_text("second turn")], "t2");

    let keys = f.data.reader().searchable_keys_blocking(None).unwrap();
    assert_eq!(
        keys.len(),
        2,
        "only the in-progress row is excluded, got {keys:?}"
    );
    assert!(keys.iter().any(|(_, seq)| *seq == 0));
    assert!(keys.iter().any(|(_, seq)| *seq == 2));
    assert!(!keys.iter().any(|(_, seq)| *seq == 1));

    f.seal();

    let keys = f.data.reader().searchable_keys_blocking(None).unwrap();
    assert_eq!(
        keys.len(),
        3,
        "sealing admits the in-progress row, got {keys:?}"
    );
}

/// The echo rule, through the real store rather than a hand-made row.
#[test]
fn a_session_echo_result_never_reaches_the_index() {
    let f = open();
    f.insert(
        vec![
            read_session_call("c1"),
            tool_result("c1", "ECHOED PAGE UNIQUE_COPY"),
            user_text("ordinary prose UNIQUE_KEEP"),
        ],
        "t1",
    );
    f.build();

    let rows = f.indexed_rows();
    assert!(
        rows.iter().any(|r| r.contains("UNIQUE_KEEP")),
        "ordinary prose is indexed: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("UNIQUE_COPY")),
        "a copy of another session's transcript must not be searchable: {rows:?}"
    );
    let state = f.indexed_state();
    let call = state
        .iter()
        .find(|r| r.contains("|item/tool_call|"))
        .expect("the call row is tracked");
    assert!(
        call.contains("|c1|1|1"),
        "the call is recorded as a session read that contributes chunks: {call}"
    );
    let result = state
        .iter()
        .find(|r| r.contains("|item/tool_result|"))
        .expect("the result row is tracked");
    assert!(
        result.contains("|c1|0|0"),
        "the result keeps its linkage but is not admitted: {result}"
    );
}

/// A revert takes rows out of the settled set; the index must follow them out.
#[test]
fn a_reverted_tail_leaves_the_index() {
    let f = open();
    f.insert(vec![user_text("keep this UNIQUE_KEEP")], "t1");
    f.insert(vec![user_text("drop this UNIQUE_DROP")], "t2");
    f.build();
    assert!(f.indexed_rows().iter().any(|r| r.contains("UNIQUE_DROP")));

    f.mutate(SessionMutation::Apply {
        session_id: f.sid.clone(),
        expected_revision: f.rev(),
        operation_id: MutationId::new(),
        op: crate::session::data::sqlite::session::SessionApply::Truncate { user_k: 1 },
    });

    sparse::refresh_from_source(&f.data.reader(), &f.index_root()).expect("refresh");
    let rows = f.indexed_rows();
    assert!(
        rows.iter().any(|r| r.contains("UNIQUE_KEEP")),
        "the anchor survived: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("UNIQUE_DROP")),
        "a reverted row must leave the index: {rows:?}"
    );
}

