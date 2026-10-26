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
        // `serve::router::listen`, and the sparse ledger by the refresh a search
        // dispatches behind it — never in front of the query that needs it.
        // Refreshing inline put a reconcile in front of the query that asked for
        // it, and left the lane unusable whenever the two overlapped — a session
        // corpus moves on every turn, so "refresh on demand" meant "refresh
        // always, right here". A stale index is the accepted price; being wrong
        // is not.
        let semantic_ready = matches!(
            self.engines.session_semantic_gate(),
            crate::engines::CodeSearchCallGate::Ready
        );

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

        let sync = crate::tools::sync_pct_for_workspace(&self.engines, Some(workspace_root));
        let mut body = String::new();
        if !semantic_ready {
            // Honest degradation: never look like a full lexical+semantic fusion.
            body.push_str("semantic: unavailable (lexical only)\n");
        }
        if view.trim().is_empty() {
            body.push_str(&format!(
                "No matching session transcript context for query '{query}'."
            ));
        } else {
            body.push_str(&view);
        }
        ToolCallResult::ok(crate::tools::prepend_sync_pct(sync.as_ref(), &body))
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
                    "description": "What to find in past session transcripts. Literal words or a phrase. A multi-word query matches rows that contain most of those words (a literal phrase matches verbatim). Separate alternatives with | (any may match, e.g. 'retry|重试'); matching is case-insensitive and literal, so separators count. The current session's live context window is never searched."
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
        crate::tools::description_text(include_str!("descriptions/session_search.md"))
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(30))
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

    /// Enable retrieval + mark code_search Warm so session_search is callable in tests.
    fn warmed_tool(root: &std::path::Path) -> SessionSearchTool {
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("code_search", crate::engines::EngineState::Warm);
        SessionSearchTool::new(engines)
    }

    #[test]
    fn cold_semantic_engine_still_returns_lexical_hits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_session(root, "COLD_LEXICAL_ONLY_TOKEN");
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "COLD_LEXICAL_ONLY_TOKEN" }),
            "",
        );
        assert!(out.contains("COLD_LEXICAL_ONLY_TOKEN"), "{out}");
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

        let tool = warmed_tool(root);
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
        let location = crate::engines::session_search::named_spill_location(&out).expect(&out);
        assert!(
            location.starts_with(".litecode/bash/session_search_"),
            "{out}"
        );
        let spilled = std::fs::read_to_string(root.join(location)).unwrap();
        assert!(spilled.contains("Remaining "), "{spilled}");
        assert!(spilled.contains("PAGE_NEEDLE"), "{spilled}");
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
        data.compact_from(&sid, "[summary] prior archived", Some(2), 10)
            .unwrap();

        let tool = warmed_tool(root);
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
    fn search_hit_line_matches_virtual_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nSEARCH_READ_ALIGN_TOKEN here\ndelta");
        let tool = warmed_tool(root);
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
                "path": path,
                "start_line": start_line,
                "limit": end_line - start_line + 1,
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
        let data = read.data.as_ref().expect("read returns data");
        let lines: Vec<&str> = data["lines"]
            .as_array()
            .expect("data.lines")
            .iter()
            .filter_map(|l| l.as_str())
            .collect();
        assert_eq!(
            lines.len(),
            (end_line - start_line + 1) as usize,
            "read at L{start_line}-{end_line} must cover the hit range: {data}"
        );
        assert!(
            lines.iter().any(|l| l.contains("SEARCH_READ_ALIGN_TOKEN")),
            "read at L{start_line}-{end_line} should contain the hit: {data}"
        );
    }

    #[test]
    fn semantic_unavailable_is_explicit_and_leads_with_sync_pct() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode")).unwrap();
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let id = seed_session(root, "UNIQUE_SESSION_SEARCH_TOKEN alpha");
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("code_search", crate::engines::EngineState::Warm);
        // Warm but no worker_pid ⇒ session_semantic_gate = Wait ⇒ lexical only.
        let tool = SessionSearchTool::new(engines);
        let out = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "UNIQUE_SESSION_SEARCH_TOKEN" }),
            "",
        );
        let first = out.lines().next().unwrap_or("");
        assert!(
            first.starts_with("sync_pct:"),
            "first line must be sync_pct, got: {first:?}\n{out}"
        );
        assert!(
            out.contains("semantic: unavailable (lexical only)"),
            "must declare lexical-only degradation:\n{out}"
        );
        assert!(
            out.contains("UNIQUE_SESSION_SEARCH_TOKEN") || out.contains("No matching"),
            "unexpected body:\n{out}"
        );
        let _ = id;
    }
}
