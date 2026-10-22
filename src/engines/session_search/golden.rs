//! Stage A golden: the sparse lane's **derived artifact** and **query
//! contract**, pinned against one fixed corpus.
//!
//! This is the regression gate the lifecycle remediation (stages B–G) must keep
//! green. It freezes, in one place:
//!
//! * which source rows enter the index — sparse whitelist = 三态 + `tool_call`;
//!   `compacted`, session-echo results, and ordinary `tool_result` stay out;
//! * the chunk grid — `sid:seq` for a row that fits one chunk, `sid:seq#k`
//!   otherwise, tiling the row losslessly;
//! * the normalized text stored next to the original chunk text;
//! * the query ladder — exact substring 1.0, CJK substring via the trigram
//!   table, word-token match, and the exclusions that must stay unsearchable.
//!
//! The corpus uses synthetic session ids (`S1`, `S2`) and calls
//! `sparse::build_index` directly, so nothing here depends on the store's ULID
//! generation or on wall-clock ordering.

use std::path::Path;

use rusqlite::Connection;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
};
use crate::session::transcript_file::SearchableRow;
use crate::types::{Item, assistant_text, user_text};

use super::sparse::{self};
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
            r#"{"path":".litecode/sessions/OTHER.md"}"#,
        ),
        function_result("S1", 3, "c1", "echoed page UNIQUE_ECHO"),
        // Ordinary tool pair: call stays (sparse whitelist); result is out.
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
        // Chinese prose for the word path: the query's words spread across the
        // row, a fragment that shares only the first of them, a mixed-script row,
        // and a two-word pair that must find both words without them touching.
        row(
            "S2",
            3,
            "item/user",
            &user_text("网络连接总是断掉，能否把重试次数放大一点"),
        ),
        row("S2", 4, "item/assistant", &assistant_text("网络连接")),
        row(
            "S2",
            5,
            "item/user",
            &user_text("opencode.ai 的 ifIndex 报错"),
        ),
        row(
            "S2",
            6,
            "item/assistant",
            &assistant_text("codex 感知 模块，retry 重试 都在这"),
        ),
        row(
            "S2",
            7,
            "item/user",
            &user_text("配置从哪来，落盘时机是什么"),
        ),
        row("S2", 8, "item/assistant", &assistant_text("配置项渲染")),
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
        assert_eq!(
            *start, cursor,
            "chunk {index} must start where the previous ended"
        );
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
    assert_eq!(
        rebuilt,
        source.trim(),
        "chunks must tile the source losslessly"
    );
}

