//! Always-on lexical lane: exact substring + FTS5 + light fuzzy fallback.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use crate::session::SessionDataReader;
use crate::session::transcript_file::{SearchableRow, row_plain_text};
use crate::types::{LitecodeError, Result};

use super::{
    FUZZY_SCORE_CAP, HIT_CORE_MAX_CHARS, MatchHaystack, SessionHitLane, SessionTextHit,
    SessionTextQuery, filter_hits, match_in_haystack, prepare_haystack, snippet_from_span,
};

/// Over-fetch FTS candidates before filters / exact boost.
const FTS_CANDIDATE_LIMIT: usize = 64;
/// Cap fuzzy full-scan extras when FTS already returned hits.
const FUZZY_EXTRA_CAP: usize = 24;

/// Lexical search over detail rows (exact + FTS + fuzzy). Always-on.
/// `|` separates alternatives; any alternative may match.
pub fn search_lexical(
    reader: &SessionDataReader,
    query: &SessionTextQuery,
) -> Result<Vec<SessionTextHit>> {
    let patterns = split_patterns(query.query.trim());
    if patterns.is_empty() {
        return Err(LitecodeError::Config(
            "session search query is required".into(),
        ));
    }

    let data_root = reader.data_root();
    let mut by_key: HashMap<(String, i64), SessionTextHit> = HashMap::new();

    // FTS candidates across all alternatives (deduped, order-stable).
    let mut candidates: Vec<(String, i64)> = Vec::new();
    let mut seen_candidates: HashSet<(String, i64)> = HashSet::new();
    for pattern in &patterns {
        let fts_hits = reader
            .fts_search_blocking(
                pattern,
                query.include_session_id.as_deref(),
                FTS_CANDIDATE_LIMIT,
            )
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "session FTS search failed; continuing with exact/fuzzy");
                Vec::new()
            });
        for (session_id, seq, _text) in fts_hits {
            if seen_candidates.insert((session_id.clone(), seq)) {
                candidates.push((session_id, seq));
            }
        }
    }

    let rows = match reader.searchable_rows_blocking(query.include_session_id.as_deref()) {
        Ok(rows) => rows,
        Err(_) => return Ok(Vec::new()),
    };
    let mut row_by_key: HashMap<(String, i64), SearchableRow> = HashMap::new();
    for row in rows {
        row_by_key.insert((row.session_id.clone(), row.seq), row);
    }

    for (session_id, seq) in candidates {
        let Some(row) = row_by_key.get(&(session_id, seq)) else {
            continue;
        };
        let Some(text) = row_plain_text(row, data_root)? else {
            continue;
        };
        let hay = prepare_haystack(&text);
        let mut best: Option<SessionTextHit> = None;
        for pattern in &patterns {
            let hit = hit_from_searchable(row, &text, &hay, pattern, 0.85);
            if best.as_ref().map(|b| hit.score > b.score).unwrap_or(true) {
                best = Some(hit);
            }
        }
        if let Some(hit) = best {
            insert_best(&mut by_key, hit);
        }
    }

    let mut fuzzy_extras = 0usize;
    let fts_nonempty = !by_key.is_empty();
    for row in row_by_key.values() {
        if !row_allowed(row, query) {
            continue;
        }
        let key = (row.session_id.clone(), row.seq);
        let Some(text) = row_plain_text(row, data_root)? else {
            continue;
        };
        let hay = prepare_haystack(&text);
        let mut best: Option<(f64, usize, usize)> = None;
        for pattern in &patterns {
            if let Some((score, char_start, char_end)) = match_in_haystack(&hay, pattern)
                && best.as_ref().map(|(b, _, _)| score > *b).unwrap_or(true)
            {
                best = Some((score, char_start, char_end));
            }
        }
        let Some((score, char_start, char_end)) = best else {
            continue;
        };
        let is_exact = (score - 1.0).abs() < 1e-9;
        if !is_exact {
            if fts_nonempty && fuzzy_extras >= FUZZY_EXTRA_CAP {
                continue;
            }
            if by_key.contains_key(&key) {
                continue;
            }
            fuzzy_extras += 1;
        }
        let summary = snippet_from_span(&text, char_start, char_end);
        let hit = SessionTextHit {
            session_id: row.session_id.clone(),
            seq: row.seq,
            item_type: row.item_type.clone(),
            summary,
            score: if is_exact {
                1.0
            } else {
                score.min(FUZZY_SCORE_CAP)
            },
            char_start,
            char_end,
            lane: SessionHitLane::Text,
        };
        insert_best(&mut by_key, hit);
    }

    let mut ranked: Vec<SessionTextHit> = by_key.into_values().collect();
    ranked = filter_hits(ranked, query);
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.session_id.cmp(&b.session_id))
            .then_with(|| a.seq.cmp(&b.seq))
    });
    Ok(ranked)
}

/// Split a query on `|` into trimmed, non-empty, deduped alternatives.
fn split_patterns(query: &str) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for part in query.split('|') {
        let part = part.trim();
        if !part.is_empty() && seen.insert(part) {
            out.push(part.to_string());
        }
    }
    out
}

/// Keep the higher-scoring hit for the same `(session_id, seq)` key.
fn insert_best(map: &mut HashMap<(String, i64), SessionTextHit>, hit: SessionTextHit) {
    match map.entry((hit.session_id.clone(), hit.seq)) {
        Entry::Occupied(mut existing) => {
            if hit.score > existing.get().score + 1e-9 {
                existing.insert(hit);
            }
        }
        Entry::Vacant(slot) => {
            slot.insert(hit);
        }
    }
}

fn row_allowed(row: &SearchableRow, query: &SessionTextQuery) -> bool {
    if query
        .exclude_session_ids
        .iter()
        .any(|ex| !ex.is_empty() && ex == &row.session_id)
    {
        return false;
    }
    if let Some(win) = query.exclude_context_window.as_ref()
        && row.session_id == win.session_id
        && win.surface_seqs.iter().any(|s| *s == row.seq)
    {
        return false;
    }
    true
}

fn hit_from_searchable(
    row: &SearchableRow,
    text: &str,
    hay: &MatchHaystack,
    needle: &str,
    fts_score: f64,
) -> SessionTextHit {
    let (score, char_start, char_end) =
        if let Some((s, start, end)) = match_in_haystack(hay, needle) {
            if (s - 1.0).abs() < 1e-9 {
                (1.0, start, end)
            } else {
                (fts_score, start, end)
            }
        } else {
            let end = text.chars().count().min(HIT_CORE_MAX_CHARS);
            (fts_score, 0, end)
        };
    SessionTextHit {
        session_id: row.session_id.clone(),
        seq: row.seq,
        item_type: row.item_type.clone(),
        summary: snippet_from_span(text, char_start, char_end),
        score,
        char_start,
        char_end,
        lane: SessionHitLane::Text,
    }
}
