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

use anyhow::Result;
use crate::session::transcript_file::{SearchableRow, VIRTUAL_SESSION_DIR, load_blob_text};
use serde_json::Value;

/// `(session_id, seq)` of every tool-result row that is a session echo.
pub fn result_keys(rows: &[SearchableRow], data_root: &Path) -> Result<HashSet<(String, i64)>> {
    let mut echo_calls: HashSet<String> = HashSet::new();
    for row in rows {
        if row.kind != "item/tool_call" {
            continue;
        }
        let Some(raw) = raw_json(row, data_root) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("function_call") {
            continue;
        }
        let Some(call_id) = v.get("call_id").and_then(Value::as_str) else {
            continue;
        };
        let name = v.get("name").and_then(Value::as_str).unwrap_or("");
        let echo = name == "session_search"
            || (matches!(name, "read" | "grep") && args_target_session(v.get("arguments")));
        if echo {
            echo_calls.insert(call_id.to_string());
        }
    }

    let mut out = HashSet::new();
    for row in rows {
        if row.kind != "item/tool_result" {
            continue;
        }
        let Some(raw) = raw_json(row, data_root) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("function_call_output") {
            continue;
        }
        if let Some(call_id) = v.get("call_id").and_then(Value::as_str)
            && echo_calls.contains(call_id)
        {
            out.insert((row.session_id.clone(), row.seq));
        }
    }
    Ok(out)
}

/// The row's raw JSON, inline or from its blob. `None` on unreadable input —
/// the caller keeps the row rather than guessing.
fn raw_json(row: &SearchableRow, data_root: &Path) -> Option<String> {
    if let Some(body) = &row.body {
        return Some(body.clone());
    }
    row.body_ref
        .as_deref()
        .and_then(|r| load_blob_text(r, data_root).ok())
}

/// `read`/`grep` arguments: any string value pointing into the session store.
fn args_target_session(arguments: Option<&Value>) -> bool {
    let Some(Value::String(raw)) = arguments else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let mut found = Vec::new();
    collect_strings(&v, &mut found);
    found.iter().any(|s| is_session_target(s))
}

fn collect_strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        Value::Object(m) => m.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

/// `.litecode/sessions` followed by `/` (a file below it) or end (the directory
/// itself). `.litecode/sessions.db` is *not* a hit. Backslashes are folded.
fn is_session_target(arg: &str) -> bool {
    let norm = arg.replace('\\', "/");
    let Some(pos) = norm.find(VIRTUAL_SESSION_DIR) else {
        return false;
    };
    let rest = &norm[pos + VIRTUAL_SESSION_DIR.len()..];
    rest.is_empty() || rest.starts_with('/')
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
    fn session_targeted_results_are_echoes() {
        let rows = vec![
            call_row(
                "s1",
                1,
                "read",
                r#"{"file_path":".litecode/sessions/01ARZ3NDEKTSV4RRFFQ69G5FAV.md","end_line":120}"#,
            ),
            result_row("s1", 2, 1),
            call_row("s1", 3, "read", r#"{"file_path":"src/main.rs"}"#),
            result_row("s1", 4, 3),
            call_row("s1", 5, "grep", r#"{"pattern":"x","path":".litecode/sessions"}"#),
            result_row("s1", 6, 5),
            call_row("s1", 7, "session_search", r#"{"query":"auth"}"#),
            result_row("s1", 8, 7),
            call_row("s1", 9, "read", r#"{"file_path":".litecode/sessions.db"}"#),
            result_row("s1", 10, 9),
        ];
        let keys = result_keys(&rows, Path::new(".")).unwrap();
        assert_eq!(keys.len(), 3, "{keys:?}");
        assert!(keys.contains(&("s1".to_string(), 2)));
        assert!(!keys.contains(&("s1".to_string(), 4)));
        assert!(keys.contains(&("s1".to_string(), 6)));
        assert!(keys.contains(&("s1".to_string(), 8)));
        assert!(!keys.contains(&("s1".to_string(), 10)));
    }

    #[test]
    fn session_target_matching() {
        assert!(is_session_target(
            ".litecode/sessions/01ARZ3NDEKTSV4RRFFQ69G5FAV.md"
        ));
        assert!(is_session_target(
            r"E:\ws\.litecode\sessions\01ARZ3NDEKTSV4RRFFQ69G5FAV.md"
        ));
        assert!(is_session_target(".litecode/sessions"));
        assert!(!is_session_target(".litecode/sessions.db"));
        assert!(!is_session_target("src/sessions/mod.rs"));
    }
}
