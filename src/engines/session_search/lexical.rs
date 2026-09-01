//! Always-on lexical lane: exact substring + FTS5 + light fuzzy fallback.

use std::collections::HashMap;

use crate::session::SessionDataReader;
use crate::session::transcript_file::{SearchableRow, row_plain_text};
use crate::types::{LitecodeError, Result};

use super::{
    FUZZY_THRESHOLD, HIT_CORE_MAX_CHARS, SessionHitLane, SessionTextHit, SessionTextQuery,
    filter_hits, fuzzy_match_span, snippet_from_span,
};

/// Over-fetch FTS candidates before filters / exact boost.
const FTS_CANDIDATE_LIMIT: usize = 64;
/// Cap fuzzy full-scan extras when FTS already returned hits.
const FUZZY_EXTRA_CAP: usize = 24;

/// Lexical search over detail rows (exact + FTS + fuzzy). Always-on.
pub fn search_lexical(
    reader: &SessionDataReader,
    query: &SessionTextQuery,
) -> Result<Vec<SessionTextHit>> {
    let needle = query.query.trim();
    if needle.is_empty() {
        return Err(LitecodeError::Config(
            "session search query is required".into(),
        ));
    }

    let data_root = reader.data_root();
    let mut by_key: HashMap<(String, i64), SessionTextHit> = HashMap::new();

    let fts_hits = reader
        .fts_search_blocking(
            needle,
            query.include_session_id.as_deref(),
            FTS_CANDIDATE_LIMIT,
        )
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "session FTS search failed; continuing with exact/fuzzy");
            Vec::new()
        });

    let rows = match reader.searchable_rows_blocking(query.include_session_id.as_deref()) {
        Ok(rows) => rows,
        Err(_) => return Ok(Vec::new()),
    };
    let mut row_by_key: HashMap<(String, i64), SearchableRow> = HashMap::new();
    for row in rows {
        row_by_key.insert((row.session_id.clone(), row.seq), row);
    }

    for (session_id, seq, _text) in fts_hits {
        let Some(row) = row_by_key.get(&(session_id, seq)) else {
            continue;
        };
        if let Some(hit) = hit_from_searchable(row, data_root, needle, 0.85)? {
            by_key.insert((hit.session_id.clone(), hit.seq), hit);
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
        let Some((score, char_start, char_end)) = fuzzy_match_span(&text, needle) else {
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
                score.min(FUZZY_THRESHOLD)
            },
            char_start,
            char_end,
            lane: SessionHitLane::Text,
        };
        by_key
            .entry(key)
            .and_modify(|existing| {
                if hit.score > existing.score
                    || ((hit.score - existing.score).abs() < 1e-9 && is_exact)
                {
                    *existing = hit.clone();
                }
            })
            .or_insert(hit);
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
    data_root: &std::path::Path,
    needle: &str,
    fts_score: f64,
) -> Result<Option<SessionTextHit>> {
    let Some(text) = row_plain_text(row, data_root)? else {
        return Ok(None);
    };
    let (score, char_start, char_end) =
        if let Some((s, start, end)) = fuzzy_match_span(&text, needle) {
            if (s - 1.0).abs() < 1e-9 {
                (1.0, start, end)
            } else {
                (fts_score, start, end)
            }
        } else {
            let end = text.chars().count().min(HIT_CORE_MAX_CHARS);
            (fts_score, 0, end)
        };
    Ok(Some(SessionTextHit {
        session_id: row.session_id.clone(),
        seq: row.seq,
        item_type: row.item_type.clone(),
        summary: snippet_from_span(&text, char_start, char_end),
        score,
        char_start,
        char_end,
        lane: SessionHitLane::Text,
    }))
}
