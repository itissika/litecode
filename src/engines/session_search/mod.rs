//! Session corpus — Lexical (always-on) + Semantic (ANN-only when engine Warm).
//!
//! Does not own session writes, schema migration, or ORT lifecycle.

mod lexical;
mod semantic_index;

pub use semantic_index::{SessionSemanticIndex, ensure_session_index};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::session::store::data_root_from_db_path;
use crate::tool::output::{BLOB_PREFIX, blob_dir};
use crate::types::{Item, LitecodeError, Result, item_text_preview};

/// Hits per agent/UI page.
pub const RESULTS_PER_PAGE: usize = 10;
/// Semantic ANN over-fetch before gating / session filter.
pub const SEMANTIC_WINDOW: usize = 16;
/// Minimum normalized Levenshtein similarity for a fuzzy hit.
pub const FUZZY_THRESHOLD: f64 = 0.72;
/// Character step for expand (agent `expand` multiplies this).
pub const EXPAND_STEP_CHARS: usize = 300;
/// Default expand steps when agent omits `expand`: `[1, 1]`.
pub const DEFAULT_EXPAND_UP: usize = 1;
pub const DEFAULT_EXPAND_DOWN: usize = 1;
/// Max characters of the hit nucleus shown/marked.
pub const HIT_CORE_MAX_CHARS: usize = 200;
/// Related lane: max plain-text chars per entry (no expand / no fake span).
pub const RELATED_ENTRY_MAX_CHARS: usize = 400;
/// Soft byte budget for agent dual-view (executor hard-caps ~`max_result_size * 4`).
pub const AGENT_VIEW_SOFT_BYTES: usize = 28_000;
/// Hit nucleus markers — not markdown (`**` collides with transcript MD).
pub const HIT_MARK_OPEN: &str = "⟦";
pub const HIT_MARK_CLOSE: &str = "⟧";
/// Semantic score gate: `score = 1/(1+dist)`; below this is noise.
pub const SEMANTIC_MIN_SCORE: f64 = 0.55;
/// Short session handle length shown to agents (unique suffix / prefix resolve).
pub const SESSION_REF_SHORT_LEN: usize = 8;
/// Split long transcript text before windowed fuzzy to bound cost.
const FUZZY_BLOCK_CHARS: usize = 4096;
/// Separator between transcript items in the session character stream.
const ITEM_SEP: &str = "\n\n";

/// Exclude the live context window of one session: drop `seq >= kept_from_seq`.
/// Archived detail below `kept_from_seq` remains searchable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowExclude {
    pub session_id: String,
    pub kept_from_seq: i64,
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
    /// Short preview (legacy / human UI); agent view uses char windows.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTextPage {
    pub hits: Vec<SessionTextHit>,
    pub offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpandSteps {
    pub up: usize,
    pub down: usize,
}

impl Default for ExpandSteps {
    fn default() -> Self {
        Self {
            up: DEFAULT_EXPAND_UP,
            down: DEFAULT_EXPAND_DOWN,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub created_at: i64,
    pub updated_at: i64,
}

/// One rendered hit window (after expand or Related entry truncate).
#[derive(Debug, Clone, PartialEq)]
pub struct HitWindow {
    pub hit: SessionTextHit,
    /// Window body; Matches wraps the nucleus in `⟦...⟧` (not markdown).
    pub body: String,
    /// Absolute char offset of window start in the session stream (merge math).
    pub chars_above: usize,
    /// Chars after window end in the session stream (merge math).
    pub chars_below: usize,
    /// Transcript items fully above the window (agent-facing overflow).
    pub items_above: usize,
    /// Transcript items fully below the window (agent-facing overflow).
    pub items_below: usize,
    /// More text in the same entry outside the shown span (mid-item / Related truncate).
    pub same_item_overflow: bool,
}

/// Strip hit markers for length / overlap math.
pub fn strip_hit_marks(s: &str) -> String {
    s.replace(HIT_MARK_OPEN, "").replace(HIT_MARK_CLOSE, "")
}

fn sanitize_hit_marks(s: &str) -> String {
    // Avoid nested / false markers inside transcript text.
    s.replace(HIT_MARK_OPEN, "[").replace(HIT_MARK_CLOSE, "]")
}

fn wrap_hit_core(core: &str) -> String {
    format!(
        "{HIT_MARK_OPEN}{}{HIT_MARK_CLOSE}",
        sanitize_hit_marks(core)
    )
}

/// Case-insensitive fuzzy search over all detail rows; paginated.
pub fn search(db_path: &Path, query: &SessionTextQuery) -> Result<SessionTextPage> {
    let ranked = search_all(db_path, query)?;
    let offset = query.offset;
    let end = offset
        .saturating_add(RESULTS_PER_PAGE)
        .saturating_add(1)
        .min(ranked.len());
    let slice = if offset >= ranked.len() {
        &[][..]
    } else {
        &ranked[offset..end]
    };
    let has_more = slice.len() > RESULTS_PER_PAGE;
    let hits = if has_more {
        slice[..RESULTS_PER_PAGE].to_vec()
    } else {
        slice.to_vec()
    };
    Ok(SessionTextPage {
        hits,
        offset,
        has_more,
    })
}

/// All ranked lexical hits (no pagination) — Matches column.
pub fn search_all(db_path: &Path, query: &SessionTextQuery) -> Result<Vec<SessionTextHit>> {
    lexical::search_lexical(db_path, query)
}

/// Drop hits that violate include / exclude / context-window filters.
/// Used for the semantic lane (SQL already applies the same rules for text).
pub fn filter_hits(hits: Vec<SessionTextHit>, query: &SessionTextQuery) -> Vec<SessionTextHit> {
    hits.into_iter().filter(|h| hit_allowed(h, query)).collect()
}

fn hit_allowed(h: &SessionTextHit, query: &SessionTextQuery) -> bool {
    if let Some(sid) = query.include_session_id.as_ref().filter(|s| !s.is_empty())
        && &h.session_id != sid {
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
        && h.session_id == win.session_id && h.seq >= win.kept_from_seq {
            return false;
        }
    true
}

/// Short handle for agent-facing output.
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
pub fn resolve_session_ref(db_path: &Path, refer: &str) -> Result<String> {
    let refer = refer.trim();
    if refer.is_empty() {
        return Err(LitecodeError::Config("empty session ref".into()));
    }
    if !db_path.is_file() {
        return Err(LitecodeError::Config(format!(
            "session ref '{refer}' not found (no sessions.db)"
        )));
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;

    let exact: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions WHERE id = ?1",
            rusqlite::params![refer],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = exact {
        return Ok(id);
    }

    let mut stmt = conn
        .prepare("SELECT id FROM sessions ORDER BY id ASC")
        .map_err(|e| LitecodeError::Config(format!("session ref prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| LitecodeError::Config(format!("session ref query: {e}")))?;
    let mut prefix_matches = Vec::new();
    let mut suffix_matches = Vec::new();
    for row in rows {
        let id = row.map_err(|e| LitecodeError::Config(format!("session ref row: {e}")))?;
        if id.starts_with(refer) {
            prefix_matches.push(id.clone());
        }
        if id.ends_with(refer) {
            suffix_matches.push(id);
        }
    }

    match prefix_matches.len() {
        1 => return Ok(prefix_matches.remove(0)),
        n if n > 1 => {
            return Err(ambiguous_session_ref(refer, &prefix_matches));
        }
        _ => {}
    }
    match suffix_matches.len() {
        0 => Err(LitecodeError::Config(format!(
            "session ref '{refer}' matched no sessions"
        ))),
        1 => Ok(suffix_matches.remove(0)),
        _ => Err(ambiguous_session_ref(refer, &suffix_matches)),
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

/// Read `kept_from_seq` for a session (context-window floor). `None` if missing.
pub fn load_kept_from_seq(db_path: &Path, session_id: &str) -> Result<Option<i64>> {
    if !db_path.is_file() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;
    match conn.query_row(
        "SELECT kept_from_seq FROM sessions WHERE id = ?1",
        rusqlite::params![session_id],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(LitecodeError::Config(format!("kept_from_seq: {e}"))),
    }
}

/// Parsed agent filter tokens (`filter` field).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFilterTokens {
    pub include_session: Option<String>,
    pub exclude_sessions: Vec<String>,
    pub exclude_current: bool,
}

/// Parse space-separated filter tokens: `session:<ref>`, `-session:<ref>`, `-current`.
pub fn parse_filter_tokens(filter: &str) -> Result<SessionFilterTokens> {
    let mut out = SessionFilterTokens::default();
    for raw in filter.split_whitespace() {
        let tok = raw.trim();
        if tok.is_empty() {
            continue;
        }
        if tok.eq_ignore_ascii_case("-current") {
            out.exclude_current = true;
            continue;
        }
        if let Some(refer) = strip_prefix_ci(tok, "-session:") {
            let refer = refer.trim();
            if refer.is_empty() {
                return Err(LitecodeError::Config(
                    "session_filter -session: requires a session ref".into(),
                ));
            }
            out.exclude_sessions.push(refer.to_string());
            continue;
        }
        if let Some(refer) = strip_prefix_ci(tok, "session:") {
            let refer = refer.trim();
            if refer.is_empty() {
                return Err(LitecodeError::Config(
                    "session_filter session: requires a session ref".into(),
                ));
            }
            if out.include_session.is_some() {
                return Err(LitecodeError::Config(
                    "session_filter allows at most one session: include".into(),
                ));
            }
            out.include_session = Some(refer.to_string());
            continue;
        }
        return Err(LitecodeError::Config(format!(
            "unknown session_filter token '{tok}'; use session:<ref>, -session:<ref>, or -current"
        )));
    }
    Ok(out)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len()
        && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Gate weak semantic scores. Dual-lane product path does not fuse columns.
pub fn gate_semantic_hits(mut semantic: Vec<SessionTextHit>) -> Vec<SessionTextHit> {
    semantic.retain(|h| h.score >= SEMANTIC_MIN_SCORE);
    semantic.sort_by(cmp_hits);
    semantic
}

fn cmp_hits(a: &SessionTextHit, b: &SessionTextHit) -> std::cmp::Ordering {
    let lane_ord = |l: SessionHitLane| match l {
        SessionHitLane::Text => 0,
        SessionHitLane::Semantic => 1,
    };
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| lane_ord(a.lane).cmp(&lane_ord(b.lane)))
        .then_with(|| a.session_id.cmp(&b.session_id))
        .then_with(|| a.seq.cmp(&b.seq))
}

/// Paginate a fused hit list.
pub fn paginate_hits(mut hits: Vec<SessionTextHit>, offset: usize) -> SessionTextPage {
    let end = offset
        .saturating_add(RESULTS_PER_PAGE)
        .saturating_add(1)
        .min(hits.len());
    let slice = if offset >= hits.len() {
        Vec::new()
    } else {
        hits.drain(offset..end).collect::<Vec<_>>()
    };
    let has_more = slice.len() > RESULTS_PER_PAGE;
    let page_hits = if has_more {
        slice[..RESULTS_PER_PAGE].to_vec()
    } else {
        slice
    };
    SessionTextPage {
        hits: page_hits,
        offset,
        has_more,
    }
}

/// Load session created_at / updated_at for ids present in hits.
pub fn load_session_meta(
    db_path: &Path,
    session_ids: &[String],
) -> Result<HashMap<String, SessionMeta>> {
    let mut out = HashMap::new();
    if session_ids.is_empty() || !db_path.is_file() {
        return Ok(out);
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;
    for id in session_ids {
        if let Ok((created_at, updated_at)) = conn.query_row(
            "SELECT created_at, updated_at FROM sessions WHERE id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ) {
            out.insert(
                id.clone(),
                SessionMeta {
                    created_at,
                    updated_at,
                },
            );
        }
    }
    Ok(out)
}

/// Build character windows for each hit (independent; overlapping content OK).
///
/// When `stream_limit` is set, the named session's stream is truncated to
/// `seq < kept_from_seq` so expand cannot pull live context-window text.
pub fn expand_hit_windows(
    db_path: &Path,
    hits: &[SessionTextHit],
    expand: ExpandSteps,
    stream_limit: Option<&ContextWindowExclude>,
) -> Result<Vec<HitWindow>> {
    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut cache: HashMap<String, SessionCharStream> = HashMap::new();
    let mut windows = Vec::with_capacity(hits.len());
    for hit in hits {
        if !cache.contains_key(&hit.session_id) {
            let seq_lt = stream_limit.and_then(|w| {
                if w.session_id == hit.session_id {
                    Some(w.kept_from_seq)
                } else {
                    None
                }
            });
            let stream = load_session_char_stream(db_path, &data_root, &hit.session_id, seq_lt)?;
            cache.insert(hit.session_id.clone(), stream);
        }
        let stream = cache.get(&hit.session_id).unwrap();
        windows.push(window_for_hit(stream, hit, expand));
    }
    Ok(merge_overlapping_windows(windows))
}

struct SessionCharStream {
    /// Full concatenated plain text.
    text: String,
    /// (seq, start_char, end_char exclusive) for each item in stream order.
    spans: Vec<(i64, usize, usize)>,
}

fn load_session_char_stream(
    db_path: &Path,
    data_root: &Path,
    session_id: &str,
    seq_lt: Option<i64>,
) -> Result<SessionCharStream> {
    if !db_path.is_file() {
        return Ok(SessionCharStream {
            text: String::new(),
            spans: Vec::new(),
        });
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;
    let mut sql = String::from(
        "SELECT seq, item_type, body, body_ref FROM transcript_items
         WHERE session_id = ?1 AND kind = 'detail'",
    );
    if seq_lt.is_some() {
        sql.push_str(" AND seq < ?2");
    }
    sql.push_str(" ORDER BY seq ASC");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| LitecodeError::Config(format!("session stream prepare: {e}")))?;

    let mut text = String::new();
    let mut spans = Vec::new();
    if let Some(lt) = seq_lt {
        let rows = stmt
            .query_map(rusqlite::params![session_id, lt], |row| {
                Ok(RawRow {
                    session_id: session_id.to_string(),
                    seq: row.get(0)?,
                    item_type: row.get(1)?,
                    body: row.get(2)?,
                    body_ref: row.get(3)?,
                })
            })
            .map_err(|e| LitecodeError::Config(format!("session stream query: {e}")))?;
        for row in rows {
            let row = row.map_err(|e| LitecodeError::Config(format!("session stream row: {e}")))?;
            let Some(plain) = row_plain_text(&row, data_root)? else {
                continue;
            };
            if !text.is_empty() {
                text.push_str(ITEM_SEP);
            }
            let start = text.chars().count();
            text.push_str(&plain);
            let end = text.chars().count();
            spans.push((row.seq, start, end));
        }
    } else {
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| {
                Ok(RawRow {
                    session_id: session_id.to_string(),
                    seq: row.get(0)?,
                    item_type: row.get(1)?,
                    body: row.get(2)?,
                    body_ref: row.get(3)?,
                })
            })
            .map_err(|e| LitecodeError::Config(format!("session stream query: {e}")))?;
        for row in rows {
            let row = row.map_err(|e| LitecodeError::Config(format!("session stream row: {e}")))?;
            let Some(plain) = row_plain_text(&row, data_root)? else {
                continue;
            };
            if !text.is_empty() {
                text.push_str(ITEM_SEP);
            }
            let start = text.chars().count();
            text.push_str(&plain);
            let end = text.chars().count();
            spans.push((row.seq, start, end));
        }
    }
    Ok(SessionCharStream { text, spans })
}

fn window_for_hit(
    stream: &SessionCharStream,
    hit: &SessionTextHit,
    expand: ExpandSteps,
) -> HitWindow {
    let chars: Vec<char> = stream.text.chars().collect();
    let total = chars.len();
    let (item_start, _item_end) = stream
        .spans
        .iter()
        .find(|(seq, _, _)| *seq == hit.seq)
        .map(|(_, s, e)| (*s, *e))
        .unwrap_or((0, total));

    let core_start = item_start.saturating_add(hit.char_start).min(total);
    let mut core_end = item_start.saturating_add(hit.char_end).min(total);
    if core_end < core_start {
        core_end = core_start;
    }
    // Cap nucleus length.
    if core_end.saturating_sub(core_start) > HIT_CORE_MAX_CHARS {
        core_end = core_start + HIT_CORE_MAX_CHARS;
    }
    if core_start == core_end && total > 0 {
        core_end = (core_start + 1).min(total);
    }

    let up = expand.up.saturating_mul(EXPAND_STEP_CHARS);
    let down = expand.down.saturating_mul(EXPAND_STEP_CHARS);
    let win_start = core_start.saturating_sub(up);
    let win_end = (core_end + down).min(total);

    let before: String = chars[win_start..core_start].iter().collect();
    let core: String = chars[core_start..core_end].iter().collect();
    let after: String = chars[core_end..win_end].iter().collect();
    let body = format!(
        "{}{}{}",
        sanitize_hit_marks(&before),
        wrap_hit_core(&core),
        sanitize_hit_marks(&after)
    );

    let (items_above, items_below) = count_items_outside(stream, win_start, win_end);
    let item_end = stream
        .spans
        .iter()
        .find(|(seq, _, _)| *seq == hit.seq)
        .map(|(_, _, e)| *e)
        .unwrap_or(total);
    let same_item_overflow = win_start > item_start || win_end < item_end;

    HitWindow {
        hit: hit.clone(),
        body,
        chars_above: win_start,
        chars_below: total.saturating_sub(win_end),
        items_above,
        items_below,
        same_item_overflow,
    }
}

fn count_items_outside(
    stream: &SessionCharStream,
    win_start: usize,
    win_end: usize,
) -> (usize, usize) {
    let mut above = 0usize;
    let mut below = 0usize;
    for &(_, s, e) in &stream.spans {
        if e <= win_start {
            above += 1;
        } else if s >= win_end {
            below += 1;
        }
    }
    (above, below)
}

/// Related lane: one truncated entry per hit (no expand, no fake char bold).
pub fn related_entry_windows(db_path: &Path, hits: &[SessionTextHit]) -> Result<Vec<HitWindow>> {
    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut windows = Vec::with_capacity(hits.len());
    for hit in hits {
        let text = match load_item_plain_text(db_path, &data_root, &hit.session_id, hit.seq)? {
            Some(t) => t,
            None => hit.summary.clone(),
        };
        let total = text.chars().count();
        let truncated: String = text.chars().take(RELATED_ENTRY_MAX_CHARS).collect();
        let shown = truncated.chars().count();
        let overflow = total > shown;
        windows.push(HitWindow {
            hit: hit.clone(),
            body: sanitize_hit_marks(&truncated),
            chars_above: 0,
            chars_below: total.saturating_sub(shown),
            items_above: 0,
            items_below: 0,
            same_item_overflow: overflow,
        });
    }
    Ok(windows)
}

fn load_item_plain_text(
    db_path: &Path,
    data_root: &Path,
    session_id: &str,
    seq: i64,
) -> Result<Option<String>> {
    if !db_path.is_file() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;
    let row = conn.query_row(
        "SELECT item_type, body, body_ref FROM transcript_items
         WHERE session_id = ?1 AND seq = ?2 AND kind = 'detail'",
        rusqlite::params![session_id, seq],
        |r| {
            Ok(RawRow {
                session_id: session_id.to_string(),
                seq,
                item_type: r.get(0)?,
                body: r.get(1)?,
                body_ref: r.get(2)?,
            })
        },
    );
    match row {
        Ok(row) => row_plain_text(&row, data_root),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(LitecodeError::Config(format!("load item plain: {e}"))),
    }
}

/// Merge overlapping windows within the same session.
///
/// Overlapping expanded regions are collapsed to the wider body so the agent
/// does not see duplicated transcript text under adjacent hits.
fn merge_overlapping_windows(windows: Vec<HitWindow>) -> Vec<HitWindow> {
    if windows.len() <= 1 {
        return windows;
    }
    let mut session_order: Vec<String> = Vec::new();
    let mut by_session: HashMap<String, Vec<HitWindow>> = HashMap::new();
    for w in windows {
        let sid = w.hit.session_id.clone();
        if !by_session.contains_key(&sid) {
            session_order.push(sid.clone());
        }
        by_session.entry(sid).or_default().push(w);
    }

    let mut out = Vec::new();
    for sid in session_order {
        let mut group = by_session.remove(&sid).unwrap_or_default();
        // Preserve input (score) order for which hit is "primary".
        while !group.is_empty() {
            let mut cur = group.remove(0);
            let mut i = 0;
            while i < group.len() {
                if windows_overlap(&cur, &group[i]) {
                    let other = group.remove(i);
                    cur = union_windows(cur, other);
                } else {
                    i += 1;
                }
            }
            out.push(cur);
        }
    }
    out
}

fn window_char_range(w: &HitWindow) -> (usize, usize) {
    let plain_len = strip_hit_marks(&w.body).chars().count();
    let start = w.chars_above;
    (start, start + plain_len)
}

fn windows_overlap(a: &HitWindow, b: &HitWindow) -> bool {
    if a.hit.session_id != b.hit.session_id {
        return false;
    }
    let (as_, ae) = window_char_range(a);
    let (bs, be) = window_char_range(b);
    as_ < be && bs < ae
}

fn union_windows(a: HitWindow, b: HitWindow) -> HitWindow {
    let (as_, ae) = window_char_range(&a);
    let (bs, be) = window_char_range(&b);
    let start = as_.min(bs);
    let a_plain = ae - as_;
    let b_plain = be - bs;
    // Keep the longer window body (already bolded); primary hit is higher score.
    let (primary, secondary) = if a.hit.score >= b.hit.score {
        (a, b)
    } else {
        (b, a)
    };
    let body = if a_plain >= b_plain {
        primary.body.clone()
    } else {
        secondary.body.clone()
    };
    let (items_above, items_below, same_item_overflow) = if a_plain >= b_plain {
        (
            primary.items_above,
            primary.items_below,
            primary.same_item_overflow,
        )
    } else {
        (
            secondary.items_above,
            secondary.items_below,
            secondary.same_item_overflow,
        )
    };
    HitWindow {
        hit: primary.hit,
        body,
        chars_above: start,
        chars_below: primary.chars_below.min(secondary.chars_below),
        items_above,
        items_below,
        same_item_overflow,
    }
}

pub(crate) struct RawRow {
    pub session_id: String,
    pub seq: i64,
    pub item_type: String,
    pub body: Option<String>,
    pub body_ref: Option<String>,
}

pub(crate) fn row_plain_text(row: &RawRow, data_root: &Path) -> Result<Option<String>> {
    let json = if let Some(body) = &row.body {
        body.clone()
    } else if let Some(body_ref) = &row.body_ref {
        match load_blob_text(body_ref, data_root) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    session_id = %row.session_id,
                    seq = row.seq,
                    error = %e,
                    "session search skip unread blob"
                );
                return Ok(None);
            }
        }
    } else {
        return Ok(None);
    };
    let item: Item = match serde_json::from_str(&json) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(
                session_id = %row.session_id,
                seq = row.seq,
                error = %e,
                "session search skip bad item json"
            );
            return Ok(None);
        }
    };
    let text = item_text_preview(&item);
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

fn load_blob_text(body_ref: &str, data_root: &Path) -> Result<String> {
    let rest = body_ref
        .strip_prefix(BLOB_PREFIX)
        .ok_or_else(|| LitecodeError::Config(format!("invalid body_ref: {body_ref}")))?;
    let (id, _) = rest
        .split_once(']')
        .ok_or_else(|| LitecodeError::Config(format!("invalid body_ref: {body_ref}")))?;
    let blob_path = blob_dir(data_root).join(format!("{id}.txt"));
    std::fs::read_to_string(blob_path).map_err(Into::into)
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

    // Exact substring (case-insensitive) on char stream.
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
    let core_len = char_end.saturating_sub(char_start).max(1);
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
    let _ = core_len;
    snippet.chars().take(HIT_CORE_MAX_CHARS).collect()
}

/// Resolve workspace sessions DB path (`<root>/.litecode/sessions.db`).
pub fn sessions_db_under(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("sessions.db")
}

/// Iterate all detail rows as plain text (for semantic indexing).
pub(crate) fn iter_detail_texts(db_path: &Path) -> Result<Vec<(String, i64, String, String)>> {
    if !db_path.is_file() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;
    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut stmt = conn
        .prepare(
            "SELECT t.session_id, t.seq, t.item_type, t.body, t.body_ref
             FROM transcript_items t
             WHERE t.kind = 'detail'
             ORDER BY t.session_id ASC, t.seq ASC",
        )
        .map_err(|e| LitecodeError::Config(format!("session index prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RawRow {
                session_id: row.get(0)?,
                seq: row.get(1)?,
                item_type: row.get(2)?,
                body: row.get(3)?,
                body_ref: row.get(4)?,
            })
        })
        .map_err(|e| LitecodeError::Config(format!("session index query: {e}")))?;

    let mut out = Vec::new();
    for row in rows {
        let row = row.map_err(|e| LitecodeError::Config(format!("session index row: {e}")))?;
        let Some(text) = row_plain_text(&row, &data_root)? else {
            continue;
        };
        out.push((row.session_id, row.seq, row.item_type, text));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::store::Session;
    use crate::types::user_text;
    use tempfile::TempDir;

    fn seed_db(dir: &Path) -> (PathBuf, String, String) {
        let db = dir.join("sessions.db");
        let a = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        a.insert_detail_rows(&[user_text("alpha UNIQUE_SESSION_PHRASE omega")])
            .unwrap();
        let id_a = a.id.clone();

        let b = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        b.insert_detail_rows(&[user_text("other OTHER_MARKER content")])
            .unwrap();
        let id_b = b.id.clone();
        (db, id_a, id_b)
    }

    #[test]
    fn session_text_search_finds_seeded_transcript() {
        let dir = TempDir::new().unwrap();
        let (db, id_a, _) = seed_db(dir.path());

        let page = search(
            &db,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRASE".into(),
                offset: 0,
                include_session_id: None,
                project: None,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].session_id, id_a);
        assert_eq!(page.hits[0].seq, 0);
        assert!(page.hits[0].summary.contains("UNIQUE_SESSION_PHRASE"));
        assert!(!page.has_more);
    }

    #[test]
    fn session_text_search_case_insensitive() {
        let dir = TempDir::new().unwrap();
        let (db, id_a, _) = seed_db(dir.path());
        let page = search(
            &db,
            &SessionTextQuery {
                query: "unique_session_phrase".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].session_id, id_a);
        assert!((page.hits[0].score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn session_text_search_fuzzy_typo() {
        let dir = TempDir::new().unwrap();
        let (db, id_a, _) = seed_db(dir.path());
        let page = search(
            &db,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRAZE".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].session_id, id_a);
        assert!(page.hits[0].score >= FUZZY_THRESHOLD);
        assert!(page.hits[0].score < 1.0);
    }

    #[test]
    fn session_text_search_respects_scope() {
        let dir = TempDir::new().unwrap();
        let (db, id_a, id_b) = seed_db(dir.path());

        let page = search(
            &db,
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
            page.hits.is_empty(),
            "scoped to session A must not see B's marker"
        );

        let page_b = search(
            &db,
            &SessionTextQuery {
                query: "OTHER_MARKER".into(),
                offset: 0,
                include_session_id: Some(id_b.clone()),
                project: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page_b.hits.len(), 1);
        assert_eq!(page_b.hits[0].session_id, id_b);
    }

    #[test]
    fn session_text_search_pagination() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        let mut items = Vec::new();
        for i in 0..25 {
            items.push(user_text(format!(
                "pageable_token_{i:02} shared_needle_xyz"
            )));
        }
        s.insert_detail_rows(&items).unwrap();

        let page0 = search(
            &db,
            &SessionTextQuery {
                query: "shared_needle_xyz".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page0.hits.len(), RESULTS_PER_PAGE);
        assert!(page0.has_more);

        let page1 = search(
            &db,
            &SessionTextQuery {
                query: "shared_needle_xyz".into(),
                offset: RESULTS_PER_PAGE,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page1.hits.len(), RESULTS_PER_PAGE);
        assert!(page1.has_more);

        let page2 = search(
            &db,
            &SessionTextQuery {
                query: "shared_needle_xyz".into(),
                offset: RESULTS_PER_PAGE * 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page2.hits.len(), 5);
        assert!(!page2.has_more);
    }

    #[test]
    fn session_text_search_skips_empty_query() {
        let dir = TempDir::new().unwrap();
        let (db, _, _) = seed_db(dir.path());
        let err = search(
            &db,
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
        let page = search(
            &dir.path().join("nope.db"),
            &SessionTextQuery {
                query: "anything".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(page.hits.is_empty());
        assert!(!page.has_more);
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
    fn dual_lane_keeps_semantic_without_text_veto() {
        // Semantic-only sessions remain valid for the Related column.
        let sem = vec![SessionTextHit {
            session_id: "b".into(),
            seq: 1,
            item_type: "message".into(),
            summary: "other".into(),
            score: 0.9,
            char_start: 0,
            char_end: 1,
            lane: SessionHitLane::Semantic,
        }];
        let gated = gate_semantic_hits(sem);
        assert_eq!(gated.len(), 1);
        assert_eq!(gated[0].session_id, "b");
    }

    #[test]
    fn expand_window_clamps_and_bolds_core() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        let long = format!("{}NEEDLE{}", "a".repeat(500), "b".repeat(500));
        s.insert_detail_rows(&[user_text(&long)]).unwrap();
        let page = search(
            &db,
            &SessionTextQuery {
                query: "NEEDLE".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        let windows =
            expand_hit_windows(&db, &page.hits, ExpandSteps { up: 1, down: 1 }, None).unwrap();
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert!(
            w.body
                .contains(&format!("{HIT_MARK_OPEN}NEEDLE{HIT_MARK_CLOSE}"))
        );
        let plain = strip_hit_marks(&w.body);
        assert!(plain.chars().count() <= EXPAND_STEP_CHARS * 2 + HIT_CORE_MAX_CHARS + 10);
        assert!(w.chars_above > 0);
        assert!(w.chars_below > 0);
        // Single long item: no other items outside, but mid-item overflow.
        assert_eq!(w.items_above, 0);
        assert_eq!(w.items_below, 0);
        assert!(w.same_item_overflow);
    }

    #[test]
    fn expand_zero_is_core_only() {
        let dir = TempDir::new().unwrap();
        let (db, _, _) = seed_db(dir.path());
        let page = search(
            &db,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRASE".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let windows =
            expand_hit_windows(&db, &page.hits, ExpandSteps { up: 0, down: 0 }, None).unwrap();
        let plain = strip_hit_marks(&windows[0].body);
        assert!(plain.chars().count() <= HIT_CORE_MAX_CHARS);
        assert!(windows[0].body.contains(HIT_MARK_OPEN));
        assert!(windows[0].body.contains(HIT_MARK_CLOSE));
    }

    #[test]
    fn related_entry_truncates_without_hit_marks() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        let long = format!("HEAD{}", "z".repeat(RELATED_ENTRY_MAX_CHARS + 80));
        s.insert_detail_rows(&[user_text(&long)]).unwrap();
        let sid = s.id.clone();
        drop(s);

        let hit = SessionTextHit {
            session_id: sid,
            seq: 0,
            item_type: "message".into(),
            summary: "HEAD".into(),
            score: 0.9,
            char_start: 0,
            char_end: 0,
            lane: SessionHitLane::Semantic,
        };
        let windows = related_entry_windows(&db, &[hit]).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(!windows[0].body.contains(HIT_MARK_OPEN));
        assert_eq!(windows[0].body.chars().count(), RELATED_ENTRY_MAX_CHARS);
        assert!(windows[0].same_item_overflow);
        assert!(windows[0].chars_below > 0);
        assert!(windows[0].body.starts_with("HEAD"));
    }

    #[test]
    fn session_text_search_includes_function_call_rows() {
        use crate::authority::responses::FunctionToolCall;
        use crate::types::Item;

        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[Item::FunctionCall(FunctionToolCall {
            arguments: r#"{"cmd":"UNIQUE_TOOL_NEEDLE"}"#.into(),
            call_id: "c1".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        })])
        .unwrap();

        let page = search(
            &db,
            &SessionTextQuery {
                query: "UNIQUE_TOOL_NEEDLE".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].item_type, "function_call");
    }

    #[test]
    fn context_window_exclude_drops_live_seqs_keeps_archive() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        let sid = s.id.clone();
        s.insert_detail_rows(&[
            user_text("ARCHIVE_NEEDLE_ONLY"),
            user_text("LIVE_NEEDLE_ONLY"),
        ])
        .unwrap();
        s.apply_compact_checkpoint_from(&user_text("sum"), Some(1), 3)
            .unwrap();
        drop(s);

        let page = search(
            &db,
            &SessionTextQuery {
                query: "NEEDLE_ONLY".into(),
                exclude_context_window: Some(ContextWindowExclude {
                    session_id: sid.clone(),
                    kept_from_seq: 1,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].seq, 0);
        assert!(page.hits[0].summary.contains("ARCHIVE_NEEDLE_ONLY"));
    }

    #[test]
    fn exclude_session_ids_filters_rows() {
        let dir = TempDir::new().unwrap();
        let (db, id_a, id_b) = seed_db(dir.path());
        let page = search(
            &db,
            &SessionTextQuery {
                query: "OTHER_MARKER".into(),
                exclude_session_ids: vec![id_b.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(page.hits.is_empty());

        let page_a = search(
            &db,
            &SessionTextQuery {
                query: "UNIQUE_SESSION_PHRASE".into(),
                exclude_session_ids: vec![id_b],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page_a.hits.len(), 1);
        assert_eq!(page_a.hits[0].session_id, id_a);
    }

    #[test]
    fn parse_filter_tokens_accepts_common_forms() {
        let t = parse_filter_tokens("session:01ABC -session:01DEF -current").unwrap();
        assert_eq!(t.include_session.as_deref(), Some("01ABC"));
        assert_eq!(t.exclude_sessions, vec!["01DEF".to_string()]);
        assert!(t.exclude_current);

        let err = parse_filter_tokens("nope:x").unwrap_err();
        assert!(err.to_string().contains("unknown session_filter"));
    }

    #[test]
    fn resolve_session_ref_unique_prefix() {
        let dir = TempDir::new().unwrap();
        let (db, id_a, _) = seed_db(dir.path());
        let short = short_session_ref(&id_a);
        assert_eq!(resolve_session_ref(&db, short).unwrap(), id_a);
        assert_eq!(resolve_session_ref(&db, &id_a).unwrap(), id_a);
        let err = resolve_session_ref(&db, "ZZZZNOPE").unwrap_err();
        assert!(err.to_string().contains("matched no sessions"));
    }

    #[test]
    fn lexical_fts_indexes_on_insert_and_finds_token() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[user_text(
            "decision: use middleware for AuthRefactorToken login path",
        )])
        .unwrap();
        drop(s);

        let page = search(
            &db,
            &SessionTextQuery {
                query: "AuthRefactorToken".into(),
                offset: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
        assert!(page.hits[0].score >= 0.55);
        assert!(page.hits[0].summary.contains("AuthRefactorToken"));
    }

    #[test]
    fn lexical_backfill_recovers_pre_fts_rows() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[user_text("BACKFILL_ONLY_MARKER unique")])
            .unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            let _ = conn.execute_batch("DELETE FROM transcript_fts;");
        }
        let page = search(
            &db,
            &SessionTextQuery {
                query: "BACKFILL_ONLY_MARKER".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.hits.len(), 1);
    }
}
