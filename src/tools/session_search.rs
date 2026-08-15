//! Agent `session_search` — past session transcripts (Matches + Related columns).

use std::collections::HashSet;
use std::sync::Mutex;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::session_search::{
    self, ContextWindowExclude, DEFAULT_EXPAND_DOWN, DEFAULT_EXPAND_UP, EXPAND_STEP_CHARS,
    ExpandSteps, HitWindow, RELATED_ENTRY_MAX_CHARS, RESULTS_PER_PAGE, SessionTextHit,
};
use crate::engines::{RetrievalFilters, WorkspaceEngines};
use crate::tool::Tool;
use crate::types::ToolCallResult;

pub struct SessionSearchTool {
    engines: WorkspaceEngines,
    active_session_id: Mutex<Option<String>>,
}

impl SessionSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self {
            engines,
            active_session_id: Mutex::new(None),
        }
    }

    fn active_session(&self) -> Option<String> {
        self.active_session_id.lock().unwrap().clone()
    }

    fn search_in_workspace(
        &self,
        workspace_root: &std::path::Path,
        query: &str,
        session_filter: Option<&str>,
        offset: usize,
        expand: ExpandSteps,
    ) -> ToolCallResult {
        let db = session_search::sessions_db_under(workspace_root);
        let active = self.active_session();

        let tokens = match session_filter {
            Some(f) if !f.trim().is_empty() => match session_search::parse_filter_tokens(f) {
                Ok(t) => t,
                Err(e) => return ToolCallResult::error(e.to_string()),
            },
            _ => session_search::SessionFilterTokens::default(),
        };

        let include_session = match tokens.include_session {
            Some(refer) => match session_search::resolve_session_ref(&db, &refer) {
                Ok(id) => Some(id),
                Err(e) => return ToolCallResult::error(e.to_string()),
            },
            None => None,
        };

        let mut exclude_session_ids = Vec::new();
        for refer in &tokens.exclude_sessions {
            match session_search::resolve_session_ref(&db, refer) {
                Ok(id) => exclude_session_ids.push(id),
                Err(e) => return ToolCallResult::error(e.to_string()),
            }
        }
        if tokens.exclude_current {
            match active.as_ref() {
                Some(sid) => {
                    if !exclude_session_ids.iter().any(|e| e == sid) {
                        exclude_session_ids.push(sid.clone());
                    }
                }
                None => {
                    return ToolCallResult::error(
                        "session_filter -current requires an active session",
                    );
                }
            }
        }

        let exclude_context_window = match active.as_ref() {
            Some(sid)
                if !exclude_session_ids.iter().any(|e| e == sid)
                    && include_session.as_ref().map(|i| i == sid).unwrap_or(true) =>
            {
                match session_search::load_kept_from_seq(&db, sid) {
                    Ok(Some(kept)) => Some(ContextWindowExclude {
                        session_id: sid.clone(),
                        kept_from_seq: kept,
                    }),
                    Ok(None) => None,
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
                exclude_session_ids,
                exclude_context_window: exclude_context_window.clone(),
                ..Default::default()
            },
            Some(workspace_root.to_path_buf()),
        ) {
            Ok(b) => b,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let related_hits = bundle.semantic_hits.clone().unwrap_or_default();
        if bundle.text_hits.is_empty() && related_hits.is_empty() {
            return ToolCallResult::ok(format!(
                "No matching session transcript context for query '{query}'."
            ));
        }

        let related_unique = related_hits_without_match_bodies(&bundle.text_hits, related_hits);

        let match_windows = match session_search::expand_hit_windows(
            &db,
            &bundle.text_hits,
            expand,
            exclude_context_window.as_ref(),
        ) {
            Ok(w) => w,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };
        // Related: entry-level truncate (ignore expand; no fake char nucleus).
        let related_windows = if related_unique.is_empty() {
            Vec::new()
        } else {
            match session_search::related_entry_windows(&db, &related_unique) {
                Ok(w) => w,
                Err(e) => return ToolCallResult::error(e.to_string()),
            }
        };

        let mut session_ids = Vec::new();
        for w in match_windows.iter().chain(related_windows.iter()) {
            if !session_ids.contains(&w.hit.session_id) {
                session_ids.push(w.hit.session_id.clone());
            }
        }
        let meta = session_search::load_session_meta(&db, &session_ids).unwrap_or_default();

        ToolCallResult::ok(format_dual_view(
            &match_windows,
            &related_windows,
            &meta,
            offset,
            bundle.text_has_more,
            bundle.semantic_has_more && bundle.semantic_hits.is_some(),
        ))
    }
}

impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to find in past session transcripts. The current session's live context window is never searched."
                },
                "session_filter": {
                    "type": "string",
                    "description": "Optional scope (space-separated): session:<ref>, -session:<ref>, -current. <ref> is full id or short suffix from hit headers. Examples: \"-current\" | \"session:G5FAVXYZ\""
                },
                "offset": {
                    "type": "integer",
                    "description": format!(
                        "0-based hit offset for pagination (default 0). Advances Matches and Related together ({RESULTS_PER_PAGE} hits per page per column)."
                    )
                },
                "expand": {
                    "type": "array",
                    "items": { "type": "integer", "minimum": 0 },
                    "minItems": 2,
                    "maxItems": 2,
                    "description": format!(
                        "Matches only: character-window expand [up, down] around each hit (step={EXPAND_STEP_CHARS} chars). Default [{DEFAULT_EXPAND_UP}, {DEFAULT_EXPAND_DOWN}]. Related shows truncated entries (max {RELATED_ENTRY_MAX_CHARS} chars) and ignores expand. Narrow with session_filter before raising expand."
                    )
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn set_active_session(&self, session_id: String) {
        *self.active_session_id.lock().unwrap() = Some(session_id);
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolCallResult::error(e),
        };
        let session_filter = input["session_filter"]
            .as_str()
            .filter(|s| !s.trim().is_empty());
        let offset = match parse_offset(&input["offset"]) {
            Ok(o) => o,
            Err(msg) => return ToolCallResult::error(msg),
        };
        let expand = match parse_expand(&input["expand"]) {
            Ok(e) => e,
            Err(msg) => return ToolCallResult::error(msg),
        };

        self.search_in_workspace(
            &crate::config::workspace::workspace_root_lap(),
            query.trim(),
            session_filter,
            offset,
            expand,
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        format!(
            "Search past conversation transcripts in this workspace. \
             Live context-window turns of the current session are always excluded. \
             Scope with session_filter (session:<ref>, -session:<ref>, -current). \
             Matches: deepen with expand [up,down] (default [{DEFAULT_EXPAND_UP},{DEFAULT_EXPAND_DOWN}], step={EXPAND_STEP_CHARS}). \
             Related (when available): truncated full entries (max {RELATED_ENTRY_MAX_CHARS} chars), not expand windows."
        )
    }

    fn timeout(&self) -> Option<u64> {
        Some(30)
    }

    fn max_result_size(&self) -> usize {
        // Soft-cap view first; keep executor hard-cap aligned (~32KB).
        8_000
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

fn parse_expand(v: &Value) -> std::result::Result<ExpandSteps, String> {
    if v.is_null() {
        return Ok(ExpandSteps::default());
    }
    let Some(arr) = v.as_array() else {
        return Err(crate::tool::must_be(
            "expand",
            "an array of two non-negative integers [up, down]",
        ));
    };
    if arr.len() != 2 {
        return Err(crate::tool::must_be(
            "expand",
            "exactly two elements [up, down]",
        ));
    }
    let up = arr[0]
        .as_u64()
        .ok_or_else(|| crate::tool::must_be("expand[0]", "a non-negative integer"))?
        as usize;
    let down = arr[1]
        .as_u64()
        .ok_or_else(|| crate::tool::must_be("expand[1]", "a non-negative integer"))?
        as usize;
    Ok(ExpandSteps { up, down })
}

fn related_hits_without_match_bodies(
    matches: &[SessionTextHit],
    related: Vec<SessionTextHit>,
) -> Vec<SessionTextHit> {
    let match_keys: HashSet<(String, i64)> = matches
        .iter()
        .map(|h| (h.session_id.clone(), h.seq))
        .collect();
    related
        .into_iter()
        .filter(|h| !match_keys.contains(&(h.session_id.clone(), h.seq)))
        .collect()
}

fn format_dual_view(
    matches: &[HitWindow],
    related: &[HitWindow],
    meta: &std::collections::HashMap<String, session_search::SessionMeta>,
    offset: usize,
    matches_has_more: bool,
    related_has_more: bool,
) -> String {
    let mut parts = Vec::new();
    let mut footers = Vec::new();
    if !matches.is_empty() {
        parts.push("## Matches".to_string());
        parts.push(format_column(matches, meta));
        if matches_has_more {
            footers.push(format!(
                "(more Matches; use offset: {} (both columns))",
                offset + RESULTS_PER_PAGE
            ));
        }
    }
    if !related.is_empty() {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push("## Related".to_string());
        parts.push(format_column(related, meta));
        if related_has_more {
            footers.push(format!(
                "(more Related; use offset: {} (both columns))",
                offset + RESULTS_PER_PAGE
            ));
        }
    }
    if offset > 0 && !matches_has_more && !related_has_more {
        footers.push(format!(
            "(showing hits from offset {offset} (both columns); no further pages)"
        ));
    }
    cap_agent_view(parts.join("\n"), &footers)
}

/// Keep pagination footers when soft-truncating large dual-column output.
fn cap_agent_view(body: String, footers: &[String]) -> String {
    use session_search::AGENT_VIEW_SOFT_BYTES;
    let footer = if footers.is_empty() {
        String::new()
    } else {
        format!("\n{}", footers.join("\n"))
    };
    let notice =
        "\n... [view truncated; narrow session_filter, lower expand, or paginate with offset]";
    let total = body.len().saturating_add(footer.len());
    if total <= AGENT_VIEW_SOFT_BYTES {
        return format!("{body}{footer}");
    }
    let reserve = footer.len().saturating_add(notice.len());
    let keep = AGENT_VIEW_SOFT_BYTES.saturating_sub(reserve);
    let end = body.floor_char_boundary(keep);
    format!("{}{}{}", &body[..end], notice, footer)
}

fn format_column(
    windows: &[HitWindow],
    meta: &std::collections::HashMap<String, session_search::SessionMeta>,
) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<&HitWindow>> =
        std::collections::HashMap::new();
    for w in windows {
        if !groups.contains_key(&w.hit.session_id) {
            order.push(w.hit.session_id.clone());
        }
        groups.entry(w.hit.session_id.clone()).or_default().push(w);
    }

    let mut parts = Vec::new();
    for sid in &order {
        let Some(hits) = groups.get(sid) else {
            continue;
        };
        let (created, updated) = meta
            .get(sid)
            .map(|m| {
                (
                    format_relative_age(m.created_at),
                    format_relative_age(m.updated_at),
                )
            })
            .unwrap_or_else(|| ("?".into(), "?".into()));
        let short = session_search::short_session_ref(sid);
        parts.push(format!(
            "### session:{short} · id:{sid} · created:{created} · updated:{updated}"
        ));
        for (i, w) in hits.iter().enumerate() {
            parts.push(format!(
                "#### hit {} · {} · seq:{}",
                i + 1,
                w.hit.item_type,
                w.hit.seq
            ));
            if let Some(hint) = overflow_hint_above(w) {
                parts.push(hint);
            }
            parts.push(w.body.clone());
            if let Some(hint) = overflow_hint_below(w) {
                parts.push(hint);
            }
        }
    }
    parts.join("\n")
}

fn overflow_hint_above(w: &HitWindow) -> Option<String> {
    if w.items_above > 0 {
        Some(format!("> ... ~{} items above ...", w.items_above))
    } else if w.same_item_overflow && w.chars_above > 0 {
        Some("> ... [earlier in this item] ...".into())
    } else {
        None
    }
}

fn overflow_hint_below(w: &HitWindow) -> Option<String> {
    if w.items_below > 0 {
        Some(format!("> ... ~{} items below ...", w.items_below))
    } else if w.same_item_overflow && w.chars_below > 0 {
        // Related truncate and mid-item expand share this path.
        Some("> ... [truncated] ...".into())
    } else {
        None
    }
}

/// Coarse relative age for agents (m / h / d). Absolute clocks are rarely actionable without "now".
fn format_relative_age(ms: i64) -> String {
    format_relative_age_at(ms, chrono::Utc::now().timestamp_millis())
}

fn format_relative_age_at(ms: i64, now_ms: i64) -> String {
    if ms <= 0 {
        return "?".into();
    }
    let secs = (now_ms.saturating_sub(ms).max(0)) / 1000;
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;
    use crate::config::workspace::{clear_runtime_paths, set_runtime_paths};
    use crate::session::store::Session;
    use crate::types::user_text;
    use std::sync::Mutex;

    use crate::types::ToolSignalLevel;

    static PATHS_LOCK: Mutex<()> = Mutex::new(());

    fn with_workspace<R>(root: &std::path::Path, f: impl FnOnce(&SessionSearchTool) -> R) -> R {
        let _guard = PATHS_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        set_runtime_paths(WorkspacePaths::for_legacy_root(root));
        let tool = SessionSearchTool::new(WorkspaceEngines::new());
        let out = f(&tool);
        clear_runtime_paths();
        out
    }

    fn call_ok(tool: &SessionSearchTool, root: &std::path::Path, input: Value) -> String {
        let r = tool.search_in_workspace(
            root,
            input["query"].as_str().unwrap_or(""),
            input["session_filter"]
                .as_str()
                .filter(|s| !s.trim().is_empty()),
            parse_offset(&input["offset"]).unwrap_or(0),
            parse_expand(&input["expand"]).unwrap_or_default(),
        );
        assert_eq!(
            r.level,
            ToolSignalLevel::Ok,
            "expected ok, got {:?}: {}",
            r.level,
            r.content
        );
        r.content
    }

    fn call_err(tool: &SessionSearchTool, root: &std::path::Path, input: Value) -> String {
        let r = tool.search_in_workspace(
            root,
            input["query"].as_str().unwrap_or(""),
            input["session_filter"]
                .as_str()
                .filter(|s| !s.trim().is_empty()),
            parse_offset(&input["offset"]).unwrap_or(0),
            parse_expand(&input["expand"]).unwrap_or_default(),
        );
        assert_eq!(
            r.level,
            ToolSignalLevel::Error,
            "expected error, got ok: {}",
            r.content
        );
        r.content
    }

    fn call_err_via_inner(tool: &SessionSearchTool, input: Value) -> String {
        let r = tool.call_inner(input);
        assert_eq!(
            r.level,
            ToolSignalLevel::Error,
            "expected error, got ok: {}",
            r.content
        );
        r.content
    }

    fn header_for(sid: &str) -> String {
        format!(
            "### session:{} · id:{sid}",
            session_search::short_session_ref(sid)
        )
    }

    #[test]
    fn typical_broad_search_renders_session_hits_and_bold_core() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();

        let a = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        a.insert_detail_rows(&[user_text(
            "We decided AUTH_REFACTOR_TOKEN moves login to middleware.",
        )])
        .unwrap();
        let id_a = a.id.clone();
        drop(a);

        let b = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        b.insert_detail_rows(&[user_text("unrelated OTHER_TOPIC_ONLY content")])
            .unwrap();
        drop(b);

        with_workspace(root, |tool| {
            let out = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "AUTH_REFACTOR_TOKEN" }),
            );
            assert!(out.contains("## Matches"), "Matches column missing:\n{out}");
            assert!(
                !out.contains("## Related"),
                "cold broad search must omit Related:\n{out}"
            );
            assert!(
                out.contains(&header_for(&id_a)),
                "session header missing:\n{out}"
            );
            assert!(out.contains(" · created:"), "created missing:\n{out}");
            assert!(out.contains(" · updated:"), "updated missing:\n{out}");
            assert!(out.contains("#### hit 1 ·"), "hit heading missing:\n{out}");
            assert!(
                out.contains(&format!(
                    "{}AUTH_REFACTOR_TOKEN{}",
                    session_search::HIT_MARK_OPEN,
                    session_search::HIT_MARK_CLOSE
                )),
                "hit mark missing:\n{out}"
            );
            assert!(
                !out.contains("OTHER_TOPIC_ONLY"),
                "unrelated session leaked:\n{out}"
            );
            eprintln!("--- broad search output ---\n{out}\n---");
        });
    }

    #[test]
    fn typical_narrow_then_expand_shows_cross_item_context() {
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
        // Short neighbors + long padding around the needle so default 1-step
        // expand still reaches adjacent items via the char stream.
        let mid = format!("{}CROSS_HIT_MARKER{}", "x".repeat(50), "y".repeat(50));
        s.insert_detail_rows(&[
            user_text("PREFIX_NEIGHBOR_ALPHA unique preamble"),
            user_text(&mid),
            user_text("SUFFIX_NEIGHBOR_OMEGA unique epilogue"),
        ])
        .unwrap();
        drop(s);

        with_workspace(root, |tool| {
            let narrow = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "CROSS_HIT_MARKER",
                    "session_filter": format!("session:{sid}"),
                    "expand": [0, 0]
                }),
            );
            assert!(
                narrow.contains(&header_for(&sid)),
                "narrow header:\n{narrow}"
            );
            assert!(
                narrow.contains(&format!(
                    "{}CROSS_HIT_MARKER{}",
                    session_search::HIT_MARK_OPEN,
                    session_search::HIT_MARK_CLOSE
                )),
                "narrow core:\n{narrow}"
            );
            assert!(
                !narrow.contains("PREFIX_NEIGHBOR_ALPHA"),
                "expand 0 should not pull previous item:\n{narrow}"
            );

            let deep = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "CROSS_HIT_MARKER",
                    "session_filter": format!("session:{sid}"),
                    "expand": [2, 2]
                }),
            );
            assert!(
                deep.contains("PREFIX_NEIGHBOR_ALPHA"),
                "expand should reach previous item:\n{deep}"
            );
            assert!(
                deep.contains("SUFFIX_NEIGHBOR_OMEGA"),
                "expand should reach next item:\n{deep}"
            );
            assert!(
                deep.contains("~") && deep.contains("items above")
                    || deep.contains("[earlier in this item]")
                    || deep.contains("PREFIX_NEIGHBOR"),
                "expected above context:\n{deep}"
            );
            eprintln!("--- expand[2,2] output ---\n{deep}\n---");
        });
    }

    #[test]
    fn typical_multi_hit_same_session_lists_hit_blocks() {
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

        with_workspace(root, |tool| {
            let out = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "MULTI_HIT_TOKEN",
                    "session_filter": format!("session:{sid}"),
                    "expand": [0, 0]
                }),
            );
            // Overlap merge may collapse windows; still expect session header
            // and at least one bolded core. Prefer two hit blocks when separate.
            assert!(out.contains(&header_for(&sid)), "\n{out}");
            assert!(
                out.contains(&format!(
                    "{}MULTI_HIT_TOKEN{}",
                    session_search::HIT_MARK_OPEN,
                    session_search::HIT_MARK_CLOSE
                )),
                "\n{out}"
            );
            let hit_headings = out.matches("#### hit ").count();
            assert!(
                hit_headings >= 1,
                "expected at least one hit block, got {hit_headings}:\n{out}"
            );
            eprintln!("--- multi-hit output ---\n{out}\n---");
        });
    }

    #[test]
    fn typical_pagination_footer_guides_next_offset() {
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
        for i in 0..12 {
            items.push(user_text(format!("PAGE_TOKEN_{i:02} shared PAGE_NEEDLE")));
        }
        s.insert_detail_rows(&items).unwrap();
        drop(s);

        with_workspace(root, |tool| {
            let page0 = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "PAGE_NEEDLE", "expand": [0, 0] }),
            );
            assert!(
                page0.contains(&format!(
                    "more Matches; use offset: {RESULTS_PER_PAGE} (both columns)"
                )),
                "expected next-offset footer:\n{page0}"
            );

            let page1 = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "PAGE_NEEDLE",
                    "offset": RESULTS_PER_PAGE,
                    "expand": [0, 0]
                }),
            );
            assert!(
                page1.contains("no further pages") || !page1.contains("use offset:"),
                "last page should not advertise another offset:\n{page1}"
            );
        });
    }

    #[test]
    fn rejects_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        with_workspace(root, |tool| {
            let err = call_err_via_inner(tool, serde_json::json!({ "query": "   " }));
            assert!(err.contains("query"), "{err}");
        });
    }

    #[test]
    fn active_session_context_window_is_excluded_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();

        let current = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        current
            .insert_detail_rows(&[user_text("VISIBLE_IN_WINDOW_MARKER only here")])
            .unwrap();
        let current_id = current.id.clone();
        drop(current);

        let other = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        other
            .insert_detail_rows(&[user_text("VISIBLE_IN_WINDOW_MARKER also in other")])
            .unwrap();
        let other_id = other.id.clone();
        drop(other);

        with_workspace(root, |tool| {
            tool.set_active_session(current_id.clone());
            let out = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "VISIBLE_IN_WINDOW_MARKER" }),
            );
            assert!(
                out.contains(&header_for(&other_id)),
                "other session should hit:\n{out}"
            );
            assert!(
                !out.contains(&current_id),
                "current live window must not echo:\n{out}"
            );
        });
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
        // Keep only the last detail in the live window.
        let keep_from = 2_i64;
        s.apply_compact_checkpoint_from(
            &user_text("[summary] prior archived"),
            Some(keep_from),
            10,
        )
        .unwrap();
        drop(s);

        with_workspace(root, |tool| {
            tool.set_active_session(sid.clone());
            let archived = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "ARCHIVED_OLD_MARKER", "expand": [0, 0] }),
            );
            assert!(
                archived.contains(&format!(
                    "{}ARCHIVED_OLD_MARKER{}",
                    session_search::HIT_MARK_OPEN,
                    session_search::HIT_MARK_CLOSE
                )),
                "archived detail should still search:\n{archived}"
            );

            let live = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "LIVE_TAIL_MARKER", "expand": [0, 0] }),
            );
            assert!(
                live.contains("No matching session transcript"),
                "live window must stay excluded:\n{live}"
            );
        });
    }

    #[test]
    fn filter_minus_current_excludes_entire_active_session_including_archive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();

        let current = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let current_id = current.id.clone();
        current
            .insert_detail_rows(&[
                user_text("SHARED_FILTER_MARKER archived"),
                user_text("tail"),
            ])
            .unwrap();
        current
            .apply_compact_checkpoint_from(&user_text("sum"), Some(1), 5)
            .unwrap();
        drop(current);

        let other = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let other_id = other.id.clone();
        other
            .insert_detail_rows(&[user_text("SHARED_FILTER_MARKER elsewhere")])
            .unwrap();
        drop(other);

        with_workspace(root, |tool| {
            tool.set_active_session(current_id.clone());
            let out = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "SHARED_FILTER_MARKER",
                    "session_filter": "-current"
                }),
            );
            assert!(out.contains(&header_for(&other_id)), "\n{out}");
            assert!(!out.contains(&current_id), "\n{out}");
        });
    }

    #[test]
    fn filter_session_prefix_and_exclude_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();

        let a = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let id_a = a.id.clone();
        a.insert_detail_rows(&[user_text("SCOPE_MARKER_AAA")])
            .unwrap();
        drop(a);

        let b = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        let id_b = b.id.clone();
        b.insert_detail_rows(&[user_text("SCOPE_MARKER_BBB")])
            .unwrap();
        drop(b);

        with_workspace(root, |tool| {
            let short_a = session_search::short_session_ref(&id_a);
            let only_a = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "SCOPE_MARKER",
                    "session_filter": format!("session:{short_a}")
                }),
            );
            assert!(only_a.contains(&header_for(&id_a)), "\n{only_a}");
            assert!(!only_a.contains(&id_b), "\n{only_a}");

            let short_b = session_search::short_session_ref(&id_b);
            let without_b = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "SCOPE_MARKER",
                    "session_filter": format!("-session:{short_b}")
                }),
            );
            assert!(without_b.contains(&header_for(&id_a)), "\n{without_b}");
            assert!(!without_b.contains(&id_b), "\n{without_b}");
        });
    }

    #[test]
    fn session_filter_short_ref_resolves_and_unknown_token_errors() {
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
        s.insert_detail_rows(&[user_text("PREFIX_RESOLVE_MARKER")])
            .unwrap();
        drop(s);

        with_workspace(root, |tool| {
            let short = session_search::short_session_ref(&sid);
            let out = call_ok(
                tool,
                root,
                serde_json::json!({
                    "query": "PREFIX_RESOLVE_MARKER",
                    "session_filter": format!("session:{short}")
                }),
            );
            assert!(out.contains(&header_for(&sid)), "\n{out}");

            let err = call_err(
                tool,
                root,
                serde_json::json!({
                    "query": "PREFIX_RESOLVE_MARKER",
                    "session_filter": "bogus:token"
                }),
            );
            assert!(err.contains("unknown session_filter"), "{err}");
        });
    }

    #[test]
    fn relative_age_uses_m_h_d_buckets() {
        let now = 1_700_000_000_000i64;
        assert_eq!(format_relative_age_at(now - 30_000, now), "just now");
        assert_eq!(format_relative_age_at(now - 5 * 60_000, now), "5m ago");
        assert_eq!(format_relative_age_at(now - 3 * 3_600_000, now), "3h ago");
        assert_eq!(format_relative_age_at(now - 2 * 86_400_000, now), "2d ago");
    }

    #[test]
    fn overflow_hints_prefer_item_counts_over_char_dumps() {
        let mut w = hit_window("SIDAAAAAXXXXXXXX", 0, "body");
        w.items_above = 12;
        w.chars_above = 93_966;
        assert_eq!(
            overflow_hint_above(&w).as_deref(),
            Some("> ... ~12 items above ...")
        );
        w.items_above = 0;
        w.same_item_overflow = true;
        assert_eq!(
            overflow_hint_above(&w).as_deref(),
            Some("> ... [earlier in this item] ...")
        );
        w.chars_below = 500;
        w.items_below = 0;
        assert_eq!(
            overflow_hint_below(&w).as_deref(),
            Some("> ... [truncated] ...")
        );
    }

    fn hit_window(sid: &str, seq: i64, body: &str) -> HitWindow {
        HitWindow {
            hit: SessionTextHit {
                session_id: sid.into(),
                seq,
                item_type: "message".into(),
                summary: body.into(),
                score: 1.0,
                char_start: 0,
                char_end: body.chars().count(),
                lane: session_search::SessionHitLane::Text,
            },
            body: body.into(),
            chars_above: 0,
            chars_below: 0,
            items_above: 0,
            items_below: 0,
            same_item_overflow: false,
        }
    }

    #[test]
    fn view_matches_only_omits_related_section() {
        let meta = std::collections::HashMap::new();
        let out = format_dual_view(
            &[hit_window("SIDAAAAAXXXXXXXX", 0, "⟦alpha⟧")],
            &[],
            &meta,
            0,
            false,
            false,
        );
        assert!(out.starts_with("## Matches"), "{out}");
        assert!(!out.contains("## Related"), "{out}");
        assert!(out.contains("⟦alpha⟧"), "{out}");
        assert!(out.contains("#### hit 1 · message · seq:0"), "{out}");
        assert!(!out.contains("FTS"), "{out}");
        assert!(!out.contains("BM25"), "{out}");
        assert!(!out.contains("ANN"), "{out}");
        assert!(!out.contains("hybrid"), "{out}");
        assert!(!out.contains("lane"), "{out}");
        assert!(!out.contains("engine"), "{out}");
    }

    #[test]
    fn view_related_only_omits_matches_section() {
        let meta = std::collections::HashMap::new();
        let out = format_dual_view(
            &[],
            &[hit_window("SIDBBBBBXXXXXXXX", 1, "beta-entry")],
            &meta,
            0,
            false,
            false,
        );
        assert!(!out.contains("## Matches"), "{out}");
        assert!(out.starts_with("## Related"), "{out}");
        assert!(out.contains("beta-entry"), "{out}");
        assert!(out.contains("#### hit 1 · message · seq:1"), "{out}");
    }

    #[test]
    fn view_both_columns_matches_then_related() {
        let meta = std::collections::HashMap::new();
        let out = format_dual_view(
            &[hit_window("SIDAAAAAXXXXXXXX", 0, "⟦match-body⟧")],
            &[hit_window("SIDBBBBBXXXXXXXX", 2, "related-body")],
            &meta,
            0,
            false,
            false,
        );
        let i_m = out.find("## Matches").expect("Matches");
        let i_r = out.find("## Related").expect("Related");
        assert!(i_m < i_r, "Matches must precede Related:\n{out}");
        assert!(
            out.contains("⟦match-body⟧") && out.contains("related-body"),
            "{out}"
        );
    }

    #[test]
    fn view_soft_cap_preserves_pagination_footers() {
        let meta = std::collections::HashMap::new();
        let huge = "X".repeat(session_search::AGENT_VIEW_SOFT_BYTES + 2_000);
        let out = format_dual_view(
            &[hit_window("SIDAAAAAXXXXXXXX", 0, &huge)],
            &[hit_window("SIDBBBBBXXXXXXXX", 1, "r")],
            &meta,
            0,
            true,
            true,
        );
        assert!(out.contains("view truncated"), "{out}");
        assert!(out.contains("more Matches; use offset:"), "{out}");
        assert!(out.contains("more Related; use offset:"), "{out}");
    }

    #[test]
    fn view_pagination_footers_are_per_column() {
        let meta = std::collections::HashMap::new();
        let out = format_dual_view(
            &[hit_window("SIDAAAAAXXXXXXXX", 0, "m")],
            &[hit_window("SIDBBBBBXXXXXXXX", 1, "r")],
            &meta,
            0,
            true,
            true,
        );
        assert!(out.contains("more Matches; use offset:"), "{out}");
        assert!(out.contains("more Related; use offset:"), "{out}");
        assert!(out.contains("(both columns)"), "{out}");
        assert!(!out.contains("view truncated"), "{out}");
    }

    #[test]
    fn view_offset_exhausted_footer_when_no_more() {
        let meta = std::collections::HashMap::new();
        let out = format_dual_view(
            &[hit_window("SIDAAAAAXXXXXXXX", 0, "m")],
            &[],
            &meta,
            RESULTS_PER_PAGE,
            false,
            false,
        );
        assert!(
            out.contains(&format!("showing hits from offset {RESULTS_PER_PAGE}")),
            "{out}"
        );
    }

    #[test]
    fn related_dedupe_skips_seqs_already_in_matches() {
        let matches = vec![SessionTextHit {
            session_id: "s1".into(),
            seq: 3,
            item_type: "message".into(),
            summary: "shared".into(),
            score: 1.0,
            char_start: 0,
            char_end: 6,
            lane: session_search::SessionHitLane::Text,
        }];
        let related = vec![
            SessionTextHit {
                session_id: "s1".into(),
                seq: 3,
                item_type: "message".into(),
                summary: "dup".into(),
                score: 0.9,
                char_start: 0,
                char_end: 3,
                lane: session_search::SessionHitLane::Semantic,
            },
            SessionTextHit {
                session_id: "s1".into(),
                seq: 4,
                item_type: "message".into(),
                summary: "only-related".into(),
                score: 0.8,
                char_start: 0,
                char_end: 4,
                lane: session_search::SessionHitLane::Semantic,
            },
        ];
        let kept = related_hits_without_match_bodies(&matches, related);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].seq, 4);
    }

    #[test]
    fn cold_engine_view_has_matches_without_related() {
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
        s.insert_detail_rows(&[user_text("COLD_VIEW_MARKER only lexical")])
            .unwrap();
        drop(s);

        with_workspace(root, |tool| {
            assert!(!tool.engines.is_warmed("code_search"));
            let out = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "COLD_VIEW_MARKER", "expand": [0, 0] }),
            );
            assert!(out.contains("## Matches"), "{out}");
            assert!(
                !out.contains("## Related"),
                "cold must omit Related:\n{out}"
            );
            assert!(
                out.contains(&format!(
                    "{}COLD_VIEW_MARKER{}",
                    session_search::HIT_MARK_OPEN,
                    session_search::HIT_MARK_CLOSE
                )),
                "{out}"
            );
            for banned in ["FTS", "BM25", "ANN", "hybrid", "Warm", "engine cold"] {
                assert!(!out.contains(banned), "leaked '{banned}' in:\n{out}");
            }
        });
    }

    #[test]
    fn empty_result_message_has_no_column_headings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let _ = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();

        with_workspace(root, |tool| {
            let out = call_ok(
                tool,
                root,
                serde_json::json!({ "query": "ZZZ_NO_SUCH_TOKEN_EVER" }),
            );
            assert!(out.contains("No matching session transcript"), "{out}");
            assert!(!out.contains("## Matches"), "{out}");
            assert!(!out.contains("## Related"), "{out}");
        });
    }
}
