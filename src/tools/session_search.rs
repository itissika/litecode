//! Agent `session_search` — a flat table over past session transcripts.

use serde::Serialize;
use serde_json::Value;

use crate::engines::session_search::{self, ContextWindowExclude};
use crate::engines::{RetrievalFilters, WorkspaceEngines};
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{ToolCallResult, ToolErrorKind};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;

pub struct SessionSearchTool {
    engines: WorkspaceEngines,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SessionSearchData {
    hits: Vec<session_search::SessionTableHit>,
    total: usize,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
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
        limit: usize,
        limit_note: Option<String>,
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

        let table = match session_search::build_agent_table(reader, &bundle.ranked, limit) {
            Ok(table) => table,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let sync = crate::tools::sync_pct_for_workspace(&self.engines, Some(workspace_root));
        let truncated = table.total > table.hits.len();
        let mut notes = Vec::new();
        if !semantic_ready {
            // Honest degradation: never look like a full lexical+semantic fusion.
            notes.push("semantic: unavailable (lexical only)".to_string());
        }
        if table.hits.is_empty() {
            notes.push(format!(
                "No matching session transcript context for query '{query}'."
            ));
        } else if truncated {
            notes.push(format!(
                "Showing {} of {} hits; narrow the query or raise limit.",
                table.hits.len(),
                table.total
            ));
        }
        if let Some(note) = limit_note {
            notes.push(note);
        }
        let message = (!notes.is_empty()).then(|| notes.join("\n"));
        let content =
            crate::tools::prepend_sync_pct(sync.as_ref(), message.as_deref().unwrap_or(""));
        let data = SessionSearchData {
            hits: table.hits,
            total: table.total,
            truncated,
            message,
        };
        ToolOutcome::ok(data).into_call_result(content)
    }
}

impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
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
        let (limit, limit_note) =
            match crate::tool::clamp::clamp_int(&input, "limit", 1, MAX_LIMIT as u64, "hits") {
                Ok(Some(clamped)) => (clamped.value as usize, clamped.note),
                Ok(None) => (DEFAULT_LIMIT, None),
                Err(error) => {
                    return ToolCallResult::error_with_kind(
                        ToolErrorKind::InvalidInput,
                        error,
                        false,
                    )
                    .with_error_code("invalid_input");
                }
            };
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
            limit,
            limit_note,
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

    fn call_ok(
        tool: &SessionSearchTool,
        root: &std::path::Path,
        input: Value,
        active: &str,
    ) -> ToolCallResult {
        let r = call(tool, root, input, active);
        assert_eq!(
            r.level,
            ToolSignalLevel::Ok,
            "expected ok, got {:?}: {}",
            r.level,
            r.content
        );
        r
    }

    fn hits(result: &ToolCallResult) -> Vec<&Value> {
        result.data.as_ref().expect("session_search returns data")["hits"]
            .as_array()
            .expect("data.hits")
            .iter()
            .collect()
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
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "COLD_LEXICAL_ONLY_TOKEN" }),
            "",
        );
        let rows = hits(&result);
        assert!(!rows.is_empty(), "{:?}", result.data);
        let row = &rows[0];
        assert_eq!(row["role"], "conversation");
        assert!(
            row["summary"]
                .as_str()
                .unwrap()
                .contains("COLD_LEXICAL_ONLY_TOKEN")
        );
        let via = row["via"].as_str().unwrap();
        assert!(
            via.split('+').any(|part| part == "lex" || part == "exact"),
            "{via}"
        );
        assert!(
            result
                .content
                .contains("semantic: unavailable (lexical only)")
        );
    }

    #[test]
    fn limit_is_applied_after_total_and_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let items: Vec<_> = (0..25)
            .map(|i| user_text(format!("LIMIT_TOKEN {i:02} with shared needle")))
            .collect();
        seed_items(root, &items);
        let tool = warmed_tool(root);
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "LIMIT_TOKEN", "limit": 5 }),
            "",
        );
        let rows = hits(&result);
        assert_eq!(rows.len(), 5, "{:?}", result.data);
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["total"], 25);
        assert_eq!(data["truncated"], true);
        assert!(
            data["message"]
                .as_str()
                .unwrap()
                .contains("Showing 5 of 25")
        );
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
            hits(&archived).iter().any(|row| row["summary"]
                .as_str()
                .unwrap()
                .contains("ARCHIVED_OLD_MARKER")),
            "archived detail should still search: {:?}",
            archived.data
        );
        let live = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "LIVE_TAIL_MARKER" }),
            &sid,
        );
        assert!(
            live.data.as_ref().unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("No matching session transcript"),
            "live window must stay excluded: {:?}",
            live.data
        );
    }

    #[test]
    fn search_hit_line_matches_virtual_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nSEARCH_READ_ALIGN_TOKEN here\ndelta");
        let tool = warmed_tool(root);
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "SEARCH_READ_ALIGN_TOKEN" }),
            "",
        );
        let rows = hits(&result);
        let line = rows
            .first()
            .and_then(|row| row["line"].as_u64())
            .expect("hit line") as u32;
        let path = transcript_file::virtual_path_for(&sid);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let read = rt.block_on(crate::tools::read::ReadTool::default().execute(
            serde_json::json!({
                "path": path,
                "start_line": line,
                "limit": 1,
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
        assert_eq!(lines.len(), 1, "read at L{line} must cover the hit: {data}");
        assert!(
            lines[0].contains("SEARCH_READ_ALIGN_TOKEN"),
            "read at L{line} should contain the hit: {data}"
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
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "UNIQUE_SESSION_SEARCH_TOKEN" }),
            "",
        );
        let first = result.content.lines().next().unwrap_or("");
        assert!(
            first.starts_with("sync_pct:"),
            "first line must be sync_pct, got: {first:?}\n{}",
            result.content
        );
        assert!(
            result.data.as_ref().unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("semantic: unavailable (lexical only)"),
            "must declare lexical-only degradation: {:?}",
            result.data
        );
        let _ = id;
    }
}
