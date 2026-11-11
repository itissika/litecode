//! Agent `session_search` — a flat table over past session transcripts.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::engines::session_search::{self, ContextWindowExclude};
use crate::engines::{RetrievalFilters, WorkspaceEngines};
use crate::session::SessionService;
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{ToolCallResult, ToolErrorKind};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;

pub struct SessionSearchTool {
    engines: WorkspaceEngines,
    /// Optional shared owner lookup. Tests that only exercise ranking omit it.
    owners: Option<Arc<SessionService>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SessionSearchData {
    hits: Vec<session_search::SessionTableHit>,
    total: usize,
    truncated: bool,
    /// False when the semantic leg was unavailable: the hits are lexical only.
    semantic: bool,
    /// Parameters adjusted to a legal value; absent when nothing moved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    clamped: Vec<crate::tool::clamp::Clamp>,
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    let hit = |bot_name: Option<&str>| session_search::SessionTableHit {
        session: "s1".into(),
        bot_name: bot_name.map(str::to_string),
        age: "1m".into(),
        line: 3,
        role: "conversation".into(),
        via: "lex".into(),
        summary: "hello".into(),
    };
    vec![DriftCase {
        tool: "session_search",
        all_optional_present: ToolOutcome::ok(SessionSearchData {
            hits: vec![hit(Some("bot"))],
            total: 1,
            truncated: false,
            semantic: true,
            clamped: vec![crate::tool::clamp::Clamp {
                parameter: "limit".into(),
                requested: serde_json::json!(500),
                applied: serde_json::json!(MAX_LIMIT),
            }],
        })
        .into_call_result(""),
        all_optional_absent: ToolOutcome::ok(SessionSearchData {
            hits: vec![hit(None)],
            total: 1,
            truncated: false,
            semantic: true,
            clamped: Vec::new(),
        })
        .into_call_result(""),
    }]
}

impl SessionSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self {
            engines,
            owners: None,
        }
    }

    /// Production constructor: enrich hits with the shared bot identity outlet.
    pub fn with_owners(engines: WorkspaceEngines, owners: Arc<SessionService>) -> Self {
        Self {
            engines,
            owners: Some(owners),
        }
    }

    fn search_in_workspace(
        &self,
        workspace_root: &std::path::Path,
        reader: &crate::session::SessionDataReader,
        query: &str,
        session_id: Option<&str>,
        active_session_id: Option<&str>,
        limit: usize,
        limit_clamp: Option<crate::tool::clamp::Clamp>,
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

        let owners = self.owners.as_deref();
        let bot_name = |session_id: &str| {
            owners
                .and_then(|owners| owners.bot_owner_for_session(session_id))
                .map(|owner| owner.name)
        };
        let table =
            match session_search::build_agent_table(reader, &bundle.ranked, limit, &bot_name) {
                Ok(table) => table,
                Err(e) => return ToolCallResult::error(e.to_string()),
            };

        let sync = crate::tools::sync_pct_for_workspace(&self.engines, Some(workspace_root));
        let truncated = table.total > table.hits.len();
        // The result is the fields: an empty table is an empty table, a cut
        // table says `truncated` + `total`, and a lexical-only answer says so.
        let clamped: Vec<_> = limit_clamp.into_iter().collect();
        let content = crate::tools::format_sync_pct_line(sync.as_ref());
        let data = SessionSearchData {
            hits: table.hits,
            total: table.total,
            truncated,
            semantic: semantic_ready,
            clamped,
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
        let (limit, limit_clamp) =
            match crate::tool::clamp::clamp_int(&input, "limit", 1, MAX_LIMIT as u64, "hits") {
                Ok(Some(clamped)) => (clamped.value as usize, clamped.as_clamp("limit")),
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
        let own_session = input["own_session"].as_bool().unwrap_or(false);
        let session_scope = if own_session {
            if session_id.is_some() {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidInput,
                    "own_session searches the calling session; drop session_id".to_string(),
                    false,
                )
                .with_error_code("invalid_input");
            }
            let Some(active) = active else {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidInput,
                    "own_session needs an active session".to_string(),
                    false,
                )
                .with_error_code("invalid_input");
            };
            Some(active.to_string())
        } else {
            session_id.map(str::to_string)
        };
        self.search_in_workspace(
            &execution.workspace_root,
            reader,
            query.trim(),
            session_scope.as_deref(),
            active,
            limit,
            limit_clamp,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::session::transcript_file;
    use crate::session::{SessionBotOwner, SessionData, SessionManager, WorkspaceWriteLease};
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
    fn hit_carries_derived_bot_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let session_id = seed_session(root, "BOT_NAME_TOKEN");
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("code_search", crate::engines::EngineState::Warm);
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let owner = SessionBotOwner {
            session_id: session_id.clone(),
            bot_id: "bot-1".into(),
            name: "Researcher".into(),
            description: "".into(),
        };
        manager.replace_bot_owners([owner.clone()]);
        let tool = SessionSearchTool::with_owners(
            engines,
            Arc::new(SessionService::new(Arc::clone(&manager))),
        );
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "BOT_NAME_TOKEN" }),
            "",
        );
        let rows = hits(&result);
        assert!(!rows.is_empty(), "{:?}", result.data);
        assert_eq!(rows[0]["bot_name"], "Researcher");

        let renamed = SessionBotOwner {
            name: "Renamed".into(),
            ..owner
        };
        manager.upsert_bot_owner(renamed);
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "BOT_NAME_TOKEN" }),
            "",
        );
        let rows = hits(&result);
        assert!(!rows.is_empty(), "{:?}", result.data);
        assert_eq!(rows[0]["bot_name"], "Renamed");

        manager.remove_bot_owner(&session_id);
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "BOT_NAME_TOKEN" }),
            "",
        );
        let rows = hits(&result);
        assert!(!rows.is_empty(), "{:?}", result.data);
        assert!(rows[0].get("bot_name").is_none());
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
        // Lexical-only is a fact on the result, not a sentence in content.
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["semantic"], false);
        assert!(data.get("message").is_none(), "{data}");
        assert!(
            !result.content.contains("semantic: unavailable"),
            "{}",
            result.content
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
        // Cut at the limit is carried by total + truncated; nothing to say.
        assert!(data.get("message").is_none(), "{data}");
    }

    #[test]
    fn over_budget_limit_is_a_clamped_field() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_session(root, "CLAMP_TOKEN alpha");
        let tool = warmed_tool(root);
        let result = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "CLAMP_TOKEN", "limit": 500 }),
            "",
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["clamped"][0]["parameter"], "limit");
        assert_eq!(data["clamped"][0]["requested"], 500);
        assert_eq!(data["clamped"][0]["applied"], MAX_LIMIT);
        assert!(data.get("message").is_none(), "{data}");
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
            hits(&live).is_empty(),
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
        assert_eq!(
            result.data.as_ref().unwrap()["semantic"],
            false,
            "must declare lexical-only degradation: {:?}",
            result.data
        );
        let _ = id;
    }

    #[test]
    fn own_session_searches_the_calling_session_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let mine = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(
            &mine,
            &[
                user_text("OWN_ARCHIVED_MARKER in my own past"),
                user_text("filler"),
                user_text("live tail"),
            ],
        )
        .unwrap();
        data.compact_from(&mine, "[summary] mine", Some(2), 10)
            .unwrap();
        let other = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(
            &other,
            &[
                user_text("OTHER_ARCHIVED_MARKER elsewhere"),
                user_text("other live tail"),
            ],
        )
        .unwrap();
        data.compact_from(&other, "[summary] other", Some(1), 20)
            .unwrap();

        let tool = warmed_tool(root);
        let scoped = call_ok(
            &tool,
            root,
            serde_json::json!({ "query": "ARCHIVED_MARKER", "own_session": true }),
            &mine,
        );
        let rows = hits(&scoped);
        assert!(
            rows.iter().any(|row| row["summary"]
                .as_str()
                .unwrap()
                .contains("OWN_ARCHIVED_MARKER")),
            "own past should be searchable: {:?}",
            scoped.data
        );
        assert!(
            rows.iter().all(|row| !row["summary"]
                .as_str()
                .unwrap()
                .contains("OTHER_ARCHIVED_MARKER")),
            "other sessions must stay out: {:?}",
            scoped.data
        );
    }

    #[test]
    fn own_session_rejects_session_id_and_requires_an_active_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "SCOPE_TOKEN");
        let tool = warmed_tool(root);
        let both = call(
            &tool,
            root,
            serde_json::json!({
                "query": "SCOPE_TOKEN",
                "own_session": true,
                "session_id": sid,
            }),
            &sid,
        );
        assert!(both.data.is_none(), "{:?}", both.data);
        assert!(both.content.contains("own_session"), "{}", both.content);

        let no_active = call(
            &tool,
            root,
            serde_json::json!({ "query": "SCOPE_TOKEN", "own_session": true }),
            "",
        );
        assert!(no_active.data.is_none(), "{:?}", no_active.data);
        assert!(
            no_active.content.contains("active session"),
            "{}",
            no_active.content
        );
    }
}
