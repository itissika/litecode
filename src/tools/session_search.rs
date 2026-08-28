//! Agent `session_search` — grouped path:line hits over past session transcripts.

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
        query: &str,
        session_id: Option<&str>,
        offset: usize,
        active_session_id: Option<&str>,
    ) -> ToolCallResult {
        let db = session_search::sessions_db_under(workspace_root);

        let include_session = match session_id {
            Some(refer) if !refer.trim().is_empty() => {
                match session_search::resolve_session_ref(&db, refer.trim()) {
                    Ok(id) => Some(id),
                    Err(e) => return ToolCallResult::error(e.to_string()),
                }
            }
            _ => None,
        };

        let exclude_context_window = match active_session_id {
            Some(sid) if include_session.as_ref().map(|i| i == sid).unwrap_or(true) => {
                match session_search::load_surface_seqs(&db, sid) {
                    Ok(surface_seqs) => Some(ContextWindowExclude {
                        session_id: sid.to_string(),
                        surface_seqs,
                    }),
                    Err(e) => return ToolCallResult::error(e.to_string()),
                }
            }
            _ => None,
        };

        let bundle = match self.engines.search_sessions(
            query,
            offset,
            RetrievalFilters {
                include_session_id: include_session,
                exclude_context_window,
                ..Default::default()
            },
            Some(workspace_root.to_path_buf()),
        ) {
            Ok(b) => b,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let page = match session_search::build_search_page(&db, &bundle.ranked, bundle.offset) {
            Ok(p) => p,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };
        if page.groups.is_empty() {
            return ToolCallResult::ok(format!(
                "No matching session transcript context for query '{query}'."
            ));
        }
        ToolCallResult::ok(session_search::format_agent_page(&page))
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
                    "description": "What to find in past session transcripts. The current session's live context window is never searched."
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session scope: full id, unique prefix, or unique suffix from a previous hit path."
                },
                "offset": {
                    "type": "integer",
                    "description": "0-based hit offset for pagination (default 0). Use the next offset from a previous result when more hits remain."
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
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        "Search past conversation transcripts in this workspace. \
         Returns session groups with a virtual path and L<line>: summary hits. \
         Live context-window turns of the current session are always excluded. \
         Scope with session_id; deepen a hit with read or grep on the returned path."
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
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolCallResult::error(e),
        };
        let session_id = input["session_id"]
            .as_str()
            .filter(|s| !s.trim().is_empty());
        let offset = match parse_offset(&input["offset"]) {
            Ok(o) => o,
            Err(msg) => return ToolCallResult::error(msg),
        };
        let active = if execution.session_id.is_empty() {
            None
        } else {
            Some(execution.session_id.as_str())
        };
        self.search_in_workspace(
            &execution.workspace_root,
            query.trim(),
            session_id,
            offset,
            active,
        )
    }
}

fn parse_offset(v: &Value) -> std::result::Result<usize, String> {
    if v.is_null() {
        return Ok(0);
    }
    if let Some(n) = v.as_u64() {
        return Ok(n as usize);
    }
    if let Some(n) = v.as_i64() {
        if n < 0 {
            return Err(crate::tool::must_be("offset", "a non-negative integer"));
        }
        return Ok(n as usize);
    }
    Err(crate::tool::must_be("offset", "a non-negative integer"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::store::Session;
    use crate::session::transcript_file;
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
            },
        ))
    }

    fn seed_session(root: &std::path::Path, text: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let s = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        s.insert_detail_rows(&[user_text(text)]).unwrap();
        let id = s.id.clone();
        drop(s);
        id
    }

    #[test]
    fn typical_broad_search_renders_path_and_line() {
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
            out.contains(&format!("### {id_a}")),
            "session header missing:\n{out}"
        );
        assert!(out.contains("created:"), "{out}");
        assert!(out.contains("updated:"), "{out}");
        assert!(
            out.contains(&transcript_file::virtual_path_for(&id_a)),
            "path missing:\n{out}"
        );
        assert!(out.contains("matches:"), "{out}");
        assert!(out.contains("L"), "{out}");
        assert!(out.contains("AUTH_REFACTOR_TOKEN"), "{out}");
        assert!(
            !out.contains("seq:"),
            "agent view must not expose seq:\n{out}"
        );
        assert!(!out.contains("OTHER_TOPIC_ONLY"), "{out}");
        assert!(!out.contains("## Matches"), "{out}");
    }

    #[test]
    fn typical_multi_hit_same_session_lists_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let s = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let sid = s.id.clone();
        s.insert_detail_rows(&[
            user_text("first MULTI_HIT_TOKEN occurrence here"),
            user_text("unrelated filler line"),
            user_text("second MULTI_HIT_TOKEN occurrence there"),
        ])
        .unwrap();
        drop(s);

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
        assert!(out.contains(&format!("### {sid}")), "{out}");
        let line_hits = out
            .lines()
            .filter(|l| l.starts_with('L') && l.contains("MULTI_HIT_TOKEN"))
            .count();
        assert!(line_hits >= 2, "expected two hit lines:\n{out}");
    }

    #[test]
    fn pagination_footer_guides_next_offset() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let s = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let mut items = Vec::new();
        for i in 0..40 {
            items.push(user_text(format!(
                "PAGE_TOKEN_{i:02} shared PAGE_NEEDLE {}",
                "word ".repeat(80)
            )));
        }
        s.insert_detail_rows(&items).unwrap();
        drop(s);

        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let page0 = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "PAGE_NEEDLE" }),
            "",
        );
        let next = page0
            .lines()
            .find_map(|l| {
                l.strip_prefix("(more hits; offset: ")
                    .and_then(|rest| rest.strip_suffix(')'))
                    .and_then(|n| n.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if next == 0 {
            return;
        }
        let page1 = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "PAGE_NEEDLE", "offset": next }),
            "",
        );
        assert!(
            page1.contains("no further pages")
                || page1.contains("more hits")
                || page1.contains("L"),
            "{page1}"
        );
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
        assert!(out.contains(&other_id), "other session should hit:\n{out}");
        assert!(
            !out.contains(&current_id),
            "current live window must not echo:\n{out}"
        );
    }

    #[test]
    fn compacted_history_below_kept_from_seq_remains_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let s = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let sid = s.id.clone();
        s.insert_detail_rows(&[
            user_text("ARCHIVED_OLD_MARKER buried before compact"),
            user_text("filler middle"),
            user_text("LIVE_TAIL_MARKER still in window"),
        ])
        .unwrap();
        s.apply_compact_checkpoint_from(&user_text("[summary] prior archived"), Some(2), 10)
            .unwrap();
        drop(s);

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
        assert!(out.contains(&a), "{out}");
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
        let hit = out
            .lines()
            .find(|l| l.starts_with('L') && l.contains("SEARCH_READ_ALIGN_TOKEN"))
            .expect(&format!("hit line missing:\n{out}"));
        let line: u32 = hit
            .trim_start_matches('L')
            .split(':')
            .next()
            .unwrap()
            .parse()
            .expect(hit);
        let path = crate::session::transcript_file::virtual_path_for(&sid);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let read = rt.block_on(crate::tools::read::ReadTool::default().execute(
            serde_json::json!({
                "file_path": path,
                "start_line": line,
                "end_line": line,
            }),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: String::new(),
            },
        ));
        assert_eq!(read.level, ToolSignalLevel::Ok, "{}", read.content);
        assert!(
            read.content.contains("SEARCH_READ_ALIGN_TOKEN"),
            "read at L{line} should contain the hit:\n{}",
            read.content
        );
    }

    #[test]
    fn schema_is_query_session_id_offset_only() {
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let schema = tool.schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("session_id"));
        assert!(props.contains_key("offset"));
        assert!(!props.contains_key("session_filter"));
        assert!(!props.contains_key("expand"));
        assert_eq!(schema["additionalProperties"], false);
    }
}
