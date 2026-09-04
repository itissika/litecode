//! Session corpus — Lexical (always-on) + Semantic (ANN-only when engine Warm).
//!
//! Does not own session writes, schema migration, or ORT lifecycle.

mod lexical;
mod semantic_index;

pub use semantic_index::{
    SessionSemanticIndex, consume_session_index, ensure_session_index, load_session_index,
    queue_session_dirty, read_session_pending_hint, session_index_status, session_should_rebuild,
    session_work_from_disk, write_session_pending_hint,
};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::session::SessionDataReader;
use crate::session::transcript_file::{self, TranscriptFile};
use crate::session::{count_text_tokens, truncate_text_tokens};
use crate::types::{LitecodeError, Result};

pub(crate) use crate::session::transcript_file::{SearchableRow as RawRow, row_plain_text};

/// Semantic ANN over-fetch before gating / session filter.
pub const SEMANTIC_WINDOW: usize = 16;
/// Minimum normalized Levenshtein similarity for a fuzzy hit.
pub const FUZZY_THRESHOLD: f64 = 0.72;
/// Max characters of the hit nucleus used while locating a match span.
pub const HIT_CORE_MAX_CHARS: usize = 200;
/// Semantic score gate: `score = 1/(1+dist)`; below this is noise.
pub const SEMANTIC_MIN_SCORE: f64 = 0.55;
/// Short session handle length (unique suffix / prefix resolve).
pub const SESSION_REF_SHORT_LEN: usize = 8;
/// Token budget for one session_search / human session page.
pub const PAGE_TOKEN_BUDGET: usize = 6_000;
/// Per-hit summary cap (cl100k tokens).
pub const HIT_SUMMARY_MAX_TOKENS: usize = 96;
/// Split long transcript text before windowed fuzzy to bound cost.
const FUZZY_BLOCK_CHARS: usize = 4096;

/// Exclude the live model window of one session: drop seqs currently on `surface.nodes`.
/// Shadowed append-origin rows remain searchable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowExclude {
    pub session_id: String,
    pub surface_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionTextQuery {
    pub query: String,
    pub offset: usize,
    /// Include-only scope (full session id).
    pub include_session_id: Option<String>,
    /// Sessions to drop entirely.
    pub exclude_session_ids: Vec<String>,
    pub project: Option<String>,
    /// Hard-exclude live context-window rows for the active session.
    pub exclude_context_window: Option<ContextWindowExclude>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionHitLane {
    #[default]
    Text,
    Semantic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTextHit {
    pub session_id: String,
    pub seq: i64,
    pub item_type: String,
    /// Lane-local preview; page hydration may replace this with a physical line.
    pub summary: String,
    pub score: f64,
    /// Match start within this item's plain text (char index).
    #[serde(default)]
    pub char_start: usize,
    /// Match end (exclusive) within this item's plain text.
    #[serde(default)]
    pub char_end: usize,
    #[serde(default)]
    pub lane: SessionHitLane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTimestamps {
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSearchHitRow {
    pub line: u32,
    pub seq: i64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSearchGroup {
    pub session_id: String,
    pub created_time: i64,
    pub updated_time: i64,
    pub path: String,
    pub match_count: usize,
    pub hits: Vec<SessionSearchHitRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSearchPage {
    pub groups: Vec<SessionSearchGroup>,
    pub offset: usize,
    pub next_offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
struct HydratedHit {
    session_id: String,
    seq: i64,
    line: u32,
    summary: String,
}

/// Case-insensitive fuzzy search over all detail rows.
pub fn search(reader: &SessionDataReader, query: &SessionTextQuery) -> Result<Vec<SessionTextHit>> {
    search_all(reader, query)
}

/// All ranked lexical hits (no pagination).
pub fn search_all(
    reader: &SessionDataReader,
    query: &SessionTextQuery,
) -> Result<Vec<SessionTextHit>> {
    lexical::search_lexical(reader, query)
}

/// Drop hits that violate include / exclude / context-window filters.
/// Used for the semantic lane (SQL already applies the same rules for text).
pub fn filter_hits(hits: Vec<SessionTextHit>, query: &SessionTextQuery) -> Vec<SessionTextHit> {
    hits.into_iter().filter(|h| hit_allowed(h, query)).collect()
}

fn hit_allowed(h: &SessionTextHit, query: &SessionTextQuery) -> bool {
    if let Some(sid) = query.include_session_id.as_ref().filter(|s| !s.is_empty())
        && &h.session_id != sid
    {
        return false;
    }
    if query
        .exclude_session_ids
        .iter()
        .any(|ex| !ex.is_empty() && ex == &h.session_id)
    {
        return false;
    }
    if let Some(win) = query.exclude_context_window.as_ref()
        && h.session_id == win.session_id
        && win.surface_seqs.iter().any(|s| *s == h.seq)
    {
        return false;
    }
    true
}

/// Lexical ranked list first; gated semantic hits append when `(session_id, seq)` is new.
pub fn merge_session_hits(
    lexical: Vec<SessionTextHit>,
    semantic: Vec<SessionTextHit>,
) -> Vec<SessionTextHit> {
    let mut seen: HashSet<(String, i64)> = lexical
        .iter()
        .map(|h| (h.session_id.clone(), h.seq))
        .collect();
    let mut out = lexical;
    for hit in semantic {
        if seen.insert((hit.session_id.clone(), hit.seq)) {
            out.push(hit);
        }
    }
    out
}

/// Short handle for unique suffix / prefix resolve.
///
/// Uses the **trailing** `SESSION_REF_SHORT_LEN` chars of the ULID (entropy),
/// not the timestamp prefix — sessions created in the same ms share a prefix.
pub fn short_session_ref(session_id: &str) -> &str {
    let mut indices = session_id.char_indices().rev().map(|(i, _)| i);
    let mut start = 0;
    for _ in 0..SESSION_REF_SHORT_LEN {
        match indices.next() {
            Some(i) => start = i,
            None => return session_id,
        }
    }
    &session_id[start..]
}

/// Resolve a full id, unique prefix, or unique short suffix to a durable session id.
pub fn resolve_session_ref(reader: &SessionDataReader, refer: &str) -> Result<String> {
    let refer = refer.trim();
    if refer.is_empty() {
        return Err(LitecodeError::Config("empty session ref".into()));
    }
    match reader.resolve_ref_blocking(refer)? {
        Some(id) => Ok(id),
        None => Err(LitecodeError::Config(format!(
            "session ref '{refer}' matched no sessions"
        ))),
    }
}

fn ambiguous_session_ref(refer: &str, matches: &[String]) -> LitecodeError {
    let shown: Vec<String> = matches
        .iter()
        .take(5)
        .map(|id| format!("{} ({id})", short_session_ref(id)))
        .collect();
    LitecodeError::Config(format!(
        "session ref '{refer}' is ambiguous ({} matches); candidates include: {}",
        matches.len(),
        shown.join(", ")
    ))
}

/// Fold the session log and return current surface seqs. Empty session → empty vec.
pub fn load_surface_seqs(reader: &SessionDataReader, session_id: &str) -> Result<Vec<i64>> {
    reader.surface_seqs_blocking(session_id)
}

/// Gate weak semantic scores.
pub fn gate_semantic_hits(mut semantic: Vec<SessionTextHit>) -> Vec<SessionTextHit> {
    semantic.retain(|h| h.score >= SEMANTIC_MIN_SCORE);
    semantic.sort_by(cmp_hits);
    semantic
}

fn cmp_hits(a: &SessionTextHit, b: &SessionTextHit) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.session_id.cmp(&b.session_id))
        .then_with(|| a.seq.cmp(&b.seq))
}

/// Load session created_at / updated_at for ids present in hits.
pub fn load_session_meta(
    reader: &SessionDataReader,
    session_ids: &[String],
) -> Result<HashMap<String, SessionTimestamps>> {
    let mut out = HashMap::new();
    for id in session_ids {
        if let Ok(meta) = reader.meta_blocking(id) {
            out.insert(
                id.clone(),
                SessionTimestamps {
                    created_at: meta.created_at,
                    updated_at: meta.updated_at,
                },
            );
        }
    }
    Ok(out)
}

/// Build a token-bounded grouped page from a fused ranked hit list.
pub fn build_search_page(
    reader: &SessionDataReader,
    ranked: &[SessionTextHit],
    offset: usize,
) -> Result<SessionSearchPage> {
    let hydrated = hydrate_hits(reader, ranked)?;
    let match_counts = count_by_session(&hydrated);
    let session_ids: Vec<String> = unique_session_order(&hydrated);
    let meta = load_session_meta(reader, &session_ids).unwrap_or_default();
    Ok(pack_page(
        &hydrated,
        &match_counts,
        &meta,
        offset,
        PAGE_TOKEN_BUDGET,
    ))
}

fn hydrate_hits(reader: &SessionDataReader, hits: &[SessionTextHit]) -> Result<Vec<HydratedHit>> {
    let mut cache: HashMap<String, TranscriptFile> = HashMap::new();
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if !cache.contains_key(&hit.session_id) {
            let file = reader.transcript_file_blocking(&hit.session_id)?;
            cache.insert(hit.session_id.clone(), file);
        }
        let file = cache.get(&hit.session_id).unwrap();
        let Some(line) = file.line_for_hit(hit.seq, hit.char_start, hit.char_end) else {
            continue;
        };
        let summary = match hit.lane {
            SessionHitLane::Text => file
                .line_text(line)
                .map(collapse_summary)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| collapse_summary(&hit.summary)),
            SessionHitLane::Semantic => collapse_summary(&hit.summary),
        };
        if summary.is_empty() {
            continue;
        }
        out.push(HydratedHit {
            session_id: hit.session_id.clone(),
            seq: hit.seq,
            line,
            summary,
        });
    }
    Ok(out)
}

fn collapse_summary(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text_tokens(&collapsed, HIT_SUMMARY_MAX_TOKENS)
}

fn count_by_session(hits: &[HydratedHit]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for hit in hits {
        *counts.entry(hit.session_id.clone()).or_insert(0) += 1;
    }
    counts
}

fn unique_session_order(hits: &[HydratedHit]) -> Vec<String> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for hit in hits {
        if seen.insert(hit.session_id.clone()) {
            order.push(hit.session_id.clone());
        }
    }
    order
}

fn pack_page(
    hydrated: &[HydratedHit],
    match_counts: &HashMap<String, usize>,
    meta: &HashMap<String, SessionTimestamps>,
    offset: usize,
    token_budget: usize,
) -> SessionSearchPage {
    if offset >= hydrated.len() {
        return SessionSearchPage {
            groups: Vec::new(),
            offset,
            next_offset: offset,
            has_more: false,
        };
    }
    let remaining = &hydrated[offset..];
    let mut groups: Vec<SessionSearchGroup> = Vec::new();
    let mut emitted = 0usize;
    for hit in remaining {
        let candidate = push_hit(groups.clone(), hit, match_counts, meta);
        let rendered = format_agent_groups(&candidate);
        let tokens = count_text_tokens(&rendered);
        if emitted > 0 && tokens > token_budget {
            break;
        }
        groups = candidate;
        emitted += 1;
    }
    if emitted == 0 && !remaining.is_empty() {
        groups = push_hit(Vec::new(), &remaining[0], match_counts, meta);
        emitted = 1;
    }
    SessionSearchPage {
        groups,
        offset,
        next_offset: offset + emitted,
        has_more: offset + emitted < hydrated.len(),
    }
}

fn push_hit(
    mut groups: Vec<SessionSearchGroup>,
    hit: &HydratedHit,
    match_counts: &HashMap<String, usize>,
    meta: &HashMap<String, SessionTimestamps>,
) -> Vec<SessionSearchGroup> {
    let row = SessionSearchHitRow {
        line: hit.line,
        seq: hit.seq,
        summary: hit.summary.clone(),
    };
    if let Some(last) = groups.last_mut()
        && last.session_id == hit.session_id
    {
        last.hits.push(row);
        return groups;
    }
    let ts = meta.get(&hit.session_id);
    groups.push(SessionSearchGroup {
        session_id: hit.session_id.clone(),
        created_time: ts.map(|t| t.created_at).unwrap_or(0),
        updated_time: ts.map(|t| t.updated_at).unwrap_or(0),
        path: transcript_file::virtual_path_for(&hit.session_id),
        match_count: match_counts.get(&hit.session_id).copied().unwrap_or(0),
        hits: vec![row],
    });
    groups
}

pub fn format_agent_page(page: &SessionSearchPage) -> String {
    let mut body = format_agent_groups(&page.groups);
    let footer = if page.has_more {
        Some(crate::tool::format_offset_more(page.next_offset))
    } else if page.offset > 0 {
        Some(crate::tool::format_offset_done(page.offset))
    } else {
        None
    };
    if let Some(footer) = footer {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&footer);
    }
    body
}

fn format_agent_groups(groups: &[SessionSearchGroup]) -> String {
    let mut parts = Vec::new();
    for group in groups {
        parts.push(format!("### {}", group.session_id));
        parts.push(format!("created: {}", format_abs_time(group.created_time)));
        parts.push(format!("updated: {}", format_abs_time(group.updated_time)));
        parts.push(format!("path: {}", group.path));
        parts.push(format!("matches: {}", group.match_count));
        parts.push(String::new());
        for hit in &group.hits {
            parts.push(format!(
                "{}: {}",
                crate::tool::format_line_label(hit.line, hit.line),
                hit.summary
            ));
        }
    }
    parts.join("\n")
}

fn format_abs_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "?".into())
}

/// Returns `(score, char_start, char_end)` when the needle matches.
pub(crate) fn fuzzy_match_span(haystack: &str, needle: &str) -> Option<(f64, usize, usize)> {
    let hay = haystack.to_lowercase();
    let ned = needle.to_lowercase();
    if ned.is_empty() {
        return None;
    }
    let chars: Vec<char> = haystack.chars().collect();
    let hay_chars: Vec<char> = hay.chars().collect();
    let ned_chars: Vec<char> = ned.chars().collect();
    let n = ned_chars.len();
    if n == 0 {
        return None;
    }

    if let Some(byte_start) = hay.find(&ned) {
        let char_start = hay[..byte_start].chars().count();
        let char_end = (char_start + n).min(chars.len());
        return Some((1.0, char_start, char_end));
    }

    let step = (n / 4).max(1);
    let mut best = 0.0f64;
    let mut best_start = 0usize;

    for block_start in (0..hay_chars.len()).step_by(FUZZY_BLOCK_CHARS) {
        let block_end = (block_start + FUZZY_BLOCK_CHARS).min(hay_chars.len());
        let block = &hay_chars[block_start..block_end];
        if block.len() < n {
            let window: String = block.iter().collect();
            let score = strsim::normalized_levenshtein(&ned, &window);
            if score > best {
                best = score;
                best_start = block_start;
            }
            continue;
        }
        let mut i = 0usize;
        while i + n <= block.len() {
            let window: String = block[i..i + n].iter().collect();
            let score = strsim::normalized_levenshtein(&ned, &window);
            if score > best {
                best = score;
                best_start = block_start + i;
            }
            if best >= 0.999 {
                return Some((1.0, best_start, best_start + n));
            }
            i += step;
        }
    }

    if best >= FUZZY_THRESHOLD {
        Some((best, best_start, (best_start + n).min(chars.len())))
    } else {
        None
    }
}

pub(crate) fn snippet_from_span(text: &str, char_start: usize, char_end: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let pad = HIT_CORE_MAX_CHARS / 4;
    let from = char_start.saturating_sub(pad);
    let to = (char_end + pad).min(chars.len());
    let mut snippet: String = chars[from..to].iter().collect();
    if from > 0 {
        snippet.insert(0, '…');
    }
    if to < chars.len() {
        snippet.push('…');
    }
    snippet.chars().take(HIT_CORE_MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionData, SessionDataReader, WorkspaceWriteLease};
    use crate::types::user_text;
    use std::path::Path;
    use tempfile::TempDir;

    fn seed_db(dir: &Path) -> (SessionDataReader, String, String) {
        let db = dir.join("sessions.db");
        let (id_a, id_b) = {
            let lease = WorkspaceWriteLease::acquire(dir).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id_a = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(&id_a, &[user_text("alpha UNIQUE_SESSION_PHRASE omega")])
                .unwrap();
            let id_b = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(&id_b, &[user_text("other OTHER_MARKER content")])
                .unwrap();
            (id_a, id_b)
        };
        (SessionDataReader::open(&db), id_a, id_b)
    }

    #[test]
    fn session_text_search_finds_seeded_transcript() {
        let dir = TempDir::new().unwrap();
        let (reader, id_a, _) = seed_db(dir.path());

        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRASE".into(),
                offset: 0,
                include_session_id: None,
                project: None,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, id_a);
        assert_eq!(hits[0].seq, 0);
        assert!(hits[0].summary.contains("UNIQUE_SESSION_PHRASE"));
    }

    #[test]
    fn session_text_search_case_insensitive() {
        let dir = TempDir::new().unwrap();
        let (reader, id_a, _) = seed_db(dir.path());
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "unique_session_phrase".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, id_a);
        assert!((hits[0].score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn session_text_search_fuzzy_typo() {
        let dir = TempDir::new().unwrap();
        let (reader, id_a, _) = seed_db(dir.path());
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRAZE".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, id_a);
        assert!(hits[0].score >= FUZZY_THRESHOLD);
        assert!(hits[0].score < 1.0);
    }

    #[test]
    fn session_text_search_respects_scope() {
        let dir = TempDir::new().unwrap();
        let (reader, id_a, id_b) = seed_db(dir.path());

        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "OTHER_MARKER".into(),
                offset: 0,
                include_session_id: Some(id_a.clone()),
                project: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            hits.is_empty(),
            "scoped to session A must not see B's marker"
        );

        let page_b = search(
            &reader,
            &SessionTextQuery {
                query: "OTHER_MARKER".into(),
                offset: 0,
                include_session_id: Some(id_b.clone()),
                project: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page_b.len(), 1);
        assert_eq!(page_b[0].session_id, id_b);
    }

    #[test]
    fn session_text_search_skips_empty_query() {
        let dir = TempDir::new().unwrap();
        let (reader, _, _) = seed_db(dir.path());
        let err = search(
            &reader,
            &SessionTextQuery {
                query: "   ".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn session_text_search_missing_db_returns_empty() {
        let dir = TempDir::new().unwrap();
        let hits = search(
            &SessionDataReader::open(&dir.path().join("nope.db")),
            &SessionTextQuery {
                query: "anything".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn gate_drops_weak_semantic() {
        let sem = vec![SessionTextHit {
            session_id: "s".into(),
            seq: 0,
            item_type: "message".into(),
            summary: "x".into(),
            score: 0.3,
            char_start: 0,
            char_end: 1,
            lane: SessionHitLane::Semantic,
        }];
        assert!(gate_semantic_hits(sem).is_empty());
    }

    #[test]
    fn gate_keeps_strong_semantic() {
        let sem = vec![SessionTextHit {
            session_id: "s".into(),
            seq: 0,
            item_type: "message".into(),
            summary: "x".into(),
            score: 0.8,
            char_start: 0,
            char_end: 1,
            lane: SessionHitLane::Semantic,
        }];
        assert_eq!(gate_semantic_hits(sem).len(), 1);
    }

    #[test]
    fn merge_keeps_lexical_first_and_appends_unique_semantic() {
        let lexical = vec![SessionTextHit {
            session_id: "a".into(),
            seq: 1,
            item_type: "message".into(),
            summary: "lex".into(),
            score: 1.0,
            char_start: 0,
            char_end: 3,
            lane: SessionHitLane::Text,
        }];
        let semantic = vec![
            SessionTextHit {
                session_id: "a".into(),
                seq: 1,
                item_type: "message".into(),
                summary: "dup".into(),
                score: 0.9,
                char_start: 0,
                char_end: 0,
                lane: SessionHitLane::Semantic,
            },
            SessionTextHit {
                session_id: "b".into(),
                seq: 2,
                item_type: "message".into(),
                summary: "only-sem".into(),
                score: 0.8,
                char_start: 0,
                char_end: 0,
                lane: SessionHitLane::Semantic,
            },
        ];
        let merged = merge_session_hits(lexical, semantic);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].session_id, "a");
        assert_eq!(merged[0].lane, SessionHitLane::Text);
        assert_eq!(merged[1].session_id, "b");
        assert_eq!(merged[1].lane, SessionHitLane::Semantic);
    }

    #[test]
    fn session_text_search_includes_function_call_rows() {
        use crate::authority::responses::FunctionToolCall;
        use crate::types::Item;

        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(
                &id,
                &[Item::FunctionCall(FunctionToolCall {
                    arguments: r#"{"cmd":"UNIQUE_TOOL_NEEDLE"}"#.into(),
                    call_id: "c1".into(),
                    namespace: None,
                    name: "bash".into(),
                    id: None,
                    status: None,
                })],
            )
            .unwrap();
        }
        let reader = SessionDataReader::open(&db);

        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "UNIQUE_TOOL_NEEDLE".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_type, "function_call");
    }

    #[test]
    fn context_window_exclude_drops_live_seqs_keeps_archive() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(
                &id,
                &[
                    user_text("ARCHIVE_NEEDLE_ONLY"),
                    user_text("LIVE_NEEDLE_ONLY"),
                ],
            )
            .unwrap();
            data.compact_from(&id, &user_text("sum"), Some(1), 3)
                .unwrap();
            id
        };
        let reader = SessionDataReader::open(&db);

        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "NEEDLE_ONLY".into(),
                exclude_context_window: Some(ContextWindowExclude {
                    session_id: sid.clone(),
                    surface_seqs: load_surface_seqs(&reader, &sid).unwrap(),
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].seq, 0);
        assert!(hits[0].summary.contains("ARCHIVE_NEEDLE_ONLY"));
    }

    #[test]
    fn exclude_session_ids_filters_rows() {
        let dir = TempDir::new().unwrap();
        let (reader, id_a, id_b) = seed_db(dir.path());
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "OTHER_MARKER".into(),
                exclude_session_ids: vec![id_b.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(hits.is_empty());

        let page_a = search(
            &reader,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRASE".into(),
                exclude_session_ids: vec![id_b],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page_a.len(), 1);
        assert_eq!(page_a[0].session_id, id_a);
    }

    #[test]
    fn resolve_session_ref_unique_prefix() {
        let dir = TempDir::new().unwrap();
        let (reader, id_a, _) = seed_db(dir.path());
        let short = short_session_ref(&id_a);
        assert_eq!(resolve_session_ref(&reader, short).unwrap(), id_a);
        assert_eq!(resolve_session_ref(&reader, &id_a).unwrap(), id_a);
        let err = resolve_session_ref(&reader, "ZZZZNOPE").unwrap_err();
        assert!(err.to_string().contains("matched no sessions"));
    }

    #[test]
    fn lexical_fts_indexes_on_insert_and_finds_token() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(
                &id,
                &[user_text(
                    "decision: use middleware for AuthRefactorToken login path",
                )],
            )
            .unwrap();
        }
        let reader = SessionDataReader::open(&db);

        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "AuthRefactorToken".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score >= 0.55);
        assert!(hits[0].summary.contains("AuthRefactorToken"));
    }

    #[test]
    fn build_search_page_uses_physical_line_and_match_count() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(
                &id,
                &[
                    user_text("first PAGE_LINE_TOKEN here"),
                    user_text("filler"),
                    user_text("second PAGE_LINE_TOKEN there"),
                ],
            )
            .unwrap();
            id
        };
        let reader = SessionDataReader::open(&db);

        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "PAGE_LINE_TOKEN".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let page = build_search_page(&reader, &hits, 0).unwrap();
        assert_eq!(page.groups.len(), 1);
        assert_eq!(page.groups[0].session_id, sid);
        assert_eq!(page.groups[0].match_count, 2);
        assert_eq!(page.groups[0].hits.len(), 2);
        assert_eq!(
            page.groups[0].path,
            crate::session::transcript_file::virtual_path_for(&sid)
        );
        assert!(page.groups[0].hits[0].line >= 2);
        assert!(page.groups[0].hits[0].summary.contains("PAGE_LINE_TOKEN"));
        assert!(!page.has_more);
    }

    #[test]
    fn pack_page_stops_on_whole_hit_and_advances_offset() {
        let hits: Vec<HydratedHit> = (0..8)
            .map(|i| HydratedHit {
                session_id: "s".into(),
                seq: i,
                line: (i + 1) as u32,
                summary: format!("hit {i} {}", "x".repeat(20)),
            })
            .collect();
        let mut counts = HashMap::new();
        counts.insert("s".into(), 8);
        let page = pack_page(&hits, &counts, &HashMap::new(), 0, 40);
        assert!(page.groups.len() == 1);
        assert!(page.groups[0].hits.len() >= 1);
        assert!(page.has_more);
        assert_eq!(page.next_offset, page.groups[0].hits.len());
        assert!(page.next_offset >= 1);
    }

    #[test]
    fn pack_page_offset_past_end_is_empty() {
        let hits = vec![HydratedHit {
            session_id: "s".into(),
            seq: 0,
            line: 1,
            summary: "only".into(),
        }];
        let page = pack_page(&hits, &HashMap::new(), &HashMap::new(), 3, 6000);
        assert!(page.groups.is_empty());
        assert_eq!(page.next_offset, 3);
        assert!(!page.has_more);
    }
}
