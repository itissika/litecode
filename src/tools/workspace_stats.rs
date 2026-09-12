//! Agent `workspace_stats` — which other sessions are recently active here.
//!
//! Read-only, no parameters. Reports litecode-managed sessions only: work
//! happening outside litecode (other editors, manual edits) is not visible.
//! Each row carries a one-line preview of the session's latest user message,
//! so the caller can see why that session is active.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::session_search::short_session_ref;
use crate::session::SessionDataReader;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// A session whose last write is older than this is treated as idle.
const ACTIVE_WINDOW_MS: i64 = 10 * 60 * 1000;
/// Keep the page small: list at most this many sessions.
const MAX_LISTED: usize = 12;
/// Bound the self-subtree walk (children of children are blocked today).
const MAX_SUBTREE_NODES: usize = 64;

pub struct WorkspaceStatsTool;

impl WorkspaceStatsTool {
    fn run(&self, execution: &ToolExecutionContext) -> ToolCallResult {
        let reader = match execution.session_reader() {
            Ok(reader) => reader,
            Err(error) => return ToolCallResult::error(error.to_string()),
        };
        let me = (!execution.session_id.is_empty()).then_some(execution.session_id.as_str());
        let now = now_ms();
        let rows = match reader.list_session_activity_blocking(now - ACTIVE_WINDOW_MS) {
            Ok(rows) => rows,
            Err(error) => return ToolCallResult::error(error.to_string()),
        };
        let excluded = self_and_family(&reader, me);
        let mut lines: Vec<String> = rows
            .into_iter()
            .filter(|(id, ..)| !excluded.contains(id))
            .map(|(id, parent, updated_at, agent, last_message)| {
                let who = match parent {
                    Some(parent) => {
                        format!("{agent}, subagent of {}", short_session_ref(&parent))
                    }
                    None => agent,
                };
                let mut line = format!(
                    "- {} ({who}) — last write {}",
                    short_session_ref(&id),
                    idle_label(now - updated_at)
                );
                let preview = one_line_preview(&last_message);
                if !preview.is_empty() {
                    line.push_str(&format!(" — “{preview}”"));
                }
                line
            })
            .collect();
        if lines.is_empty() {
            return ToolCallResult::ok(format!(
                "no other sessions active in this workspace (last {}m).",
                ACTIVE_WINDOW_MS / 60_000
            ));
        }
        let overflow = lines.len().saturating_sub(MAX_LISTED);
        lines.truncate(MAX_LISTED);
        let mut out = format!(
            "other sessions active in this workspace (last {}m):\n",
            ACTIVE_WINDOW_MS / 60_000
        );
        out.push_str(&lines.join("\n"));
        out.push('\n');
        if overflow > 0 {
            out.push_str(&format!("… and {overflow} more\n"));
        }
        out.push_str(
            "note: litecode-managed sessions only — work outside litecode is not visible. \
             Concurrent work is possible: re-read a file before overwriting it, and expect \
             other edits while you work.",
        );
        ToolCallResult::ok(out)
    }
}

/// Self, self's parent chain, and self's whole subtree.
fn self_and_family(reader: &SessionDataReader, me: Option<&str>) -> HashSet<String> {
    let mut set: HashSet<String> = HashSet::new();
    let Some(me) = me else {
        return set;
    };
    set.insert(me.to_string());
    let mut cursor = me.to_string();
    for _ in 0..8 {
        let Ok(meta) = reader.meta_blocking(&cursor) else {
            break;
        };
        match meta.parent_session_id {
            Some(parent) => {
                if !set.insert(parent.clone()) {
                    break;
                }
                cursor = parent;
            }
            None => break,
        }
    }
    let mut queue = vec![me.to_string()];
    while let Some(parent) = queue.pop() {
        if set.len() > MAX_SUBTREE_NODES {
            break;
        }
        if let Ok(children) = reader.list_child_ids_blocking(&parent) {
            for child in children {
                if set.insert(child.clone()) {
                    queue.push(child);
                }
            }
        }
    }
    set
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn idle_label(elapsed_ms: i64) -> String {
    let secs = elapsed_ms.max(0) as u64 / 1000;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

/// Collapse the stored user-message preview onto one short line. Even
/// truncated, it reveals why the session is active.
fn one_line_preview(raw: &str) -> String {
    const MAX_CHARS: usize = 120;
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(MAX_CHARS).collect();
    if collapsed.chars().count() > MAX_CHARS {
        out.push('…');
    }
    out.trim().to_string()
}

impl Tool for WorkspaceStatsTool {
    fn name(&self) -> &str {
        "workspace_stats"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(self.run(&execution)))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        self.run(&ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: crate::config::workspace::workspace_root_lap(),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: self.max_result_size(),
            session_id: String::new(),
            session: None,
        })
    }

    fn description(&self, _ctx: &Context) -> String {
        "List other sessions recently active in this workspace, including their subagents. \
         Call this before large refactors or overwrites to check whether someone else may be \
         editing the same files. Read-only."
            .into()
    }

    fn timeout(&self) -> Option<u64> {
        Some(30)
    }

    fn max_result_size(&self) -> usize {
        4_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::data::command::{MutationId, SessionMutation};
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::{ToolSignalLevel, user_text};

    fn run_tool(root: &std::path::Path, active: &str) -> ToolCallResult {
        let tool = WorkspaceStatsTool;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(tool.execute(
            serde_json::json!({}),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: tool.max_result_size(),
                session_id: active.to_string(),
                session: Some(crate::session::SessionDataReader::open(
                    &root.join(".litecode").join("sessions.db"),
                )),
            },
        ))
    }

    fn open_data(root: &std::path::Path) -> (std::sync::Arc<SessionData>, std::path::PathBuf) {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        (data, db)
    }

    fn create(data: &SessionData, root: &std::path::Path, text: &str) -> String {
        let id = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(&id, &[user_text(text)]).unwrap();
        id
    }

    fn create_child(
        data: &SessionData,
        root: &std::path::Path,
        parent: &str,
        text: &str,
    ) -> String {
        let id = data
            .mutate_blocking(SessionMutation::Create {
                operation_id: MutationId::new(),
                project: root.to_str().unwrap().into(),
                agent_id: "default".into(),
                model_id: None,
                parent_session_id: Some(parent.to_string()),
                parent_call_id: Some(format!("call_{parent}")),
            })
            .unwrap()
            .session_id;
        data.insert_items(&id, &[user_text(text)]).unwrap();
        id
    }

    #[test]
    fn reports_none_when_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (data, _db) = open_data(root);
        let me = create(&data, root, "hello");
        drop(data);

        let r = run_tool(root, &me);
        assert_eq!(r.level, ToolSignalLevel::Ok, "{}", r.content);
        assert!(
            r.content.contains("no other sessions active"),
            "{}",
            r.content
        );
    }

    #[test]
    fn lists_other_sessions_and_excludes_self() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (data, _db) = open_data(root);
        let me = create(&data, root, "mine");
        let other = create(&data, root, "other work");
        drop(data);

        let r = run_tool(root, &me);
        assert_eq!(r.level, ToolSignalLevel::Ok, "{}", r.content);
        assert!(
            r.content.contains(short_session_ref(&other)),
            "other session missing:\n{}",
            r.content
        );
        assert!(
            r.content.contains("last write") && r.content.contains("other work"),
            "last-write label or user-message preview missing:\n{}",
            r.content
        );
        assert!(
            !r.content.contains(short_session_ref(&me)),
            "self must not be listed:\n{}",
            r.content
        );
    }

    #[test]
    fn collapses_long_user_message_to_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (data, _db) = open_data(root);
        let me = create(&data, root, "mine");
        let long_text = format!(
            "first line about refactoring\n\n{}",
            "padding word ".repeat(60)
        );
        let other = create(&data, root, &long_text);
        drop(data);

        let r = run_tool(root, &me);
        assert_eq!(r.level, ToolSignalLevel::Ok, "{}", r.content);
        let line = r
            .content
            .lines()
            .find(|l| l.contains(short_session_ref(&other)))
            .expect("other session listed");
        assert!(
            line.contains("first line about refactoring padding word"),
            "collapsed preview missing:\n{line}"
        );
        assert!(line.contains('…'), "truncation marker missing:\n{line}");
        assert!(!line.contains('\n'), "preview must be one line:\n{line}");
    }

    #[test]
    fn annotates_other_subagents_and_hides_own_children() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (data, _db) = open_data(root);
        let me = create(&data, root, "mine");
        let my_child = create_child(&data, root, &me, "my own helper");
        let other = create(&data, root, "other work");
        let other_child = create_child(&data, root, &other, "their helper");
        drop(data);

        let r = run_tool(root, &me);
        assert_eq!(r.level, ToolSignalLevel::Ok, "{}", r.content);
        assert!(
            r.content
                .contains(&format!("subagent of {}", short_session_ref(&other))),
            "other's subagent annotation missing:\n{}",
            r.content
        );
        assert!(
            r.content.contains(short_session_ref(&other_child)),
            "other's subagent missing:\n{}",
            r.content
        );
        assert!(
            !r.content.contains(short_session_ref(&my_child)),
            "own child must not be listed:\n{}",
            r.content
        );
        assert!(
            !r.content.contains(short_session_ref(&me)),
            "self must not be listed:\n{}",
            r.content
        );
    }
}
