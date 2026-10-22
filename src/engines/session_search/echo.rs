//! Session-echo result rows: `tool_result`s whose content is a copy of another
//! session's transcript, because the producing call was `session_search`, or a
//! `read`/`grep` aimed at the session store (`.litecode/sessions/…`).
//!
//! 定稿（2026-09-22）：两条腿一致——回声产出**整条剔除**，调用行（意图）保留。
//! 理由：复制品不是原件；稀疏腿的精确落点会钉在复制品上，而且生产排序同分
//! 先看"最近更新"，复制品天然压过原件。代价：fixture 里 1 条 `session_level`
//! 真值行（"读会话回声"那个元用例）随之不可检索。
//!
//! Detection is structural — call linkage through `call_id` — not textual. A
//! content signature (session path / `[seq:N item/` markers) catches only ~71%
//! of the echoes and misfires on bash/glob output, so it is not used.

use std::collections::HashSet;
use std::path::Path;

use crate::session::transcript_file::{
    SearchableRow, call_reads_sessions, ledger_reads_sessions, load_blob_text,
};
use anyhow::Result;
use serde_json::Value;

/// Call linkage of one `item/tool_call` row: which result belongs to it, and
/// whether that result is a copy of session content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInfo {
    pub call_id: String,
    /// The call reads from the session store, so its result is an echo.
    pub session_read: bool,
}

/// The linkage of one `item/tool_call` row. `None` for any other kind, for an
/// unreadable body, or for a call that carries no id.
///
/// This is the single place that decides "is this call an echo call?": the
/// admission rule ([`result_keys`]) and the derivation stage both go through it,
/// so a row's stored metadata can never disagree with how it was admitted.
pub fn call_info(row: &SearchableRow, data_root: &Path) -> Option<CallInfo> {
    if row.kind != "item/tool_call" {
        return None;
    }
    let v = row_json(row, data_root)?;
    if v.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let call_id = v.get("call_id").and_then(Value::as_str)?.to_string();
    let name = v.get("name").and_then(Value::as_str).unwrap_or("");
    let session_read = call_reads_sessions(name, v.get("arguments").and_then(Value::as_str));
    Some(CallInfo {
        call_id,
        session_read,
    })
}

/// The `call_id` an `item/tool_result` row answers, when it names one.
pub fn result_call_id(row: &SearchableRow, data_root: &Path) -> Option<String> {
    if row.kind != "item/tool_result" {
        return None;
    }
    let v = row_json(row, data_root)?;
    if v.get("type").and_then(Value::as_str) != Some("function_call_output") {
        return None;
    }
    v.get("call_id").and_then(Value::as_str).map(str::to_string)
}

/// `(session_id, seq)` of every tool-result row that is a session echo.
///
/// A `call_id` only means something inside the transcript that produced it, so
/// the pairing is scoped to the session. Keeping it scoped is what makes the
/// relation *local*: a session can be projected on its own, without consulting
/// any other session's calls.
pub fn result_keys(rows: &[SearchableRow], data_root: &Path) -> Result<HashSet<(String, i64)>> {
    result_keys_with(rows, data_root, &HashSet::new())
}

/// [`result_keys`], with the closure seeded from calls the index already knows
/// about.
///
/// An incremental batch may contain a result whose call was settled in an
/// earlier batch; without the seed that result would be readmitted as ordinary
/// content and its copy would become searchable. Call linkage is
/// `(session_id, call_id)` — a result can only answer a call in its own session.
pub fn result_keys_with(
    rows: &[SearchableRow],
    data_root: &Path,
    known: &HashSet<(String, String)>,
) -> Result<HashSet<(String, i64)>> {
    let mut echo_calls: HashSet<(String, String)> = known.clone();
    echo_calls.extend(
        rows.iter()
            .filter_map(|row| call_info(row, data_root).map(|info| (row, info)))
            .filter(|(_, info)| info.session_read)
            .map(|(row, info)| (row.session_id.clone(), info.call_id)),
    );

    let mut out = HashSet::new();
    for row in rows {
        if let Some(call_id) = result_call_id(row, data_root)
            && echo_calls.contains(&(row.session_id.clone(), call_id))
        {
            out.insert((row.session_id.clone(), row.seq));
        }
        if result_body_reads_sessions(row, data_root) {
            out.insert((row.session_id.clone(), row.seq));
        }
    }
    Ok(out)
}

fn result_body_reads_sessions(row: &SearchableRow, data_root: &Path) -> bool {
    let Some(value) = row_json(row, data_root) else {
        return false;
    };
    if value.get("type").and_then(Value::as_str) != Some("function_call_output") {
        return false;
    }
    match value.get("output") {
        Some(Value::String(text)) => ledger_reads_sessions(text),
        Some(other) => ledger_reads_sessions(&other.to_string()),
        None => false,
    }
}

/// The row's raw JSON, parsed. `None` on unreadable input — the caller keeps
/// the row rather than guessing.
fn row_json(row: &SearchableRow, data_root: &Path) -> Option<Value> {
    let raw = if let Some(body) = &row.body {
        body.clone()
    } else {
        row.body_ref
            .as_deref()
            .and_then(|r| load_blob_text(r, data_root).ok())?
    };
    serde_json::from_str::<Value>(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call_row(sid: &str, seq: i64, name: &str, args: &str) -> SearchableRow {
        let body = json!({
            "type": "function_call",
            "call_id": format!("c{seq}"),
            "name": name,
            "arguments": args,
        })
        .to_string();
        SearchableRow {
            session_id: sid.into(),
            seq,
            kind: "item/tool_call".into(),
            item_type: "function_call".into(),
            body: Some(body),
            body_ref: None,
        }
    }

    fn result_row(sid: &str, seq: i64, call_seq: i64) -> SearchableRow {
        let body = json!({
            "type": "function_call_output",
            "call_id": format!("c{call_seq}"),
            "output": "copied page",
        })
        .to_string();
        SearchableRow {
            session_id: sid.into(),
            seq,
            kind: "item/tool_result".into(),
            item_type: "function_call_output".into(),
            body: Some(body),
            body_ref: None,
        }
    }

    #[test]
    fn nested_code_calls_are_echoes_when_the_ledger_reads_sessions() {
        let rows = vec![
            call_row(
                "s1",
                1,
                "run_code",
                r#"{"code":"await tools.read({path:'.litecode/sessions/abc.md'})"}"#,
            ),
            {
                let body = json!({
                    "type": "function_call_output",
                    "call_id": "c1",
                    "output": "1. read path=.litecode/sessions/abc.md ok\n\nsecret session body",
                })
                .to_string();
                SearchableRow {
                    session_id: "s1".into(),
                    seq: 2,
                    kind: "item/tool_result".into(),
                    item_type: "function_call_output".into(),
                    body: Some(body),
                    body_ref: None,
                }
            },
            call_row("s1", 3, "run_code", r#"{"code":"return 1"}"#),
            result_row("s1", 4, 3),
        ];
        let keys = result_keys(&rows, Path::new(".")).unwrap();
        assert!(keys.contains(&("s1".to_string(), 2)), "{keys:?}");
        assert!(!keys.contains(&("s1".to_string(), 4)), "{keys:?}");
    }
}
