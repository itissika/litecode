//! Code-surface result projection.
//!
//! Tools may carry richer data for direct/UI callers. A nested `run_code`
//! call receives only the agent-facing `data`; reminder-only values move to
//! `ToolCallResult.metadata`, which never enters JS or the generated SDK.

use serde_json::{Map, Value};

use crate::types::ToolCallResult;

fn move_keys(data: &mut Map<String, Value>, metadata: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        if let Some(value) = data.remove(*key) {
            metadata.insert((*key).to_string(), value);
        }
    }
}

fn object(value: Option<&Value>) -> Option<Map<String, Value>> {
    value.and_then(Value::as_object).cloned()
}

pub fn project_code_result(tool: &str, mut result: ToolCallResult) -> ToolCallResult {
    let Some(mut data) = object(result.data.as_ref()) else {
        return result;
    };
    let mut metadata = object(result.metadata.as_ref()).unwrap_or_default();

    match tool {
        "exec_command" | "write_stdin" => {
            let mut kept = Map::new();
            for key in ["session_id", "exit_code", "output"] {
                if let Some(value) = data.remove(key) {
                    kept.insert(key.to_string(), value);
                }
            }
            move_keys(&mut data, &mut metadata, &["truncated", "clamped"]);
            data = kept;
        }
        "write" => {
            let mut kept = Map::new();
            let path = data
                .get("file")
                .and_then(Value::as_object)
                .and_then(|file| file.get("path"))
                .cloned()
                .or_else(|| data.get("path").cloned());
            if let Some(path) = path {
                kept.insert("path".into(), path);
            }
            move_keys(&mut data, &mut metadata, &["path_risk", "diagnostics"]);
            data = kept;
        }
        "apply_edits" => {
            let results = data
                .remove("results")
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
            let mut kept_results = Vec::with_capacity(results.len());
            let mut fact_results = Vec::with_capacity(results.len());
            for result in results {
                let mut item = object(Some(&result)).unwrap_or_default();
                let mut facts = Map::new();
                move_keys(&mut item, &mut facts, &["path_risk", "diagnostics"]);
                item.remove("nearest_line");
                kept_results.push(Value::Object(item));
                fact_results.push(Value::Object(facts));
            }
            data = Map::from_iter([("results".into(), Value::Array(kept_results))]);
            metadata.insert("results".into(), Value::Array(fact_results));
        }
        "read" => {
            move_keys(
                &mut data,
                &mut metadata,
                &[
                    "start_line",
                    "total_lines",
                    "has_more",
                    "skipped_in_context",
                    "lines_capped",
                    "clamped",
                ],
            );
            data.remove("path");
        }
        "glob" => move_keys(
            &mut data,
            &mut metadata,
            &[
                "total",
                "truncated",
                "outside_workspace",
                "empty_reason",
                "clamped",
            ],
        ),
        "grep" => {
            let mut kept = Map::new();
            if let Some(hits) = data.remove("hits") {
                kept.insert("hits".into(), hits);
            }
            for key in [
                "truncated",
                "empty_reason",
                "total_hits",
                "at_limit_cap",
                "lines_truncated",
                "context_omitted",
                "output_capped",
                "clamped",
            ] {
                if let Some(value) = data.remove(key) {
                    metadata.insert(key.into(), value);
                }
            }
            data = kept;
        }
        "diagnostics"
        | "document_symbol"
        | "workspace_symbol"
        | "find_references"
        | "go_to_definition"
        | "go_to_implementation"
        | "hover" => {
            move_keys(&mut data, &mut metadata, &["state", "truncated"]);
        }
        "websearch" => {
            data.remove("query");
            move_keys(&mut data, &mut metadata, &["clamped"]);
        }
        "code_search" | "session_search" => {
            move_keys(&mut data, &mut metadata, &["clamped"]);
        }
        "browser_read" => {
            data.remove("message");
        }
        _ => {}
    }

    result.data = Some(Value::Object(data));
    if !metadata.is_empty() {
        result.metadata = Some(Value::Object(metadata));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(tool: &str, data: Value) -> ToolCallResult {
        project_code_result(tool, crate::tool::outcome::ok_data(data))
    }

    #[test]
    fn read_moves_paging_to_metadata_and_drops_path() {
        let result = project(
            "read",
            json!({
                "path": "a.txt",
                "lines": ["one"],
                "start_line": 1,
                "total_lines": 3,
                "has_more": true,
                "skipped_in_context": 0,
                "clamped": [{"parameter": "limit", "requested": 0, "applied": 1}]
            }),
        );
        assert_eq!(result.data, Some(json!({"lines": ["one"]})));
        assert_eq!(
            result.metadata,
            Some(json!({
                "start_line": 1,
                "total_lines": 3,
                "has_more": true,
                "skipped_in_context": 0,
                "clamped": [{"parameter": "limit", "requested": 0, "applied": 1}]
            }))
        );
    }

    #[test]
    fn write_flattens_path_and_keeps_reminders_in_metadata() {
        let result = project(
            "write",
            json!({
                "created": true,
                "file": {"path": "a.txt", "bytes": 1, "hash": "x", "abs": "/a.txt"},
                "path_risk": {"outside_workspace": true, "sensitive": false},
                "diagnostics": [{"code": "lsp_error", "severity": "warning", "message": "m"}]
            }),
        );
        assert_eq!(result.data, Some(json!({"path": "a.txt"})));
        assert_eq!(
            result.metadata,
            Some(json!({
                "path_risk": {"outside_workspace": true, "sensitive": false},
                "diagnostics": [{"code": "lsp_error", "severity": "warning", "message": "m"}]
            }))
        );
    }

    #[test]
    fn apply_edits_keeps_one_item_shape_and_moves_item_facts() {
        let result = project(
            "apply_edits",
            json!({
                "results": [{
                    "ok": false,
                    "path": "a.txt",
                    "reason": "no_match",
                    "detail": "closest match is at line 2",
                    "nearest_line": 2,
                    "path_risk": {"outside_workspace": true, "sensitive": false},
                    "diagnostics": []
                }]
            }),
        );
        assert_eq!(
            result.data,
            Some(json!({
                "results": [{
                    "ok": false,
                    "path": "a.txt",
                    "reason": "no_match",
                    "detail": "closest match is at line 2"
                }]
            }))
        );
        assert_eq!(
            result.metadata,
            Some(json!({
                "results": [{
                    "path_risk": {"outside_workspace": true, "sensitive": false},
                    "diagnostics": []
                }]
            }))
        );
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exec_moves_truncation_and_clamp_to_metadata() {
        let result = project_code_result(
            "exec_command",
            crate::tool::outcome::ok_data(json!({
                "session_id": "bg_1",
                "exit_code": null,
                "output": "hello",
                "truncated": true,
                "clamped": [{"parameter": "yield_time_ms", "requested": 60000, "applied": 30000}]
            })),
        );
        assert_eq!(
            result.data,
            Some(json!({"session_id": "bg_1", "exit_code": null, "output": "hello"}))
        );
        assert_eq!(
            result.metadata,
            Some(json!({
                "truncated": true,
                "clamped": [{"parameter": "yield_time_ms", "requested": 60000, "applied": 30000}]
            }))
        );
    }
}
