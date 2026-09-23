//! Session corpus — Lexical (always-on) + Semantic (ANN-only when engine Warm).
//!
//! Does not own session writes, schema migration, or ORT lifecycle.

mod chunk;
mod corpus;
mod derive;
mod echo;
#[cfg(test)]
mod dense_parity;
#[cfg(test)]
mod golden;
#[cfg(test)]
mod parity;
mod lexical;
mod semantic_index;
mod slots;
mod sparse;
mod tokenizer;

pub use semantic_index::{
    SessionSemanticIndex, consume_session_index, ensure_session_index, load_session_index,
    queue_session_dirty, read_session_pending_hint, session_index_status, session_should_rebuild,
    session_work_from_disk, session_work_now, write_session_pending_hint,
};
pub use lexical::ensure_sparse_index;
pub use sparse::sparse_index_path;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::session::SessionDataReader;
use crate::session::transcript_file::{self, TranscriptFile};
use crate::session::{count_text_tokens, truncate_text_tokens};
use crate::types::{LitecodeError, Result};

/// Semantic ANN over-fetch before gating / session filter.
pub const SEMANTIC_WINDOW: usize = 16;
/// Cap fuzzy-scan scores below FTS-confirmed hits (0.85): confirmed hits rank first.
pub const FUZZY_SCORE_CAP: f64 = 0.72;
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
    /// The rendered line the match itself lands on.
    line: u32,
    /// First and last rendered body line of the hit's item: a view shape is
    /// clamped to this range so context never leaks into a neighbouring item.
    first_line: u32,
    last_line: u32,
    /// Reader-facing type label of the item (`assistant reasoning`, `tool result`).
    label: String,
    summary: String,
}

/// Case-insensitive fuzzy search over all detail rows.
pub fn search(reader: &SessionDataReader, query: &SessionTextQuery) -> Result<Vec<SessionTextHit>> {
    search_all(reader, query)
}

/// All ranked lexical hits (no pagination).
///
/// Returns an error rather than an empty list when the lane could not run. The
/// two are indistinguishable to a reader and mean opposite things, and the whole
/// point of `LaneState` is to stop the second from wearing the first's clothes:
/// an empty `Ok` says "I looked, the history has nothing", which callers act on
/// by concluding the answer is not in the history at all.
pub fn search_all(
    reader: &SessionDataReader,
    query: &SessionTextQuery,
) -> Result<Vec<SessionTextHit>> {
    let (hits, state) = lexical::search_lexical(reader, query)?;
    match state.unanswered() {
        None => Ok(hits),
        Some(reason) => Err(LitecodeError::IndexNotReady(reason)),
    }
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

/// Agent-facing final ordering: score desc → caller family first → most
/// recently updated session first → stable (session_id, seq) for pagination.
///
/// The time key is the carrier of "newest first" among equal scores (mostly the
/// exact-match tier, where every hit scores 1.0). It is deliberately **not**
/// dropped: without it the fallback would be earliest-first (or the lane's ULID
/// order), i.e. the original would stop outranking later mentions. The rendered
/// view also carries `created`/`updated`, but that is for the reader, not a
/// substitute for the ordering.
pub fn sort_hits_for_agent(
    hits: &mut [SessionTextHit],
    prefer_session_ids: &[String],
    updated_at: &HashMap<String, i64>,
) {
    let prefer: HashSet<&str> = prefer_session_ids.iter().map(String::as_str).collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let ra = usize::from(!prefer.contains(a.session_id.as_str()));
                let rb = usize::from(!prefer.contains(b.session_id.as_str()));
                ra.cmp(&rb)
            })
            .then_with(|| {
                let ua = updated_at.get(&a.session_id).copied().unwrap_or(0);
                let ub = updated_at.get(&b.session_id).copied().unwrap_or(0);
                ub.cmp(&ua)
            })
            .then_with(|| a.session_id.cmp(&b.session_id))
            .then_with(|| a.seq.cmp(&b.seq))
    });
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
    let rows = dedup_chunks_to_rows(ranked);
    let (hydrated, _) = hydrate_hits(reader, &rows)?;
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

/// One row is one hit.
///
/// The lanes rank per chunk, so a long row can match at several offsets and every
/// one of those hits would hydrate to the same seq (and, before wrapping, to the
/// same line). The hit list is already rank-ordered, so the first chunk of a row
/// wins and the rest are folded away — `matches` then counts rows, not chunks.
fn dedup_chunks_to_rows(hits: &[SessionTextHit]) -> Vec<SessionTextHit> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if seen.insert((hit.session_id.clone(), hit.seq)) {
            out.push(hit.clone());
        }
    }
    out
}

fn hydrate_hits(
    reader: &SessionDataReader,
    hits: &[SessionTextHit],
) -> Result<(Vec<HydratedHit>, HashMap<String, TranscriptFile>)> {
    let mut cache: HashMap<String, TranscriptFile> = HashMap::new();
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if !cache.contains_key(&hit.session_id) {
            let file = reader.transcript_file_blocking(&hit.session_id)?;
            cache.insert(hit.session_id.clone(), file);
        }
        let file = cache.get(&hit.session_id).unwrap();
        // The index may be behind the store, and being behind is only ever
        // allowed to cost recall — never to put a row back that the live rule
        // keeps out. Both checks below are answered from the parse the renderer
        // already did: a seq the store no longer renders, and an echo copy
        // answering a session read (the lane that can go stale in practice).
        if file.is_echo_result(hit.seq) {
            continue;
        }
        let Some(line) = file.line_for_hit(hit.seq, hit.char_start, hit.char_end) else {
            continue;
        };
        let Some((first_line, last_line, label)) = body_span(file, hit.seq) else {
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
            first_line,
            last_line,
            label,
            summary,
        });
    }
    Ok((out, cache))
}

/// First and last rendered body line of one item, plus its reader-facing label.
///
/// The label is the item's type, plus the tool name for the rows that are a
/// tool call or a tool result — "which tool" is the question a bare `tool call`
/// leaves open.
fn body_span(file: &TranscriptFile, seq: i64) -> Option<(u32, u32, String)> {
    let mut first: Option<u32> = None;
    let mut last: Option<u32> = None;
    let mut label = String::new();
    for span in file
        .line_index
        .iter()
        .filter(|s| s.seq == seq && !s.is_header)
    {
        first.get_or_insert(span.line);
        last = Some(span.line);
        if label.is_empty() {
            label = transcript_file::type_label(&span.kind, &span.item_type);
            if let Some(tool) = file.tool_name(seq) {
                label = format!("{label} · {tool}");
            }
        }
    }
    Some((first?, last?, label))
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
        let rendered = packed_page_text(&candidate);
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

/// Size estimate of one packed page, in tokens.
///
/// The human page is drawn by the client from the structured
/// [`SessionSearchPage`], so this only has to be *proportional* to what the
/// client shows: a header block per group plus one line per hit.
fn packed_page_text(groups: &[SessionSearchGroup]) -> String {
    let mut parts = Vec::new();
    for group in groups {
        parts.push(format!("### {}", group.session_id));
        parts.push(format!("created: {}", group.created_time));
        parts.push(format!("updated: {}", group.updated_time));
        parts.push(format!("path: {}", group.path));
        parts.push(format!("matches: {}", group.match_count));
        parts.push(String::new());
        for hit in &group.hits {
            parts.push(format!("{}: {}", hit.line, hit.summary));
        }
    }
    parts.join("\n")
}

/// Token budget for one agent-facing view. The same value `grep` uses, so the
/// model meets one contract everywhere: one response is one window, and whatever
/// does not fit is named in a file instead of being paged with an offset.
pub const SEARCH_VIEW_TOKEN_BUDGET: usize = 2_000;

/// Lines of context shown around a hit when the whole item does not fit.
const HIT_CONTEXT_LINES: u32 = 2;

/// Shortest session handle inside a view; extended until it is unique.
const HANDLE_MIN_LEN: usize = SESSION_REF_SHORT_LEN;

/// The shape a whole view is rendered in. Every shape is judged against the
/// budget as a *whole* view, so a rich shape is never shown for the first few
/// hits only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewShape {
    /// Every hit's entire item.
    WholeItem,
    /// `HIT_CONTEXT_LINES` lines around every hit, clamped to its item.
    Context,
    /// The hit line alone.
    Line,
}

/// Build the agent-facing answer: sessions grouped, one token-bounded shape.
///
/// An empty string means nothing could be resolved — the caller reports "no
/// match" rather than a partial answer that reads like a whole one.
pub fn build_agent_view(
    reader: &SessionDataReader,
    ranked: &[SessionTextHit],
    workspace_root: &std::path::Path,
) -> Result<String> {
    build_agent_view_with_budget(reader, ranked, workspace_root, SEARCH_VIEW_TOKEN_BUDGET)
}

fn build_agent_view_with_budget(
    reader: &SessionDataReader,
    ranked: &[SessionTextHit],
    workspace_root: &std::path::Path,
    budget: usize,
) -> Result<String> {
    let rows = dedup_chunks_to_rows(ranked);
    let (hits, files) = hydrate_hits(reader, &rows)?;
    if hits.is_empty() {
        return Ok(String::new());
    }
    let session_ids = unique_session_order(&hits);
    let meta = load_session_meta(reader, &session_ids).unwrap_or_default();
    let handles = unique_handles(&session_ids);
    let now = chrono::Utc::now().timestamp_millis();
    let headers = view_headers(&hits, &session_ids, &meta, &handles, now);

    // View order: rank order, except that a session's hits stay together with the
    // header that names them.
    let ordered: Vec<&HydratedHit> = session_ids
        .iter()
        .flat_map(|sid| hits.iter().filter(move |h| &h.session_id == sid))
        .collect();

    for shape in [ViewShape::WholeItem, ViewShape::Context, ViewShape::Line] {
        let view = render_view(&ordered, &files, &headers, shape, ordered.len());
        if count_text_tokens(&view) <= budget {
            return Ok(view);
        }
    }

    // Nothing fits whole: carry as many hit lines as the budget allows and name
    // the rest in a file `read`/`grep` can page through.
    let slot = spill_slot(workspace_root);
    let location = slot
        .as_ref()
        .map(|s| s.location.clone())
        .unwrap_or_default();
    // Reserve the footer's longest form: it shrinks as hits are carried.
    let footer_bound = spill_footer(&ordered, &handles, &location, 0);
    let shown = fit_prefix(&ordered, &files, &headers, &footer_bound, budget);
    let body = render_view(&ordered, &files, &headers, ViewShape::Line, shown);
    if shown == ordered.len() {
        return Ok(body);
    }
    let written = slot
        .as_ref()
        .and_then(|s| {
            s.write(&ordered[shown..], &files, &handles)
                .map(|()| s.location.clone())
        })
        .unwrap_or(location);
    let footer = spill_footer(&ordered, &handles, &written, shown);
    Ok(format!("{body}{footer}"))
}

/// One header line per session — best handle, relative age, hit count — keyed by
/// session, so each hit can carry the header of the session it came from. The
/// count is that session's total, not what this view carries: it says whether
/// narrowing the query is worth it.
fn view_headers(
    hits: &[HydratedHit],
    order: &[String],
    meta: &HashMap<String, SessionTimestamps>,
    handles: &HashMap<String, String>,
    now: i64,
) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(order.len());
    for sid in order {
        let count = hits.iter().filter(|h| &h.session_id == sid).count();
        let handle = handles.get(sid).map(String::as_str).unwrap_or(sid.as_str());
        let updated = meta.get(sid).map(|t| t.updated_at).unwrap_or(0);
        out.insert(
            sid.clone(),
            format!(
                "### {handle} · {} · {count} Matches\n",
                format_age(now, updated)
            ),
        );
    }
    out
}

/// Render one shape for the first `take` hits of the view order. A session's
/// header is emitted together with its first hit, so no hit is ever left
/// unattributed.
fn render_view(
    ordered: &[&HydratedHit],
    files: &HashMap<String, TranscriptFile>,
    headers: &HashMap<String, String>,
    shape: ViewShape,
    take: usize,
) -> String {
    let mut out = String::new();
    let mut current: Option<&str> = None;
    for hit in ordered.iter().take(take) {
        if current != Some(hit.session_id.as_str()) {
            if let Some(header) = headers.get(&hit.session_id) {
                out.push_str(header);
            }
            current = Some(hit.session_id.as_str());
        }
        let (from, to) = match shape {
            ViewShape::WholeItem => (hit.first_line, hit.last_line),
            ViewShape::Context => (
                hit.line
                    .saturating_sub(HIT_CONTEXT_LINES)
                    .max(hit.first_line),
                (hit.line + HIT_CONTEXT_LINES).min(hit.last_line),
            ),
            ViewShape::Line => (hit.line, hit.line),
        };
        let label = if from == to {
            format!("L{from}")
        } else {
            format!("L{from}-{to}")
        };
        out.push_str(&format!("{label}: {}\n", hit.label));
        if let Some(file) = files.get(&hit.session_id) {
            for line in from..=to {
                if let Some(text) = file.line_text(line) {
                    out.push_str("  ");
                    out.push_str(text);
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Largest prefix of the single-line shape whose whole view still fits.
fn fit_prefix(
    ordered: &[&HydratedHit],
    files: &HashMap<String, TranscriptFile>,
    headers: &HashMap<String, String>,
    footer: &str,
    budget: usize,
) -> usize {
    let mut best = 0;
    for n in 1..=ordered.len() {
        let body = render_view(ordered, files, headers, ViewShape::Line, n);
        if count_text_tokens(&format!("{body}{footer}")) > budget {
            break;
        }
        best = n;
    }
    best
}

/// Footer of a view that could not carry every hit: what was shown, where the
/// rest is, and how many per session are missing.
fn spill_footer(
    ordered: &[&HydratedHit],
    handles: &HashMap<String, String>,
    location: &str,
    shown: usize,
) -> String {
    let total = ordered.len();
    let remaining = total.saturating_sub(shown);
    let mut counts: Vec<(String, usize)> = Vec::new();
    for hit in ordered.iter().skip(shown) {
        let handle = handles
            .get(&hit.session_id)
            .cloned()
            .unwrap_or_else(|| hit.session_id.clone());
        match counts.iter_mut().find(|(h, _)| h == &handle) {
            Some((_, n)) => *n += 1,
            None => counts.push((handle, 1)),
        }
    }
    let per_session = counts
        .iter()
        .map(|(handle, n)| format!("{handle} {n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!(
        "\nShowing {shown} of {total} hits; the remaining {remaining} are in {location} — read or grep it."
    );
    if !per_session.is_empty() {
        out.push_str(&format!("\nMore in: {per_session}."));
    }
    out.push('\n');
    out
}

/// Relative age, front-end style: `just now`, `5m ago`, `2h ago`, `3d ago`,
/// `2w ago`, `4mo ago`, `1y+ ago`. An absolute timestamp would only mean
/// something to a reader who already knows the current time.
fn format_age(now_ms: i64, then_ms: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;
    let secs = (now_ms - then_ms).max(0) / 1000;
    if secs < MINUTE {
        "just now".into()
    } else if secs < HOUR {
        format!("{}m ago", secs / MINUTE)
    } else if secs < DAY {
        format!("{}h ago", secs / HOUR)
    } else if secs < WEEK {
        format!("{}d ago", secs / DAY)
    } else if secs < MONTH {
        format!("{}w ago", secs / WEEK)
    } else if secs < YEAR {
        format!("{}mo ago", secs / MONTH)
    } else {
        format!("{}y+ ago", secs / YEAR)
    }
}

/// Session handles for one view: the shortest unique trailing slice, at least
/// [`HANDLE_MIN_LEN`] chars. `resolve_session_ref` accepts a unique suffix, so a
/// printed handle can be pasted into `session_id` or into
/// `.litecode/sessions/<handle>.md`.
fn unique_handles(ids: &[String]) -> HashMap<String, String> {
    let longest = ids.iter().map(|id| id.chars().count()).max().unwrap_or(0);
    let mut len = HANDLE_MIN_LEN;
    loop {
        let mut seen: HashSet<String> = HashSet::new();
        let mut unique = true;
        let mut out = HashMap::new();
        for id in ids {
            let handle = tail_chars(id, len);
            if !seen.insert(handle.clone()) {
                unique = false;
            }
            out.insert(id.clone(), handle);
        }
        if unique || len >= longest {
            return out;
        }
        len += 1;
    }
}

fn tail_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    if count <= n {
        return text.to_string();
    }
    text.chars().skip(count - n).collect()
}

/// A file under `.litecode/bash/` for the hits one view cannot carry. Reserved
/// before the view is fitted so the footer can name it; only written when
/// something is actually left over.
struct SpillSlot {
    path: std::path::PathBuf,
    /// Workspace-relative path with `/` separators, ready for `read`.
    location: String,
}

impl SpillSlot {
    /// Write exactly the hits the view could not carry — never a second copy of
    /// what the reader already has.
    fn write(
        &self,
        ordered: &[&HydratedHit],
        files: &HashMap<String, TranscriptFile>,
        handles: &HashMap<String, String>,
    ) -> Option<()> {
        if ordered.is_empty() {
            return None;
        }
        let mut body = format!(
            "Remaining {} session-search hits not shown inline.\n\n",
            ordered.len()
        );
        for hit in ordered {
            let handle = handles
                .get(&hit.session_id)
                .map(String::as_str)
                .unwrap_or(&hit.session_id);
            // Exactly the line the single-line shape would have shown, so the
            // spilled form and `read` agree about what `L<n>` holds.
            let text = files
                .get(&hit.session_id)
                .and_then(|f| f.line_text(hit.line))
                .unwrap_or(hit.summary.as_str());
            body.push_str(&format!("{handle} L{}: {} {}\n", hit.line, hit.label, text));
        }
        std::fs::write(&self.path, body).ok()?;
        Some(())
    }
}

fn spill_slot(workspace_root: &std::path::Path) -> Option<SpillSlot> {
    let dir = workspace_root.join(".litecode").join("bash");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!(
        "session_search_{}.txt",
        crate::terminal::bash_nonce()
    ));
    let location = crate::workspace::filter::cheap_rel_under(workspace_root, &path)
        .map(|rel| rel.replace('\\', "/"))
        .unwrap_or_else(|| path.display().to_string());
    Some(SpillSlot { path, location })
}

/// Match-ready view of one text: normalized chars + a map from normalized
/// char index back to the original char index (translates spans for snippets).
pub(crate) struct MatchHaystack {
    normalized: String,
    chars: Vec<char>,
    /// Byte offset of each normalized char inside `normalized`.
    char_bytes: Vec<usize>,
    map: Vec<usize>,
    orig_len: usize,
}

/// Fold a text once for all alternatives: per-char lowercase, fullwidth →
/// halfwidth (incl. U+3000), whitespace runs collapsed to one space, while
/// keeping a position map so spans can be translated back to the original.
fn normalize_for_match(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len());
    let mut prev_space = false;
    for (idx, raw) in text.chars().enumerate() {
        let ch = match raw {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(raw as u32 - 0xFEE0).unwrap_or(raw),
            _ => raw,
        };
        if ch.is_whitespace() {
            if prev_space {
                continue;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
        for lower in ch.to_lowercase() {
            out.push(lower);
            map.push(idx);
        }
    }
    (out, map)
}

pub(crate) fn prepare_haystack(text: &str) -> MatchHaystack {
    let (normalized, map) = normalize_for_match(text);
    let mut chars = Vec::with_capacity(normalized.len());
    let mut char_bytes = Vec::with_capacity(normalized.len());
    for (byte, ch) in normalized.char_indices() {
        chars.push(ch);
        char_bytes.push(byte);
    }
    MatchHaystack {
        normalized,
        chars,
        char_bytes,
        map,
        orig_len: text.chars().count(),
    }
}

/// Allowed edits by needle char length — the Elasticsearch `AUTO` ladder
/// (0 for ≤2 chars to avoid noise, 1 for 3–5, 2 beyond, capped there).
fn fuzzy_edit_budget(needle_chars: usize) -> usize {
    match needle_chars {
        0..=2 => 0,
        3..=5 => 1,
        _ => 2,
    }
}

/// Returns `(score, char_start, char_end)` in original char positions when the
/// needle matches within the length-banded edit budget.
///
/// Approximate pass uses the pigeonhole split: with edit budget `k`, at least
/// one of the `2k+1` needle chunks stays untouched in any within-budget text
/// window (a transposition touches at most two chunks), so chunk occurrences
/// are the only alignment anchors worth probing.
pub(crate) fn match_in_haystack(hs: &MatchHaystack, needle: &str) -> Option<(f64, usize, usize)> {
    let ned = normalize_for_match(needle).0;
    let ned_chars: Vec<char> = ned.chars().collect();
    let n = ned_chars.len();
    if n == 0 {
        return None;
    }
    if let Some(byte_start) = hs.normalized.find(&ned) {
        let char_start = hs.normalized[..byte_start].chars().count();
        let (start, end) = map_span(hs, char_start, char_start + n);
        return Some((1.0, start, end));
    }
    let budget = fuzzy_edit_budget(n);
    if budget == 0 {
        return None;
    }
    let chunk_count = 2 * budget + 1;
    let base = n / chunk_count;
    let rem = n % chunk_count;
    let mut best_dist = usize::MAX;
    let mut best_start = 0usize;
    let mut offset = 0usize;
    'search: for i in 0..chunk_count {
        let len = base + usize::from(i < rem);
        if len == 0 {
            continue;
        }
        let chunk: String = ned_chars[offset..offset + len].iter().collect();
        let mut from = 0usize;
        while let Some(rel) = hs.normalized[from..].find(chunk.as_str()) {
            let byte_pos = from + rel;
            let occ_char = hs.char_bytes.partition_point(|&b| b < byte_pos);
            let cand = occ_char as isize - offset as isize;
            for delta in -(budget as isize)..=(budget as isize) {
                let start = cand + delta;
                if start < 0 {
                    continue;
                }
                let start = start as usize;
                if start + n > hs.chars.len() {
                    continue;
                }
                let window: String = hs.chars[start..start + n].iter().collect();
                let dist = strsim::damerau_levenshtein(&window, &ned);
                if dist < best_dist {
                    best_dist = dist;
                    best_start = start;
                }
                if best_dist <= 1 {
                    break 'search;
                }
            }
            from = byte_pos + chunk.len();
            if from >= hs.normalized.len() {
                break;
            }
        }
        offset += len;
    }
    if best_dist > budget {
        return None;
    }
    let score = 1.0 - (best_dist as f64 / n as f64);
    let (start, end) = map_span(hs, best_start, best_start + n);
    Some((score, start, end))
}

fn map_span(hs: &MatchHaystack, char_start: usize, char_end: usize) -> (usize, usize) {
    let start = hs.map.get(char_start).copied().unwrap_or(hs.orig_len);
    let end = if char_end == 0 {
        0
    } else {
        hs.map
            .get(char_end - 1)
            .map(|i| i + 1)
            .unwrap_or(hs.orig_len)
    };
    (start.min(hs.orig_len), end.min(hs.orig_len))
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
        assert!(hits[0].score > 0.5, "fuzzy hit should score above 0.5");
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

    /// The three states a lane can be in, and the one thing they must not do:
    /// look alike.
    ///
    /// "Nothing matched" and "the search never ran" both arrive as an empty list,
    /// and they could not mean more different things. A caller that reads the
    /// second as the first concludes the history does not hold the answer and
    /// stops — which is right for an empty corpus and a permanent false negative
    /// for an index that was merely broken or late. So an unanswered search is an
    /// error, and an answered one is not, and the tests below pin both halves.
    mod lane_state {
        use super::*;
        use crate::engines::session_search::sparse;
        use crate::types::LitecodeError;

        /// Plant a row that is searchable in shape and unreadable in body: the
        /// reconcile will schedule it, and the derivation will then refuse it.
        fn plant_unreadable_row(dir: &Path, session_id: &str) {
            let conn = rusqlite::Connection::open(dir.join("sessions.db")).unwrap();
            conn.execute(
                "INSERT INTO transcript_items
                    (session_id, seq, turn_id, turn_seq, item_type, kind, body,
                     token_estimate, created_at, event_type, surface_op, state)
                 VALUES (?1, 99, 't', 0, 'message', 'item/user', '{{{ not an item',
                         1, 1, 'message', 'append', 'final')",
                rusqlite::params![session_id],
            )
            .unwrap();
        }

        fn q(text: &str) -> SessionTextQuery {
            SessionTextQuery {
                query: text.into(),
                offset: 0,
                ..Default::default()
            }
        }

        /// `Ready`: the query ran, and an empty result is a fact about the corpus.
        #[test]
        fn ready_is_a_real_answer_even_when_it_is_empty() {
            let dir = TempDir::new().unwrap();
            let (reader, _, _) = seed_db(dir.path());
            let hits = search(&reader, &q("zqxjvkjv")).unwrap();
            assert!(
                hits.is_empty(),
                "a searched corpus with no match answers with nothing"
            );
        }

        /// `Failed`: the index could not be prepared, so the query did not run and
        /// the caller is told, rather than handed the empty list that would have
        /// been indistinguishable from `Ready` above.
        #[test]
        fn a_broken_index_is_an_error_not_an_empty_result() {
            let dir = TempDir::new().unwrap();
            let (reader, id_a, _) = seed_db(dir.path());

            // A first search builds the index, so the missing one below is a real
            // rebuild rather than a first run.
            assert_eq!(search(&reader, &q("UNIQUE_SESSION_PHRASE")).unwrap().len(), 1);

            plant_unreadable_row(dir.path(), &id_a);
            std::fs::remove_file(sparse::sparse_index_path(dir.path())).unwrap();

            let err = search(&reader, &q("UNIQUE_SESSION_PHRASE")).unwrap_err();
            assert!(
                matches!(err, LitecodeError::IndexNotReady(_)),
                "the lane must say it did not answer, not that it found nothing: {err}"
            );
            assert!(
                err.to_string().contains("did not run"),
                "and say it plainly: {err}"
            );
        }

        /// `Failed` must not be reachable through `ensure_sparse_index` as a
        /// success either — the blocking warmup makes the same promise.
        #[test]
        fn warmup_reports_a_broken_index_too() {
            let dir = TempDir::new().unwrap();
            let (reader, id_a, _) = seed_db(dir.path());
            assert!(lexical::ensure_sparse_index(&reader).is_ok());
            plant_unreadable_row(dir.path(), &id_a);
            std::fs::remove_file(sparse::sparse_index_path(dir.path())).unwrap();
            assert!(matches!(
                lexical::ensure_sparse_index(&reader),
                Err(LitecodeError::IndexNotReady(_))
            ));
        }

        /// The reconcile happens *before* the query, not after it. A search that
        /// answered from a stale index and tidied up afterwards would return the
        /// hits of yesterday and look perfectly healthy doing it.
        #[test]
        fn a_stale_index_is_caught_up_before_the_query_runs() {
            let dir = TempDir::new().unwrap();
            let (reader, id_a, _) = seed_db(dir.path());
            assert!(
                search(&reader, &q("zqxjvkjv")).unwrap().is_empty(),
                "the marker does not exist yet"
            );

            {
                let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
                let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
                data.insert_items(&id_a, &[user_text("a zqxjvkjv arrives")])
                    .unwrap();
            }

            let hits = search(&reader, &q("zqxjvkjv")).unwrap();
            assert_eq!(
                hits.len(),
                1,
                "the row written before the search is in its results"
            );
        }

        /// Searches that arrive together take turns. The lock is what keeps two of
        /// them from rebuilding the same index at once; equally important is that
        /// the second one, on its turn, finds the work already done rather than
        /// repeating it. Either way the answers must agree with each other and with
        /// what a single search would have said.
        #[test]
        fn concurrent_searches_agree_and_none_is_left_unanswered() {
            let dir = TempDir::new().unwrap();
            let (reader, _, _) = seed_db(dir.path());
            let expected = search(&reader, &q("UNIQUE_SESSION_PHRASE"));
            let expected = expected.unwrap();
            assert_eq!(expected.len(), 1);

            let readers: Vec<SessionDataReader> =
                (0..4).map(|_| SessionDataReader::open(&dir.path().join("sessions.db"))).collect();
            let handles: Vec<_> = readers
                .into_iter()
                .map(|r| {
                    std::thread::spawn(move || search(&r, &q("UNIQUE_SESSION_PHRASE")))
                })
                .collect();

            for handle in handles {
                let hits = handle
                    .join()
                    .expect("a concurrent search must not panic")
                    .expect("a concurrent search must be answered");
                assert_eq!(hits.len(), 1, "and answered the same way");
            }
        }
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
    fn chunk_hits_on_one_row_collapse_to_a_single_hit() {
        let hit = |seq: i64, start: usize| SessionTextHit {
            session_id: "s".into(),
            seq,
            item_type: "message".into(),
            summary: "chunk".into(),
            score: 1.0,
            char_start: start,
            char_end: start + 4,
            lane: SessionHitLane::Text,
        };
        // The same row matched at three chunk offsets, plus a second row.
        let rows = dedup_chunks_to_rows(&[hit(7, 0), hit(7, 320), hit(9, 0), hit(7, 900)]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 7);
        assert_eq!(rows[0].char_start, 0, "the best-ranked chunk wins");
        assert_eq!(rows[1].seq, 9);
    }

    #[test]
    fn agent_view_shows_the_whole_item_when_it_fits() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(&id, &[user_text("one\nVIEW_NEEDLE two\nthree")])
                .unwrap();
            id
        };
        let reader = SessionDataReader::open(&db);
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "VIEW_NEEDLE".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let view = build_agent_view(&reader, &hits, dir.path()).unwrap();
        // The documented shape, pinned: session handle + age + count, then one
        // label line per hit and the indented lines of its range.
        assert_eq!(
            view,
            format!(
                "### {} · just now · 1 Matches\nL2-4: user\n  one\n  VIEW_NEEDLE two\n  three\n",
                short_session_ref(&sid)
            ),
            "{view}"
        );
        // Internal coordinates stay out of the view.
        assert!(!view.contains("seq"), "{view}");
        assert!(!view.contains("Showing"), "{view}");
    }

    #[test]
    fn agent_view_names_the_tool_of_a_call_and_its_result() {
        use crate::authority::responses::{
            FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
        };
        use crate::types::Item;

        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(
                &id,
                &[
                    Item::FunctionCall(FunctionToolCall {
                        arguments: r#"{"command":"grep TOOL_NAME_NEEDLE"}"#.into(),
                        call_id: "call_1".into(),
                        namespace: None,
                        name: "bash".into(),
                        id: None,
                        status: None,
                    }),
                    Item::FunctionCallOutput(FunctionCallOutputItemParam {
                        call_id: "call_1".into(),
                        output: FunctionCallOutput::Text("TOOL_NAME_NEEDLE found".into()),
                        id: None,
                        status: None,
                    }),
                ],
            )
            .unwrap();
            id
        };
        let reader = SessionDataReader::open(&db);
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "TOOL_NAME_NEEDLE".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 2, "the call and its result both match");

        let view = build_agent_view(&reader, &hits, dir.path()).unwrap();
        assert!(view.contains("tool call · bash"), "{view}");
        assert!(view.contains("tool result · bash"), "{view}");
    }

    #[test]
    fn agent_view_spills_the_remainder_into_a_named_file() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            let items: Vec<_> = (0..12)
                .map(|i| {
                    user_text(format!(
                        "DEGRADE_NEEDLE {i} filled with words to spend budget"
                    ))
                })
                .collect();
            data.insert_items(&id, &items).unwrap();
            id
        };
        let reader = SessionDataReader::open(&db);
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "DEGRADE_NEEDLE".into(),
                ..Default::default()
            },
        )
        .unwrap();

        // A budget too small for the whole set at any shape: a prefix of the
        // single-line shape is carried and the rest is named.
        let view = build_agent_view_with_budget(&reader, &hits, dir.path(), 90).unwrap();
        assert!(view.contains("Showing "), "{view}");
        assert!(
            view.contains("are in .litecode/bash/session_search_"),
            "{view}"
        );
        assert!(view.contains("More in: "), "{view}");
        let location = view
            .split(" are in ")
            .nth(1)
            .and_then(|rest| rest.split(" —").next())
            .expect(&view);
        let spilled = std::fs::read_to_string(dir.path().join(location)).unwrap();
        assert!(spilled.contains("Remaining "), "{spilled}");
        let entries: Vec<&str> = spilled
            .lines()
            .filter(|l| !l.trim().is_empty())
            .skip(1)
            .collect();
        assert!(!entries.is_empty(), "{spilled}");
        for entry in entries {
            assert!(
                entry.starts_with(&format!("{} L", short_session_ref(&sid))),
                "every spilled hit names its session: {entry}"
            );
            assert!(
                entry.contains("filled with words to spend budget"),
                "the spilled line is the rendered line: {entry}"
            );
        }

        // The real budget carries the same corpus whole: no file, no footer.
        let whole = build_agent_view(&reader, &hits, dir.path()).unwrap();
        assert!(!whole.contains("Showing"), "{whole}");
    }

    #[test]
    fn every_hit_travels_with_the_header_of_its_session() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let (a, b) = {
            let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let a = data.create_session("/proj", "default", None).unwrap();
            let b = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(&a, &[user_text("TWO_SESSION_MARKER from a")])
                .unwrap();
            data.insert_items(&b, &[user_text("TWO_SESSION_MARKER from b")])
                .unwrap();
            (a, b)
        };
        let reader = SessionDataReader::open(&db);
        let hits = search(
            &reader,
            &SessionTextQuery {
                query: "TWO_SESSION_MARKER".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 2, "both sessions must hit");

        let view = build_agent_view(&reader, &hits, dir.path()).unwrap();
        let mut current = String::new();
        let mut attributed = 0;
        for line in view.lines() {
            if let Some(rest) = line.strip_prefix("### ") {
                current = rest.split(" · ").next().unwrap_or_default().to_string();
            } else if let Some(body) = line.strip_prefix("  ") {
                let expected = if body.contains(" from a") {
                    short_session_ref(&a)
                } else {
                    short_session_ref(&b)
                };
                assert!(
                    current.ends_with(&expected),
                    "a hit must sit under its own session header:\n{view}"
                );
                attributed += 1;
            }
        }
        assert_eq!(attributed, 2, "{view}");
    }

    #[test]
    fn handles_extend_until_they_are_unique() {
        let ids = vec![
            "01AAAA000000000001".to_string(),
            "01BBBB000000000001".to_string(),
        ];
        let handles = unique_handles(&ids);
        let a = handles.get(&ids[0]).unwrap();
        let b = handles.get(&ids[1]).unwrap();
        assert_ne!(a, b);
        assert!(a.chars().count() > SESSION_REF_SHORT_LEN, "{a}");
        assert!(ids[0].ends_with(a.as_str()), "{a}");
    }

    #[test]
    fn age_reads_as_relative_time() {
        let now = 1_700_000_000_000i64;
        assert_eq!(format_age(now, now), "just now");
        assert_eq!(format_age(now, now - 5 * 60_000), "5m ago");
        assert_eq!(format_age(now, now - 2 * 3_600_000), "2h ago");
        assert_eq!(format_age(now, now - 3 * 86_400_000), "3d ago");
        assert_eq!(format_age(now, now - 8 * 86_400_000), "1w ago");
        assert_eq!(format_age(now, now - 40 * 86_400_000), "1mo ago");
        assert_eq!(format_age(now, now - 400 * 86_400_000), "1y+ ago");
    }

    #[test]
    fn pack_page_stops_on_whole_hit_and_advances_offset() {
        let hits: Vec<HydratedHit> = (0..8)
            .map(|i| HydratedHit {
                session_id: "s".into(),
                seq: i,
                line: (i + 1) as u32,
                first_line: (i + 1) as u32,
                last_line: (i + 1) as u32,
                label: "user".into(),
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
            first_line: 1,
            last_line: 1,
            label: "user".into(),
            summary: "only".into(),
        }];
        let page = pack_page(&hits, &HashMap::new(), &HashMap::new(), 3, 6000);
        assert!(page.groups.is_empty());
        assert_eq!(page.next_offset, 3);
        assert!(!page.has_more);
    }
}
