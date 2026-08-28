//! Always-on lexical lane: exact substring + FTS5 bm25 + light fuzzy fallback.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::session::store::data_root_from_db_path;
use crate::session::transcript_fts::{self, FtsHit};
use crate::types::{LitecodeError, Result};

use super::{
    FUZZY_THRESHOLD, HIT_CORE_MAX_CHARS, RawRow, SessionHitLane, SessionTextHit, SessionTextQuery,
    filter_hits, fuzzy_match_span, row_plain_text, snippet_from_span,
};

fn raw_row(
    session_id: String,
    seq: i64,
    item_type: String,
    body: Option<String>,
    body_ref: Option<String>,
) -> RawRow {
    RawRow {
        session_id,
        seq,
        kind: String::new(),
        item_type,
        body,
        body_ref,
    }
}

/// Over-fetch FTS candidates before filters / exact boost.
const FTS_CANDIDATE_LIMIT: usize = 64;
/// Cap fuzzy full-scan extras when FTS already returned hits.
const FUZZY_EXTRA_CAP: usize = 24;

/// Lexical search over detail rows (exact + FTS bm25 + fuzzy). Always-on.
pub fn search_lexical(db_path: &Path, query: &SessionTextQuery) -> Result<Vec<SessionTextHit>> {
    let needle = query.query.trim();
    if needle.is_empty() {
        return Err(LitecodeError::Config(
            "session search query is required".into(),
        ));
    }
    if !db_path.is_file() {
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db: {e}")))?;
    transcript_fts::ensure_schema(&conn)?;
    if transcript_fts::needs_backfill(&conn)? {
        backfill_fts(&conn, db_path)?;
    }

    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut by_key: HashMap<(String, i64), SessionTextHit> = HashMap::new();

    // --- FTS5 / bm25 ---
    let match_q = transcript_fts::escape_match_query(needle);
    if !match_q.is_empty() {
        let fts_hits = transcript_fts::search(
            &conn,
            &match_q,
            FTS_CANDIDATE_LIMIT,
            query.include_session_id.as_deref(),
        )
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "session FTS search failed; continuing with exact/fuzzy");
            Vec::new()
        });
        for fh in fts_hits {
            if let Some(hit) =
                hit_from_row(&conn, &data_root, &fh, needle, /*prefer_exact*/ true)?
            {
                by_key.insert((hit.session_id.clone(), hit.seq), hit);
            }
        }
    }

    // --- Exact / fuzzy scan over detail (fills gaps + typo recall) ---
    let mut sql = String::from(
        "SELECT t.session_id, t.seq, t.item_type, t.body, t.body_ref
         FROM transcript_items t
         INNER JOIN sessions s ON s.id = t.session_id
         WHERE t.kind IN ('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted')",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(sid) = query.include_session_id.as_ref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND t.session_id = ?");
        params.push(Box::new(sid.clone()));
    }
    for ex in &query.exclude_session_ids {
        if ex.is_empty() {
            continue;
        }
        sql.push_str(" AND t.session_id != ?");
        params.push(Box::new(ex.clone()));
    }
    if let Some(win) = query.exclude_context_window.as_ref()
        && !win.surface_seqs.is_empty()
    {
        sql.push_str(" AND NOT (t.session_id = ? AND t.seq IN (");
        params.push(Box::new(win.session_id.clone()));
        for (i, seq) in win.surface_seqs.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push('?');
            params.push(Box::new(*seq));
        }
        sql.push_str("))");
    }
    if let Some(project) = query.project.as_ref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND s.project = ?");
        params.push(Box::new(project.clone()));
    }
    sql.push_str(" ORDER BY t.session_id ASC, t.seq ASC");

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| LitecodeError::Config(format!("lexical scan prepare: {e}")))?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(raw_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|e| LitecodeError::Config(format!("lexical scan query: {e}")))?;

    let mut fuzzy_extras = 0usize;
    let fts_nonempty = !by_key.is_empty();
    for row in rows {
        let row = row.map_err(|e| LitecodeError::Config(format!("lexical scan row: {e}")))?;
        let key = (row.session_id.clone(), row.seq);
        let Some(text) = row_plain_text(&row, &data_root)? else {
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
            session_id: row.session_id,
            seq: row.seq,
            item_type: row.item_type,
            summary,
            score: if is_exact {
                1.0
            } else {
                // Keep fuzzy below typical FTS-normalized scores.
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
    // Apply exclude filters for FTS-sourced rows that skipped SQL predicates.
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

fn hit_from_row(
    conn: &Connection,
    data_root: &Path,
    fh: &FtsHit,
    needle: &str,
    prefer_exact: bool,
) -> Result<Option<SessionTextHit>> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, seq, item_type, body, body_ref FROM transcript_items
             WHERE session_id = ?1 AND seq = ?2
               AND kind IN ('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted')",
        )
        .map_err(|e| LitecodeError::Config(format!("fts hydrate prepare: {e}")))?;
    let row = stmt
        .query_row(rusqlite::params![fh.session_id, fh.seq], |r| {
            Ok(raw_row(
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })
        .optional()
        .map_err(|e| LitecodeError::Config(format!("fts hydrate: {e}")))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let Some(text) = row_plain_text(&row, data_root)? else {
        return Ok(None);
    };
    let (score, char_start, char_end) = if prefer_exact {
        if let Some(span) = fuzzy_match_span(&text, needle) {
            if (span.0 - 1.0).abs() < 1e-9 {
                span
            } else {
                // FTS hit without exact span: use bm25-derived score + best fuzzy span or prefix.
                let bm25_score = bm25_to_unit(fh.bm25);
                if let Some((_, s, e)) = fuzzy_match_span(&text, needle) {
                    (bm25_score, s, e)
                } else {
                    let end = text.chars().count().min(HIT_CORE_MAX_CHARS);
                    (bm25_score, 0, end)
                }
            }
        } else {
            let bm25_score = bm25_to_unit(fh.bm25);
            let end = text.chars().count().min(HIT_CORE_MAX_CHARS);
            (bm25_score, 0, end)
        }
    } else {
        let bm25_score = bm25_to_unit(fh.bm25);
        let end = text.chars().count().min(HIT_CORE_MAX_CHARS);
        (bm25_score, 0, end)
    };
    Ok(Some(SessionTextHit {
        session_id: row.session_id,
        seq: row.seq,
        item_type: row.item_type,
        summary: snippet_from_span(&text, char_start, char_end),
        score,
        char_start,
        char_end,
        lane: SessionHitLane::Text,
    }))
}

fn bm25_to_unit(bm25: f64) -> f64 {
    // SQLite bm25 is typically <= 0 (more negative = better). Map to (0, 1).
    let s = 1.0 / (1.0 + (-bm25).max(0.0));
    // Keep below exact (1.0), above fuzzy threshold band when strong.
    s.clamp(0.55, 0.99)
}

fn backfill_fts(conn: &Connection, db_path: &Path) -> Result<()> {
    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut stmt = conn
        .prepare(
            "SELECT session_id, seq, item_type, body, body_ref FROM transcript_items
             WHERE kind IN ('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted')
             ORDER BY session_id, seq",
        )
        .map_err(|e| LitecodeError::Config(format!("fts backfill prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(raw_row(
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })
        .map_err(|e| LitecodeError::Config(format!("fts backfill query: {e}")))?;
    let mut n = 0usize;
    for row in rows {
        let row = row.map_err(|e| LitecodeError::Config(format!("fts backfill row: {e}")))?;
        let Some(text) = row_plain_text(&row, &data_root)? else {
            continue;
        };
        transcript_fts::upsert(conn, &row.session_id, row.seq, &text)?;
        n += 1;
    }
    tracing::info!(rows = n, "session transcript_fts backfill complete");
    Ok(())
}
