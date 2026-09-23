//! Agent `session_search` — one token-bounded view over past session transcripts.

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::session_search::{self, ContextWindowExclude};
use crate::engines::{RetrievalFilters, WorkspaceEngines};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

pub struct SessionSearchTool {
    engines: WorkspaceEngines,
}

impl SessionSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self { engines }
    }

    fn search_in_workspace(
        &self,
        workspace_root: &std::path::Path,
        reader: &crate::session::SessionDataReader,
        query: &str,
        session_id: Option<&str>,
        active_session_id: Option<&str>,
    ) -> ToolCallResult {
        let include_session = match session_id {
            Some(refer) if !refer.trim().is_empty() => {
                match session_search::resolve_session_ref(reader, refer.trim()) {
                    Ok(id) => Some(id),
                    Err(e) => return ToolCallResult::error(e.to_string()),
                }
            }
            _ => None,
        };

        let exclude_context_window = match active_session_id {
            Some(sid) if include_session.as_ref().map(|i| i == sid).unwrap_or(true) => {
                match session_search::load_surface_seqs(reader, sid) {
                    Ok(surface_seqs) => Some(ContextWindowExclude {
                        session_id: sid.to_string(),
                        surface_seqs,
                    }),
                    Err(e) => return ToolCallResult::error(e.to_string()),
                }
            }
            _ => None,
        };

        // Read-only on purpose: the session ANN is refreshed by the idle tick in
        // `serve::router::listen`, never by the search that needs it. Refreshing
        // here put a reconcile in front of the query that asked for it, and left
        // the lane unusable whenever the two overlapped — a session corpus moves
        // on every turn, so "refresh on demand" meant "refresh always, right
        // here". A stale index is the accepted price; being wrong is not.
        let bundle = match self.engines.search_sessions(
            query,
            0,
            RetrievalFilters {
                include_session_id: include_session,
                exclude_context_window,
                caller_session_id: active_session_id.map(str::to_string),
                session: Some(reader.clone()),
                ..Default::default()
            },
            Some(workspace_root.to_path_buf()),
        ) {
            Ok(b) => b,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let view = match session_search::build_agent_view(reader, &bundle.ranked, workspace_root) {
            Ok(view) => view,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };
        if view.trim().is_empty() {
            return ToolCallResult::ok(format!(
                "No matching session transcript context for query '{query}'."
            ));
        }
        ToolCallResult::ok(view)
    }
}

impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to find in past session transcripts. Literal words/phrase; separate alternatives with | (any may match, e.g. 'retry|重试'); matched literally and case-insensitively, so separators count. The current session's live context window is never searched."
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session scope: full id, unique prefix, or the handle shown in a result."
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(
            self.search_from_input(input, &execution),
        ))
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.search_from_input(
            input,
            &ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: crate::config::workspace::workspace_root_lap(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
                session: None,
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        "Search past conversation transcripts. Use it to recall a decision, a fact, or a code change from an earlier session, including work another session did. \
         Matching is literal and case-insensitive; alternatives: separate with | (e.g. 'retry|重试'). \
         The current session's live context-window turns are never searched. \
         Read a hit with read or grep on `.litecode/sessions/<id>.md` — join that directory, the id shown in the result, and `.md`; bash cannot reach it, and a hit's `L` range is the start_line/end_line to ask for. \
         Narrow with session_id when the result names another session. \
         A result too large for one response names a .txt file that holds the remainder."
            .into()
    }

    fn timeout(&self) -> Option<u64> {
        Some(30)
    }

    fn max_result_size(&self) -> usize {
        usize::MAX
    }
}

impl SessionSearchTool {
    fn search_from_input(&self, input: Value, execution: &ToolExecutionContext) -> ToolCallResult {
        let reader = match execution.session_reader() {
            Ok(reader) => reader,
            Err(error) => return ToolCallResult::error(error.to_string()),
        };
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolCallResult::error(e),
        };
        let session_id = input["session_id"]
            .as_str()
            .filter(|s| !s.trim().is_empty());
        let active = if execution.session_id.is_empty() {
            None
        } else {
            Some(execution.session_id.as_str())
        };
        self.search_in_workspace(
            &execution.workspace_root,
            reader,
            query.trim(),
            session_id,
            active,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::transcript_file;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::ToolSignalLevel;
    use crate::types::user_text;

    fn call_ok(
        tool: &SessionSearchTool,
        root: &std::path::Path,
        input: Value,
        active: &str,
    ) -> String {
        let r = call(tool, root, input, active);
        assert_eq!(
            r.level,
            ToolSignalLevel::Ok,
            "expected ok, got {:?}: {}",
            r.level,
            r.content
        );
        r.content
    }

    fn call(
        tool: &SessionSearchTool,
        root: &std::path::Path,
        input: Value,
        active: &str,
    ) -> ToolCallResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(tool.execute(
            input,
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

    fn seed_session(root: &std::path::Path, text: &str) -> String {
        seed_items(root, &[user_text(text)])
    }

    fn seed_items(root: &std::path::Path, items: &[crate::types::Item]) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(&id, items).unwrap();
        id
    }

    #[test]
    fn typical_broad_search_renders_handle_and_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let id_a = seed_session(
            root,
            "We decided AUTH_REFACTOR_TOKEN moves login to middleware.",
        );
        seed_session(root, "unrelated OTHER_TOPIC_ONLY content");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "AUTH_REFACTOR_TOKEN" }),
            "",
        );
        assert!(
            out.contains(&format!(
                "### {} · ",
                session_search::short_session_ref(&id_a)
            )),
            "session header missing:\n{out}"
        );
        assert!(out.contains("Matches"), "{out}");
        assert!(out.contains(": user"), "{out}");
        assert!(out.contains("AUTH_REFACTOR_TOKEN"), "{out}");
        assert!(!out.contains("created:"), "{out}");
        assert!(
            !out.contains("virtual"),
            "no path plumbing in the view:\n{out}"
        );
        assert!(!out.contains("seq"), "the view must not expose seq:\n{out}");
        assert!(!out.contains("offset"), "{out}");
        assert!(!out.contains("OTHER_TOPIC_ONLY"), "{out}");
    }

    #[test]
    fn typical_multi_hit_same_session_lists_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_items(
            root,
            &[
                user_text("first MULTI_HIT_TOKEN occurrence here"),
                user_text("unrelated filler line"),
                user_text("second MULTI_HIT_TOKEN occurrence there"),
            ],
        );

        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({
                "query": "MULTI_HIT_TOKEN",
                "session_id": sid,
            }),
            "",
        );
        assert!(
            out.contains(&format!(
                "### {} · ",
                session_search::short_session_ref(&sid)
            )),
            "{out}"
        );
        let line_hits = out
            .lines()
            .filter(|l| l.contains("MULTI_HIT_TOKEN"))
            .count();
        assert!(line_hits >= 2, "expected two hit blocks:\n{out}");
    }

    #[test]
    fn a_result_over_budget_names_the_file_that_holds_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut items = Vec::new();
        for i in 0..120 {
            items.push(user_text(format!(
                "PAGE_TOKEN_{i:03} shared PAGE_NEEDLE {}",
                "word ".repeat(80)
            )));
        }
        seed_items(root, &items);

        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "PAGE_NEEDLE" }),
            "",
        );
        assert!(!out.contains("offset"), "paging is gone:\n{out}");
        assert!(
            out.matches(": user").count() < 120,
            "the view must not carry every hit:\n{out}"
        );
        let location = out
            .split(" are in ")
            .nth(1)
            .and_then(|rest| rest.split(" —").next())
            .expect(&out);
        assert!(
            location.starts_with(".litecode/bash/session_search_"),
            "{out}"
        );
        let spilled = std::fs::read_to_string(root.join(location)).unwrap();
        assert!(spilled.contains("Remaining "), "{spilled}");
        assert!(spilled.contains("PAGE_NEEDLE"), "{spilled}");
    }

    #[test]
    fn rejects_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let r = call(&tool, dir.path(), serde_json::json!({ "query": "   " }), "");
        assert_eq!(r.level, ToolSignalLevel::Error);
        assert!(r.content.contains("query"), "{}", r.content);
    }

    #[test]
    fn active_session_context_window_is_excluded_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let current_id = seed_session(root, "VISIBLE_IN_WINDOW_MARKER only here");
        let other_id = seed_session(root, "VISIBLE_IN_WINDOW_MARKER also in other");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "VISIBLE_IN_WINDOW_MARKER" }),
            &current_id,
        );
        assert!(
            out.contains(session_search::short_session_ref(&other_id)),
            "other session should hit:\n{out}"
        );
        assert!(
            !out.contains(session_search::short_session_ref(&current_id)),
            "current live window must not echo:\n{out}"
        );
    }

    #[test]
    fn a_two_character_cjk_query_finds_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_session(root, "短词标记在此");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(&tool, root, serde_json::json!({ "query": "短词" }), "");
        // Two characters is an ordinary query: the row is found, with the normal
        // view, and nothing is said about the index.
        assert!(out.contains("### "), "{out}");
        assert!(out.contains("L2: user"), "{out}");
        assert!(out.contains("短词标记在此"), "{out}");
        assert!(!out.contains("Nothing was searched"), "{out}");
        assert!(!out.contains("grep(pattern="), "{out}");
    }

    #[test]
    fn compacted_history_below_kept_from_seq_remains_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let sid = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(
            &sid,
            &[
                user_text("ARCHIVED_OLD_MARKER buried before compact"),
                user_text("filler middle"),
                user_text("LIVE_TAIL_MARKER still in window"),
            ],
        )
        .unwrap();
        data.compact_from(&sid, &user_text("[summary] prior archived"), Some(2), 10)
            .unwrap();

        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let archived = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "ARCHIVED_OLD_MARKER" }),
            &sid,
        );
        assert!(
            archived.contains("ARCHIVED_OLD_MARKER"),
            "archived detail should still search:\n{archived}"
        );
        let live = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "LIVE_TAIL_MARKER" }),
            &sid,
        );
        assert!(
            live.contains("No matching session transcript"),
            "live window must stay excluded:\n{live}"
        );
    }

    #[test]
    fn session_id_scopes_to_one_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = seed_session(root, "SCOPE_MARKER in a");
        seed_session(root, "SCOPE_MARKER in b");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "SCOPE_MARKER", "session_id": a }),
            "",
        );
        assert!(out.contains(session_search::short_session_ref(&a)), "{out}");
        assert_eq!(out.matches("### ").count(), 1, "{out}");
    }

    #[test]
    fn unknown_session_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_session(root, "something");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let r = call(
            &tool,
            root,
            serde_json::json!({ "query": "something", "session_id": "ZZZZNOPE" }),
            "",
        );
        assert_eq!(r.level, ToolSignalLevel::Error);
        assert!(r.content.contains("matched no sessions"), "{}", r.content);
    }

    #[test]
    fn empty_result_has_no_group_headings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_session(root, "hello");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "NO_SUCH_TOKEN_ZZZ" }),
            "",
        );
        assert!(out.contains("No matching session transcript"), "{out}");
        assert!(!out.contains("### "), "{out}");
    }

    #[test]
    fn search_hit_line_matches_virtual_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nSEARCH_READ_ALIGN_TOKEN here\ndelta");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "SEARCH_READ_ALIGN_TOKEN" }),
            "",
        );
        // A hit is a label line (`L2-4: user`) plus the indented lines of that
        // range: reading the range must land on the very same rendering.
        let hit = out
            .lines()
            .find(|l| l.starts_with('L') && l.contains(": "))
            .expect(&format!("hit line missing:\n{out}"));
        let range = hit.split(':').next().unwrap().trim_start_matches('L');
        let (start_line, end_line) = match range.split_once('-') {
            Some((a, b)) => (a.parse::<u32>().expect(hit), b.parse::<u32>().expect(hit)),
            None => {
                let n = range.parse::<u32>().expect(hit);
                (n, n)
            }
        };
        let path = transcript_file::virtual_path_for(&sid);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let read = rt.block_on(crate::tools::read::ReadTool::default().execute(
            serde_json::json!({
                "file_path": path,
                "start_line": start_line,
                "end_line": end_line,
            }),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: String::new(),
                session: Some(crate::session::SessionDataReader::open(
                    &root.join(".litecode").join("sessions.db"),
                )),
            },
        ));
        assert_eq!(read.level, ToolSignalLevel::Ok, "{}", read.content);
        assert!(
            read.content.contains("SEARCH_READ_ALIGN_TOKEN"),
            "read at L{start_line}-{end_line} should contain the hit:\n{}",
            read.content
        );
    }

    #[test]
    fn schema_is_query_and_session_scope() {
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let schema = tool.schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("session_id"));
        assert!(!props.contains_key("offset"));
        assert!(!props.contains_key("session_filter"));
        assert!(!props.contains_key("expand"));
        assert_eq!(schema["additionalProperties"], false);
    }
}
