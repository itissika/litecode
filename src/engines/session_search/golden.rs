//! Stage A golden: the sparse lane's **derived artifact** and **query
//! contract**, pinned against one fixed corpus.
//!
//! This is the regression gate the lifecycle remediation (stages B–G) must keep
//! green. It freezes, in one place:
//!
//! * which source rows enter the index — `compacted` rows and session-echo
//!   tool results are dropped, everything else searchable is indexed;
//! * the chunk grid — `sid:seq` for a row that fits one chunk, `sid:seq#k`
//!   otherwise, tiling the row losslessly;
//! * the normalized text stored next to the original chunk text;
//! * the query ladder — exact substring 1.0, CJK substring via the trigram
//!   table, word-token match, and the exclusions that must stay unsearchable.
//!
//! The corpus uses synthetic session ids (`S1`, `S2`) and calls
//! `sparse::build_index` directly, so nothing here depends on the store's ULID
//! generation or on wall-clock ordering.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
};
use crate::session::transcript_file::SearchableRow;
use crate::types::{Item, assistant_text, user_text};

use super::sparse::{self, Lane};
use super::tokenizer;

/// One `SearchableRow` from an `Item`, with `item_type` taken from the serde
/// `type` tag (the same string the index stores and the hit reports).
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

/// The fixed corpus. Every row is deterministic and addressed by seq.
fn corpus() -> Vec<SearchableRow> {
    let long: String = (0..300)
        .map(|i| format!("segment {i}: the quick brown fox jumps over the lazy dog.\n"))
        .collect();
    vec![
        row("S1", 0, "item/user", &user_text("alpha beta gamma")),
        row(
            "S1",
            1,
            "item/assistant",
            &assistant_text("the AuthRefactorToken decision is final"),
        ),
        // A `read` aimed at the session store: its result is a session echo.
        function_call(
            "S1",
            2,
            "c1",
            "read",
            r#"{"file_path":".litecode/sessions/OTHER.md"}"#,
        ),
        function_result("S1", 3, "c1", "echoed page UNIQUE_ECHO"),
        // An ordinary tool pair: both rows stay searchable.
        function_call("S1", 4, "c2", "bash", r#"{"cmd":"grep NEEDLE_TOOL"}"#),
        function_result("S1", 5, "c2", "normal output NEEDLE_TOOL"),
        // CJK prose, then a compacted summary (dropped), then a long row.
        row(
            "S2",
            0,
            "item/user",
            &user_text("这是一个中文段落，包含唯一标记：会话检索测试"),
        ),
        row("S2", 1, "compacted", &user_text("COMPACTED_SUMMARY_UNIQUE")),
        row("S2", 2, "item/user", &user_text(long)),
    ]
}

fn build(dir: &Path) -> Vec<SearchableRow> {
    let rows = corpus();
    sparse::build_index(&rows, dir).expect("build index");
    rows
}

fn open(dir: &Path) -> Connection {
    Connection::open(sparse::sparse_index_path(dir)).expect("open sparse db")
}

/// `(session_id, seq, chunk, single)` for every indexed chunk, ordered.
fn chunk_keys(dir: &Path) -> Vec<(String, i64, i64, i64)> {
    let conn = open(dir);
    let mut stmt = conn
        .prepare("SELECT session_id, seq, chunk, single FROM rows ORDER BY session_id, seq, chunk")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

#[test]
fn golden_indexes_exactly_the_admitted_rows() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());

    let indexed: BTreeSet<(String, i64)> = chunk_keys(dir.path())
        .into_iter()
        .map(|(sid, seq, _, _)| (sid, seq))
        .collect();

    let expected: BTreeSet<(String, i64)> = [
        ("S1", 0),
        ("S1", 1),
        ("S1", 2),
        ("S1", 4),
        ("S1", 5),
        ("S2", 0),
        ("S2", 2),
    ]
    .into_iter()
    .map(|(s, q)| (s.to_string(), q))
    .collect();

    assert_eq!(
        indexed, expected,
        "admission changed: echo results and compacted rows must stay out"
    );
    // The two excluded rows explicitly, so a future failure names them.
    assert!(!indexed.contains(&("S1".into(), 3)), "session-echo result indexed");
    assert!(!indexed.contains(&("S2".into(), 1)), "compacted row indexed");
}

#[test]
fn golden_chunk_keys_and_normalized_text() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());

    let conn = open(dir.path());
    let norm = |sid: &str, seq: i64| -> String {
        conn.query_row(
            "SELECT text_norm FROM rows WHERE session_id = ?1 AND seq = ?2 AND chunk = 0",
            rusqlite::params![sid, seq],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(norm("S1", 0), sparse::normalize("alpha beta gamma"));
    assert_eq!(
        norm("S1", 1),
        sparse::normalize("the AuthRefactorToken decision is final")
    );
    // The tool call row's plain text is `name(arguments)`.
    assert_eq!(
        norm("S1", 4),
        sparse::normalize(r#"bash({"cmd":"grep NEEDLE_TOOL"})"#)
    );

    // Every indexed row that is not the long row is a single chunk.
    let singles: BTreeSet<(String, i64)> = chunk_keys(dir.path())
        .into_iter()
        .filter(|(_, _, _, single)| *single == 1)
        .map(|(sid, seq, _, _)| (sid, seq))
        .collect();
    assert!(singles.contains(&("S1".into(), 0)));
    assert!(singles.contains(&("S2".into(), 0)));
    assert!(!singles.contains(&("S2".into(), 2)), "long row must split");
}

#[test]
fn golden_long_row_tiles_losslessly_within_budget() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());

    let conn = open(dir.path());
    let mut stmt = conn
        .prepare(
            "SELECT chunk, char_start, char_end, text FROM rows
              WHERE session_id = 'S2' AND seq = 2 ORDER BY chunk",
        )
        .unwrap();
    let chunks: Vec<(i64, i64, i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();

    assert!(chunks.len() >= 2, "long row must be cut into tiles");
    let tk = tokenizer::open().unwrap();
    let mut cursor = 0i64;
    let mut rebuilt = String::new();
    for (index, start, end, text) in &chunks {
        assert_eq!(*start, cursor, "chunk {index} must start where the previous ended");
        assert!(*end > *start, "chunk {index} must make progress");
        assert!(
            tokenizer::token_len(&tk, text) <= 448,
            "chunk {index} exceeds the 448-token budget"
        );
        cursor = *end;
        rebuilt.push_str(text);
    }
    // Lossless: the concatenated chunks equal the trimmed plain text.
    let rows = corpus();
    let source = crate::session::transcript_file::row_plain_text(&rows[8], dir.path())
        .unwrap()
        .unwrap();
    assert_eq!(rebuilt, source.trim(), "chunks must tile the source losslessly");
}

#[test]
fn golden_query_ladder_and_exclusions() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());
    let index = index(dir.path());

    // Exact Latin substring: LIKE tier, score 1.0, one row.
    let hits = index.search(Lane::Final, "AuthRefactorToken", 50).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].row_key, "S1:1");
    assert_eq!(hits[0].score, 1.0);
    // The span lands on the literal.
    let text = "the AuthRefactorToken decision is final";
    let span: String = text
        .chars()
        .skip(hits[0].char_start)
        .take(hits[0].char_end - hits[0].char_start)
        .collect();
    assert_eq!(span, "AuthRefactorToken");

    // The literal appears in both the call and the result row.
    let hits = index.search(Lane::Like, "NEEDLE_TOOL", 50).unwrap();
    let keys: BTreeSet<String> = hits.iter().map(|h| h.row_key.clone()).collect();
    assert_eq!(
        keys,
        ["S1:4".to_string(), "S1:5".to_string()].into_iter().collect()
    );

    // CJK substring is answerable by both the LIKE path and the trigram table.
    assert!(
        index
            .search(Lane::Like, "会话检索测试", 50)
            .unwrap()
            .iter()
            .any(|h| h.row_key == "S2:0"),
        "CJK substring must hit via LIKE"
    );
    assert!(
        index
            .search(Lane::Trigram, "会话检索", 50)
            .unwrap()
            .iter()
            .any(|h| h.row_key == "S2:0"),
        "CJK substring must be in the trigram candidate set"
    );

    // The echo result and the compacted summary are absent from the exact
    // (LIKE) index, and no lane may surface them by key — the fuzzy n-gram
    // fallback is allowed to match other rows, but never an excluded one.
    assert!(
        index.search(Lane::Like, "UNIQUE_ECHO", 50).unwrap().is_empty(),
        "session-echo result must not be indexed"
    );
    assert!(
        index
            .search(Lane::Like, "COMPACTED_SUMMARY_UNIQUE", 50)
            .unwrap()
            .is_empty(),
        "compacted summary must not be indexed"
    );
    for query in ["UNIQUE_ECHO", "COMPACTED_SUMMARY_UNIQUE"] {
        for lane in [Lane::Final, Lane::Hybrid, Lane::Recipe] {
            let keys: BTreeSet<String> = index
                .search(lane, query, 50)
                .unwrap()
                .into_iter()
                .map(|h| h.row_key)
                .collect();
            assert!(!keys.contains("S1:3"), "{lane:?} surfaced the echo result");
            assert!(!keys.contains("S2:1"), "{lane:?} surfaced the compacted row");
        }
    }
}

#[test]
fn golden_scope_and_candidate_recall() {
    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());

    // Scope is applied inside the SQL: another session's rows never surface.
    let scoped = sparse::open_read_only(&sparse::sparse_index_path(dir.path()))
        .unwrap()
        .with_scope(Some("S2"));
    let hits = scoped.search(Lane::Final, "NEEDLE_TOOL", 50).unwrap();
    assert!(hits.is_empty(), "S2 must not see S1's tool rows");

    // Candidate recall: a truth row is in the candidate set even below top-k.
    let index = sparse::open_read_only(&sparse::sparse_index_path(dir.path())).unwrap();
    assert!(index.contains(Lane::Final, "AuthRefactorToken", "S1", 1).unwrap());
    assert!(!index.contains(Lane::Final, "AuthRefactorToken", "S1", 0).unwrap());
}

fn index(dir: &Path) -> sparse::SparseIndex {
    sparse::open_read_only(&sparse::sparse_index_path(dir)).expect("open index")
}

/// The product layer's contract, on the same fixed corpus: a query that shares
/// only part of its words must not return the row that has the other part, and
/// every hit must say which mechanism found it.
#[test]
fn golden_final_layer_gates_on_intent_and_reports_its_evidence() {
    use super::ranking::{LayerId, RankBand};

    let dir = tempfile::TempDir::new().unwrap();
    build(dir.path());
    let index = index(dir.path());

    // The literal: exact evidence, and the strongest band there is.
    let hits = index.search(Lane::Final, "AuthRefactorToken", 50).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].row_key, "S1:1");
    assert_eq!(hits[0].rank.band, RankBand::Exact);
    assert!(hits[0].layers().contains(&LayerId::Exact), "{hits:#?}");

    // Two words are an AND, even when one of them is an identifier: the row is
    // the same, and the second word is not there, so nothing answers.
    assert!(
        index
            .search(Lane::Final, "authrefactortoken missingword", 50)
            .unwrap()
            .is_empty(),
        "a query must not be answered by half of itself"
    );

    // The same words, out of the literal's order: proximity answers, and the
    // word layer agrees.
    let hits = index
        .search(Lane::Final, "authrefactortoken final", 50)
        .unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].row_key, "S1:1");
    assert_eq!(hits[0].rank.band, RankBand::Proximity);
    assert!(hits[0].layers().contains(&LayerId::Proximity), "{hits:#?}");
    assert_eq!(hits[0].coverage_of(LayerId::Lexical), Some(1.0));

    // A scope only narrows: the same row, from a session-scoped index.
    let scoped = sparse::open_read_only(&sparse::sparse_index_path(dir.path()))
        .unwrap()
        .with_scope(Some("S2"));
    assert!(
        scoped
            .search(Lane::Final, "authrefactortoken", 50)
            .unwrap()
            .is_empty()
    );
}
