//! Sparse lane, built to the documented SQLite FTS5 recipe instead of a
//! hand-rolled scan.
//!
//! The recipe (SQLite FTS5 docs, "Trigram tokenizer" + "CJK" guidance):
//!
//! 1. index the **normalized** text, once, at write time — never per query;
//! 2. two tokenizers, routed by script: `trigram` for CJK (which `unicode61`
//!    cannot segment), `unicode61` for the rest;
//! 3. exact substring → the trigram table's **indexed `LIKE`**, which the FTS5
//!    trigram tokenizer answers from the index rather than a table scan;
//! 4. ranked retrieval → `MATCH` + `ORDER BY rank` (BM25, k1=1.2 b=0.75);
//! 5. queries shorter than 3 chars have no trigram and must fall back to `LIKE`;
//! 6. `LIKE` needs a pattern of ≥3 literal chars to be index-served, so the
//!    normalization applied here must be the *same* fold the query gets;
//! 7. proximity → `NEAR(...)`, FTS5's spelling of the span-near tier every
//!    engine has (Lucene `SpanNearQuery`, ES `span_near`);
//! 8. typo tolerance → n-gram overlap (the `trigram` OR), the documented
//!    stand-in for the fuzzy matching FTS5 does not have (`pg_trgm`-style);
//!    it is a **last-resort tier**, only consulted when 3/4/7 found nothing;
//! 9. `|` separates alternatives and any branch may match — the product's query
//!    contract (`lexical::split_patterns`): split, trim, dedupe, search each
//!    branch, keep the best hit per key.
//!
//! Two FTS5 details are load-bearing and easy to get wrong:
//!
//! * the `LIKE` pattern must **not** carry an `ESCAPE` clause — that alone drops
//!   the plan back to a bare virtual-table scan (~18x slower here), so `_` and
//!   `%` stay wildcards and `instr` re-checks the literal substring;
//! * the `MATCH` terms must be quoted, or FTS5 reads punctuation as query
//!   syntax instead of text.
//!
//! # Granularity
//!
//! The index is **chunked**, not per-row. BM25's length normalization is
//! relative to the average document, and one transcript row here can be tens of
//! thousands of characters, so at row granularity the ranker simply prefers long
//! rows: measured 84/121 truth rows in the candidate set against 0 at rank 1.
//! Chunking is what makes the ranker's length prior mean anything.
//!
//! The cut is [`chunk::chunk_text`] in its hard-cut form over
//! `row_plain_text(row).trim()` — the *same* function, parameters and source
//! text the semantic lane uses, so the chunk grid is shared and a row's chunk
//! `k` covers the same char range in both lanes.
//!
//! On top of that grid the sparse lane reports the **literal's own span**, which
//! is finer than the chunk: `LIKE` knows the substring's offset (`instr`) and
//! `MATCH` can mark every matched token (`highlight`, since FTS5 has no
//! `offsets()`). The index therefore stores the original chunk text next to the
//! normalized one, so normalized offsets can be mapped back to original
//! coordinates at query time. This is the lane's whole advantage: a literal hit
//! can point at a line, not a chunk.
//!
//! Two deliberate differences from the semantic corpus, both about *which* rows
//! rather than *how* they are cut:
//!
//! * every searchable row is indexed, not just the prose slots — the sparse lane
//!   is the fallback when the semantic engine is off, and the tool rows are the
//!   longest ones, so excluding them would drop the rows that need chunking most;
//! * no anchor document. The anchor is a head+tail projection whose coordinates
//!   span the whole row; it exists to rescue semantic ranking, and indexing it
//!   here would put non-faithful text in a lane whose whole value is that its
//!   hits are literal.
//!
//! Lanes are kept separate so recall can be attributed to a mechanism rather
//! than to "the sparse lane" as a whole:
//!
//! | lane      | mechanism                                            |
//! |-----------|------------------------------------------------------|
//! | `like`    | indexed `LIKE` — the exact-substring path            |
//! | `trigram` | CJK `MATCH` + BM25                                   |
//! | `unicode` | non-CJK `MATCH` + BM25                               |
//! | `recipe`  | routed `MATCH`, short-CJK `LIKE` fallback            |
//! | `hybrid`  | `like` ∪ `recipe`, exact substring ranked first      |
//! | `near`    | `NEAR(...)` proximity over non-CJK word tokens       |
//! | `final`   | the product ladder: `like` 1.0 > `near` 0.9 >        |
//! |           | routed `MATCH` 0.85 > n-gram fallback 0.72           |

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use crate::session::SessionDataReader;
use crate::session::transcript_file::{SearchableRow, row_plain_text};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tokenizers::Tokenizer;

use super::chunk::{self, ChunkCfg};
use super::echo;

/// Bump when the index layout changes; an old cache is then rebuilt.
/// v4: `rows` stores the original chunk text (for offset mapping) and the
/// compacted summaries are no longer indexed.
/// v5: single-pass token-boundary chunker (cuts shifted by ≤1 token).
/// v6: session-echo result rows dropped (intent-only).
/// v7: product lifecycle — `row_meta` text hashes, FTS5 triggers, `change_id`.
const INDEX_SCHEMA: i64 = 7;

/// Exact-substring hits score like the product's verbatim path, ranked hits
/// land in the product's "index confirmed" band. The split is what lets the
/// board separate "the scan found it" from "BM25 found it".
const SCORE_LIKE: f64 = 1.0;
const SCORE_MATCH: f64 = 0.90;
/// The `final` lane emits the product ladder instead: exact 1.0 > proximity 0.9
/// > BM25 0.85 > n-gram fallback 0.72 (the product's existing fuzzy band).
const SCORE_RANKED: f64 = 0.85;
const SCORE_NEAR: f64 = 0.90;
const SCORE_FALLBACK: f64 = 0.72;

/// `NEAR` window: FTS5's default distance, counted as tokens *between* the two
/// phrases. A whole-sentence query usually exceeds it, which is the point —
/// this is the precision tier; BM25 below it is the recall tier.
const NEAR_DISTANCE: usize = 10;
/// More terms than this would make `NEAR` unsatisfiable; keep the first few
/// (sorted, so the query stays deterministic).
const NEAR_MAX_TERMS: usize = 6;

/// Below this many chars a query has no trigram at all.
const TRIGRAM_LEN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Like,
    Trigram,
    Unicode,
    Recipe,
    Hybrid,
    Near,
    Final,
}

impl Lane {
    pub const ALL: [Lane; 7] = [
        Lane::Like,
        Lane::Trigram,
        Lane::Unicode,
        Lane::Recipe,
        Lane::Hybrid,
        Lane::Near,
        Lane::Final,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Like => "like",
            Lane::Trigram => "trigram",
            Lane::Unicode => "unicode",
            Lane::Recipe => "recipe",
            Lane::Hybrid => "hybrid",
            Lane::Near => "near",
            Lane::Final => "final",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == raw)
    }
}

#[derive(Debug, Clone)]
pub struct SparseHit {
    /// Chunk key: `sid:seq` for a row that fits one chunk, `sid:seq#k` otherwise.
    /// The same key the semantic corpus gives the same span.
    pub key: String,
    /// `sid:seq` of the row this chunk came from. The board scores per *row*.
    pub row_key: String,
    pub session_id: String,
    pub seq: i64,
    /// 0-based chunk index inside the row.
    pub chunk: usize,
    /// Precise span of the literal match inside the row's trimmed plain text,
    /// `[start, end)`. `LIKE` hits carry the exact substring; `MATCH` hits carry
    /// the densest cluster of matched tokens. Finer than the chunk range — this
    /// is what lets the renderer show context around the match.
    pub char_start: usize,
    pub char_end: usize,
    pub score: f64,
    /// The row's item type (`function_call`, `message`, …) — callers label hits
    /// with it.
    pub item_type: String,
    /// Snippet around the match, taken from the chunk text. The renderer prefers
    /// the physical line it resolves from `char_start`; this is the fallback.
    pub summary: String,
    /// Raw `bm25()` for ranked hits; `None` for `LIKE` hits. Negative is a
    /// better match (SQLite returns the negated score). Kept for diagnosis: it
    /// is what explains a poor rank on a corpus of very long rows.
    #[allow(dead_code)]
    pub bm25: Option<f64>,
}

pub struct SparseIndex {
    conn: Connection,
    path: PathBuf,
    /// Optional session scope, applied inside the SQL: a scoped search must not
    /// be truncated by other sessions' hits before the filter runs.
    scope: Option<String>,
}

impl SparseIndex {
    /// Restrict every query to one session.
    pub fn with_scope(mut self, scope: Option<&str>) -> Self {
        self.scope = scope.filter(|s| !s.is_empty()).map(str::to_string);
        self
    }
}

/// The chunking the index is built with: hard cut only, which means no boundary
/// search and no overlap, so the chunks tile the row exactly.
pub fn chunk_cfg(tokens: usize) -> ChunkCfg {
    ChunkCfg {
        tokens,
        // The anchor is a semantic-lane ranking device over non-faithful text;
        // a sparse index must hold literal spans only.
        anchor: false,
    }
}

impl SparseIndex {
    /// Change id the index was last reconciled at (0 = never seen).
    pub fn change_id(&self) -> i64 {
        meta(&self.conn, "change_id")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hits for one lane, best first. `limit` caps the list; the recipe's own
    /// order is preserved (exact substring first, then BM25).
    ///
    /// `|` separates alternatives and any branch may match — the product's
    /// query contract (`lexical::split_patterns`). Each branch is searched and
    /// the best hit per key wins, so "A|B" behaves like the product's union.
    pub fn search(&self, lane: Lane, query: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let alternatives = split_alternatives(query);
        if alternatives.is_empty() {
            return Ok(Vec::new());
        }
        if alternatives.len() == 1 {
            return self.search_one(lane, alternatives[0], limit);
        }
        let mut by_key: BTreeMap<String, SparseHit> = BTreeMap::new();
        for alternative in alternatives {
            for hit in self.search_one(lane, alternative, limit)? {
                match by_key.entry(hit.key.clone()) {
                    std::collections::btree_map::Entry::Occupied(mut slot) => {
                        if hit.score > slot.get().score + 1e-9 {
                            slot.insert(hit);
                        }
                    }
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(hit);
                    }
                }
            }
        }
        let mut out: Vec<SparseHit> = by_key.into_values().collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.session_id.cmp(&b.session_id))
                .then_with(|| a.seq.cmp(&b.seq))
        });
        out.truncate(limit);
        Ok(out)
    }

    fn search_one(&self, lane: Lane, query: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let needle = normalize(query);
        if needle.trim().is_empty() {
            return Ok(Vec::new());
        }
        match lane {
            Lane::Like => self.like_hits(&needle, limit),
            Lane::Trigram => self.trigram_hits(&needle, limit),
            Lane::Unicode => self.unicode_hits(&needle, limit),
            Lane::Recipe => self.recipe_hits(&needle, limit),
            Lane::Hybrid => {
                let mut hits = self.like_hits(&needle, limit)?;
                let seen: BTreeSet<String> = hits.iter().map(|h| h.key.clone()).collect();
                let mut extra = self.recipe_hits(&needle, limit)?;
                extra.retain(|h| !seen.contains(&h.key));
                hits.extend(extra);
                hits.truncate(limit);
                Ok(hits)
            }
            Lane::Near => self.near_hits(&needle, limit),
            Lane::Final => self.final_hits(&needle, limit),
        }
    }

    /// Membership probe: is this exact row in the lane's candidate set?
    ///
    /// Answered by the index with the row pinned, so no `LIMIT` and no ranking
    /// can hide a truth row. This is the lane's *candidate recall* — the
    /// question "could the agent have been given this row at all" — and it is
    /// deliberately separate from the top-k board, which answers "how high did
    /// it rank". Row-level on purpose: any chunk of the row counts.
    pub fn contains(&self, lane: Lane, query: &str, session_id: &str, seq: i64) -> Result<bool> {
        for alternative in split_alternatives(query) {
            if self.contains_one(lane, alternative, session_id, seq)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn contains_one(&self, lane: Lane, query: &str, session_id: &str, seq: i64) -> Result<bool> {
        let needle = normalize(query);
        if needle.trim().is_empty() {
            return Ok(false);
        }
        match lane {
            Lane::Like => self.contains_like(&needle, session_id, seq),
            Lane::Trigram => match trigram_query(&needle) {
                Some(mq) => self.contains_match("tri", &mq, session_id, seq),
                None => Ok(false),
            },
            Lane::Unicode => match word_query(&needle) {
                Some(mq) => self.contains_match("uni", &mq, session_id, seq),
                None => Ok(false),
            },
            Lane::Recipe => self.contains_recipe(&needle, session_id, seq),
            Lane::Hybrid => {
                if self.contains_like(&needle, session_id, seq)? {
                    return Ok(true);
                }
                self.contains_recipe(&needle, session_id, seq)
            }
            Lane::Near => match near_query(&needle) {
                Some(mq) => self.contains_match("uni", &mq, session_id, seq),
                None => Ok(false),
            },
            Lane::Final => {
                if self.contains_like(&needle, session_id, seq)? {
                    return Ok(true);
                }
                if let Some(mq) = near_query(&needle)
                    && self.contains_match("uni", &mq, session_id, seq)?
                {
                    return Ok(true);
                }
                if self.contains_recipe(&needle, session_id, seq)? {
                    return Ok(true);
                }
                match trigram_query(&needle) {
                    Some(mq) => self.contains_match("tri", &mq, session_id, seq),
                    None => Ok(false),
                }
            }
        }
    }

    fn contains_recipe(&self, needle: &str, session_id: &str, seq: i64) -> Result<bool> {
        if has_cjk(needle) {
            if needle.chars().count() < TRIGRAM_LEN {
                return self.contains_like(needle, session_id, seq);
            }
            if let Some(mq) = trigram_query(needle) {
                if self.contains_match("tri", &mq, session_id, seq)? {
                    return Ok(true);
                }
            }
            self.contains_like(needle, session_id, seq)
        } else {
            match word_query(needle) {
                Some(mq) => self.contains_match("uni", &mq, session_id, seq),
                None => Ok(false),
            }
        }
    }

    fn contains_like(&self, needle: &str, session_id: &str, seq: i64) -> Result<bool> {
        if needle.chars().count() < TRIGRAM_LEN {
            return Ok(false);
        }
        let pattern = format!("%{needle}%");
        let hit: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM tri t JOIN rows r ON r.rowid = t.rowid
                  WHERE t.text_norm LIKE ?1
                    AND instr(t.text_norm, ?2) > 0
                    AND r.session_id = ?3 AND r.seq = ?4
                  LIMIT 1",
                params![pattern, needle, session_id, seq],
                |row| row.get(0),
            )
            .optional()?;
        Ok(hit.is_some())
    }

    fn contains_match(
        &self,
        table: &str,
        match_query: &str,
        session_id: &str,
        seq: i64,
    ) -> Result<bool> {
        // `table` is one of two hard-coded literals, never user input.
        let sql = format!(
            "SELECT 1 FROM {table} t JOIN rows r ON r.rowid = t.rowid
              WHERE {table} MATCH ?1 AND r.session_id = ?2 AND r.seq = ?3
              LIMIT 1"
        );
        let hit: Option<i64> = self
            .conn
            .query_row(&sql, params![match_query, session_id, seq], |row| row.get(0))
            .optional()?;
        Ok(hit.is_some())
    }

    /// Indexed `LIKE`. On a trigram FTS5 table this is answered from the index,
    /// not from a scan — the point of the whole exercise.
    ///
    /// The pattern is deliberately **not** escaped. `ESCAPE` costs the trigram
    /// optimization outright (the plan falls back to a bare virtual-table scan,
    /// ~18x slower), so a `_` or `%` in the query stays a LIKE wildcard and the
    /// result is a superset. `instr` then confirms the literal substring, which
    /// keeps the lane's "exact match" claim true without giving up the index.
    fn like_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        // FTS5's LIKE optimization needs ≥3 literal characters in the pattern.
        if needle.chars().count() < TRIGRAM_LEN {
            return Ok(Vec::new());
        }
        // The `ORDER BY` is not cosmetic: a `LIKE` probe with a `LIMIT` and no
        // order truncates in rowid order, which is arbitrary and silently loses
        // the newest rows. Session ids are ULIDs, so descending `(session_id,
        // seq, chunk)` is newest-first and stable.
        //
        // `instr` is asked for the same literal alongside the LIKE, so the hit
        // carries where the substring landed (normalized chars; `r.text` maps it
        // back to the original row coordinates).
        let scope_clause = if self.scope.is_some() {
            " AND r.session_id = ?4"
        } else {
            ""
        };
        let sql = format!(
            "SELECT r.session_id, r.seq, r.chunk, r.text, r.single, r.item_type,
                    instr(t.text_norm, ?2) AS pos, r.char_start AS chunk_start
               FROM tri t
               JOIN rows r ON r.rowid = t.rowid
              WHERE t.text_norm LIKE ?1
                AND instr(t.text_norm, ?2) > 0{scope_clause}
              ORDER BY r.session_id DESC, r.seq DESC, r.chunk DESC
              LIMIT ?3"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let pattern = format!("%{needle}%");
        let limit_i64 = limit as i64;
        let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&pattern, &needle, &limit_i64];
        if let Some(scope) = &self.scope {
            bind.push(scope);
        }
        let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
            let text: String = row.get(3)?;
            let pos: i64 = row.get(6)?;
            let chunk_start: i64 = row.get(7)?;
            let span = like_match_span(&text, needle, pos.max(1) as usize);
            row_hit(
                row,
                &text,
                span,
                SCORE_LIKE,
                None,
                chunk_start.max(0) as usize,
            )
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// CJK path: trigram `MATCH` over the whole normalized query, BM25 order.
    fn trigram_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        self.trigram_hits_scored(needle, limit, SCORE_MATCH)
    }

    fn trigram_hits_scored(&self, needle: &str, limit: usize, score: f64) -> Result<Vec<SparseHit>> {
        let Some(match_query) = trigram_query(needle) else {
            return Ok(Vec::new());
        };
        self.match_hits("tri", &match_query, limit, score)
    }

    /// Non-CJK path: `unicode61` `MATCH` over the query's word tokens.
    fn unicode_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        self.unicode_hits_scored(needle, limit, SCORE_MATCH)
    }

    fn unicode_hits_scored(&self, needle: &str, limit: usize, score: f64) -> Result<Vec<SparseHit>> {
        let Some(match_query) = word_query(needle) else {
            return Ok(Vec::new());
        };
        self.match_hits("uni", &match_query, limit, score)
    }

    /// Proximity tier: FTS5's `NEAR(...)` over the query's word tokens.
    fn near_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let Some(match_query) = near_query(needle) else {
            return Ok(Vec::new());
        };
        self.match_hits("uni", &match_query, limit, SCORE_NEAR)
    }

    /// The routed recipe: script decides the tokenizer, and a query too short
    /// for a trigram falls back to `LIKE` (documented trigram limitation).
    fn recipe_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        self.routed_hits(needle, limit, SCORE_MATCH)
    }

    /// Routed `MATCH` with the caller's score, so the `final` ladder can place
    /// the BM25 tier below the proximity tier.
    fn routed_hits(&self, needle: &str, limit: usize, score: f64) -> Result<Vec<SparseHit>> {
        if has_cjk(needle) {
            if needle.chars().count() < TRIGRAM_LEN {
                return self.like_hits(needle, limit);
            }
            let mut hits = self.trigram_hits_scored(needle, limit, score)?;
            if hits.is_empty() {
                hits = self.like_hits(needle, limit)?;
            }
            Ok(hits)
        } else {
            self.unicode_hits_scored(needle, limit, score)
        }
    }

    /// The product ladder: exact substring 1.0 > proximity 0.9 > BM25 0.85 >
    /// n-gram fallback 0.72. The fallback is only consulted when every tier
    /// above found nothing — typo tolerance as a last resort, not a peer lane.
    fn final_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let mut hits = self.like_hits(needle, limit)?;
        let mut seen: BTreeSet<String> = hits.iter().map(|h| h.key.clone()).collect();
        extend_unique(&mut hits, self.near_hits(needle, limit)?, &mut seen);
        extend_unique(
            &mut hits,
            self.routed_hits(needle, limit, SCORE_RANKED)?,
            &mut seen,
        );
        if hits.is_empty() {
            extend_unique(
                &mut hits,
                self.trigram_hits_scored(needle, limit, SCORE_FALLBACK)?,
                &mut seen,
            );
        }
        hits.truncate(limit);
        Ok(hits)
    }

    fn match_hits(
        &self,
        table: &str,
        match_query: &str,
        limit: usize,
        score: f64,
    ) -> Result<Vec<SparseHit>> {
        // `table` is one of two hard-coded literals, never user input.
        //
        // `highlight()` wraps every matched token in the stored (normalized)
        // text; the hit turns the densest cluster of those marks into one span
        // in original row coordinates. (FTS5 has no `offsets()`.)
        let scope_clause = if self.scope.is_some() {
            " AND r.session_id = ?3"
        } else {
            ""
        };
        let sql = format!(
            "SELECT r.session_id, r.seq, r.chunk, r.text, r.single, r.item_type,
                    highlight({table}, 0, char(2), char(3)) AS hl,
                    bm25({table}) AS rank,
                    r.char_start AS chunk_start
               FROM {table} t
               JOIN rows r ON r.rowid = t.rowid
              WHERE {table} MATCH ?1{scope_clause}
              ORDER BY rank
              LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let limit_i64 = limit as i64;
        let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&match_query, &limit_i64];
        if let Some(scope) = &self.scope {
            bind.push(scope);
        }
        let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
            let text: String = row.get(3)?;
            let marked: Option<String> = row.get(6)?;
            let bm25 = row.get(7).ok();
            let chunk_start: i64 = row.get(8)?;
            let span = highlight_span(&text, marked.as_deref().unwrap_or(""));
            row_hit(
                row,
                &text,
                span,
                score,
                bm25,
                chunk_start.max(0) as usize,
            )
        })?;
        // Keep BM25's order *inside* the tier: the agent-facing sort is score
        // desc and only breaks ties by recency, so a flat tier score would
        // silently replace the ranker with "newest session first".
        let mut out = Vec::new();
        for (rank, hit) in rows.enumerate() {
            let mut hit = hit?;
            hit.score = tier_score(score, rank, limit);
            out.push(hit);
        }
        Ok(out)
    }
}

/// Build one hit from the shared column order (`session_id, seq, chunk, text,
/// single, item_type` at 0..=5; the span comes from the mechanism the caller
/// ran). `single` is 1 when the row was not split, in which case the key is the
/// bare row key — the same rule the semantic corpus uses, so a chunk key means
/// the same thing in both lanes.
fn row_hit(
    row: &rusqlite::Row<'_>,
    text: &str,
    span: (usize, usize),
    score: f64,
    bm25: Option<f64>,
    chunk_start: usize,
) -> rusqlite::Result<SparseHit> {
    let session_id: String = row.get(0)?;
    let seq: i64 = row.get(1)?;
    let chunk: i64 = row.get(2)?;
    let single: i64 = row.get(4)?;
    let item_type: String = row.get(5)?;
    let row_key = format!("{session_id}:{seq}");
    let key = if single == 1 {
        row_key.clone()
    } else {
        format!("{row_key}#{chunk}")
    };
    // The span is in row coordinates; the snippet comes from the chunk text, so
    // shift it into the chunk first.
    let local_start = span.0.saturating_sub(chunk_start);
    let local_end = span.1.saturating_sub(chunk_start);
    let summary = super::snippet_from_span(text, local_start, local_end);
    Ok(SparseHit {
        key,
        row_key,
        session_id,
        seq,
        chunk: chunk as usize,
        char_start: span.0,
        char_end: span.1,
        score,
        item_type,
        summary,
        bm25,
    })
}

/// Append hits whose chunk key was not seen yet — the first tier that found a
/// chunk owns it, so a lower tier can never re-rank a higher tier's hit.
fn extend_unique(hits: &mut Vec<SparseHit>, extra: Vec<SparseHit>, seen: &mut BTreeSet<String>) {
    for h in extra {
        if seen.insert(h.key.clone()) {
            hits.push(h);
        }
    }
}

/// Split a query on `|` into trimmed, non-empty, deduped alternatives — the
/// product's contract (`lexical::split_patterns`), mirrored exactly so the lane
/// answers "A|B" the way production does.
fn split_alternatives(query: &str) -> Vec<&str> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();
    for part in query.split('|') {
        let part = part.trim();
        if !part.is_empty() && seen.insert(part) {
            out.push(part);
        }
    }
    out
}

/// Score for the `rank`-th BM25 hit of a tier: a 2%-of-band decay that keeps
/// the tier strictly below the one above and above the one below, while
/// leaving the ranker's order intact through the product's score sort.
/// (Exact substring hits keep their flat 1.0: among equals, recency is the
/// product's intended tie-break.)
fn tier_score(base: f64, rank: usize, limit: usize) -> f64 {
    base - 0.02 * (rank as f64 / limit.max(1) as f64)
}

fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open sparse index {}", path.display()))?;
    // The refresh path writes while searches read; WAL + a busy timeout keeps
    // them from tripping over each other. (`execute_batch` because the
    // journal_mode pragma returns a row, which `pragma_update` rejects.)
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(conn)
}

/// A read-only handle for one search. Takes a path, not a `&SparseIndex`, so a
/// connection is never shared across threads.
pub fn open_read_only(path: &Path) -> Result<SparseIndex> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open sparse index read-only {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(SparseIndex {
        conn,
        path: path.to_path_buf(),
        scope: None,
    })
}

fn meta(conn: &Connection, key: &str) -> Result<String> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
        r.get(0)
    })?)
}

/// Rows-table DDL, shared by the index builder and its tests. `text` keeps the
/// original chunk text so normalized offsets can be mapped back at query time;
/// `row_meta` keeps one text hash per row, which is the reconcile's change
/// detector (an edited row is re-chunked without comparing chunk text).
const ROWS_DDL: &str = "CREATE TABLE IF NOT EXISTS rows (
        rowid      INTEGER PRIMARY KEY,
        session_id TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        chunk      INTEGER NOT NULL DEFAULT 0,
        single     INTEGER NOT NULL DEFAULT 1,
        item_type  TEXT NOT NULL DEFAULT '',
        char_start INTEGER NOT NULL DEFAULT 0,
        char_end   INTEGER NOT NULL DEFAULT 0,
        text_norm  TEXT NOT NULL,
        text       TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS rows_key ON rows(session_id, seq);
     CREATE TABLE IF NOT EXISTS row_meta (
        session_id TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        hash       INTEGER NOT NULL,
        PRIMARY KEY (session_id, seq)
     );";

/// FTS5 tables over `rows`. No bulk `rebuild` here: the triggers below keep
/// them in sync on every insert/delete, which is what makes appends cheap.
const FTS_DDL: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS tri USING fts5(
            text_norm,
            content='rows',
            content_rowid='rowid',
            tokenize = 'trigram'
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS uni USING fts5(
            text_norm,
            content='rows',
            content_rowid='rowid',
            tokenize = 'unicode61 remove_diacritics 2'
         );";

/// External-content FTS5: inserts mirror the new row, deletes use the FTS5
/// `delete` command with the old values.
const TRIGGER_DDL: &str = "CREATE TRIGGER IF NOT EXISTS rows_ai AFTER INSERT ON rows BEGIN
            INSERT INTO tri(rowid, text_norm) VALUES (new.rowid, new.text_norm);
            INSERT INTO uni(rowid, text_norm) VALUES (new.rowid, new.text_norm);
         END;
         CREATE TRIGGER IF NOT EXISTS rows_ad AFTER DELETE ON rows BEGIN
            INSERT INTO tri(tri, rowid, text_norm) VALUES ('delete', old.rowid, old.text_norm);
            INSERT INTO uni(uni, rowid, text_norm) VALUES ('delete', old.rowid, old.text_norm);
         END;";

/// The chunk budget the index is built with.
const CHUNK_TOKENS: usize = 448;

/// Index location under the workspace data root (`.litecode/session-index/`,
/// next to the semantic index).
pub fn sparse_index_path(data_root: &Path) -> PathBuf {
    data_root.join("session-index").join("sparse.db")
}

/// True when the file is missing or was written by another schema version.
pub fn needs_rebuild(path: &Path) -> Result<bool> {
    if !path.is_file() {
        return Ok(true);
    }
    let conn = open(path)?;
    let schema_ok = meta(&conn, "schema")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        == Some(INDEX_SCHEMA);
    Ok(!schema_ok)
}

/// Cheap freshness probe: has the session store moved past the index's change id?
pub fn is_stale(reader: &SessionDataReader, path: &Path) -> Result<bool> {
    let conn = open(path)?;
    let indexed = meta(&conn, "change_id")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(-1);
    Ok(indexed != reader.latest_change_id_blocking().unwrap_or(0))
}

/// Full build: recreate the file and index every searchable row.
pub fn build_index(rows: &[SearchableRow], data_root: &Path, change_id: i64) -> Result<()> {
    let path = sparse_index_path(data_root);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("reset {}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create sparse index dir {}", parent.display()))?;
    }
    let tk = super::tokenizer::open()?;
    let cfg = chunk_cfg(CHUNK_TOKENS);
    let echo_keys = echo::result_keys(rows, data_root)?;
    let mut conn = open(&path)?;
    conn.execute_batch("PRAGMA synchronous = OFF;")?;
    ensure_schema(&conn)?;
    let tx = conn.transaction()?;
    let mut indexed = 0usize;
    for row in rows {
        if !indexable(row, &echo_keys) {
            continue;
        }
        let Some(raw) = row_plain_text(row, data_root)? else {
            continue;
        };
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        insert_row_chunks(&tx, &tk, &cfg, &row.session_id, row.seq, &row.item_type, text)?;
        tx.execute(
            "INSERT OR REPLACE INTO row_meta(session_id, seq, hash) VALUES (?1, ?2, ?3)",
            params![row.session_id, row.seq, text_hash(text)],
        )?;
        indexed += 1;
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema', ?1)",
        params![INDEX_SCHEMA.to_string()],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('change_id', ?1)",
        params![change_id.to_string()],
    )?;
    tx.commit()?;
    tracing::info!(rows = indexed, "sparse session index built");
    Ok(())
}

/// Incremental refresh: diff the index against the live rows and touch only the
/// delta (new rows, edited rows, removed rows). Returns how many rows changed.
pub fn refresh_index(
    reader: &SessionDataReader,
    rows: &[SearchableRow],
    data_root: &Path,
) -> Result<usize> {
    let path = sparse_index_path(data_root);
    let tk = super::tokenizer::open()?;
    let cfg = chunk_cfg(CHUNK_TOKENS);
    let echo_keys = echo::result_keys(rows, data_root)?;
    let mut conn = open(&path)?;
    let indexed: HashMap<(String, i64), i64> = {
        let mut stmt = conn.prepare("SELECT session_id, seq, hash FROM row_meta")?;
        let it = stmt.query_map([], |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, i64>(1)?),
                r.get::<_, i64>(2)?,
            ))
        })?;
        it.collect::<rusqlite::Result<HashMap<_, _>>>()?
    };
    let mut live: Vec<(String, i64, String, i64, String)> = Vec::new();
    for row in rows {
        if !indexable(row, &echo_keys) {
            continue;
        }
        let Some(raw) = row_plain_text(row, data_root)? else {
            continue;
        };
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        live.push((
            row.session_id.clone(),
            row.seq,
            text.to_string(),
            text_hash(text),
            row.item_type.clone(),
        ));
    }

    let tx = conn.transaction()?;
    let mut changed = 0usize;
    let mut live_keys: HashSet<(String, i64)> = HashSet::new();
    for (sid, seq, text, hash, item_type) in &live {
        live_keys.insert((sid.clone(), *seq));
        if indexed.get(&(sid.clone(), *seq)) == Some(hash) {
            continue;
        }
        delete_row_chunks(&tx, sid, *seq)?;
        insert_row_chunks(&tx, &tk, &cfg, sid, *seq, item_type, text)?;
        tx.execute(
            "INSERT OR REPLACE INTO row_meta(session_id, seq, hash) VALUES (?1, ?2, ?3)",
            params![sid, seq, hash],
        )?;
        changed += 1;
    }
    for ((sid, seq), _) in &indexed {
        if live_keys.contains(&(sid.clone(), *seq)) {
            continue;
        }
        delete_row_chunks(&tx, sid, *seq)?;
        tx.execute(
            "DELETE FROM row_meta WHERE session_id = ?1 AND seq = ?2",
            params![sid, seq],
        )?;
        changed += 1;
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('change_id', ?1)",
        params![reader.latest_change_id_blocking().unwrap_or(0).to_string()],
    )?;
    tx.commit()?;
    Ok(changed)
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         {ROWS_DDL}
         {FTS_DDL}
         {TRIGGER_DDL}"
    ))?;
    Ok(())
}

/// A row is indexed unless it is a compacted summary or a session-echo result.
fn indexable(row: &SearchableRow, echo_keys: &HashSet<(String, i64)>) -> bool {
    row.kind != "compacted" && !echo_keys.contains(&(row.session_id.clone(), row.seq))
}

fn insert_row_chunks(
    conn: &Connection,
    tk: &Tokenizer,
    cfg: &ChunkCfg,
    session_id: &str,
    seq: i64,
    item_type: &str,
    text: &str,
) -> Result<usize> {
    let pieces = chunk::chunk_text(tk, text, cfg);
    if pieces.is_empty() {
        return Ok(0);
    }
    let total = pieces.len();
    let mut ins = conn.prepare_cached(
        "INSERT INTO rows(session_id, seq, chunk, single, item_type, char_start,
                          char_end, text_norm, text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for c in pieces {
        ins.execute(params![
            session_id,
            seq,
            c.index as i64,
            i64::from(total == 1),
            item_type,
            c.start as i64,
            c.end as i64,
            normalize(&c.text),
            c.text,
        ])?;
    }
    Ok(total)
}

fn delete_row_chunks(conn: &Connection, session_id: &str, seq: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM rows WHERE session_id = ?1 AND seq = ?2",
        params![session_id, seq],
    )?;
    Ok(())
}

/// One text hash per row: the reconcile's change detector.
fn text_hash(text: &str) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as i64
}

/// Normalized text plus the map back to original char indices.
pub struct NormText {
    pub norm: String,
    /// `map[i]` = original char index that produced normalized char `i`.
    pub map: Vec<usize>,
    /// Char count of the original text.
    pub orig_chars: usize,
}

/// Same fold the product's verbatim matcher applies (`normalize_for_match`):
/// per-char lowercase, fullwidth → halfwidth, and whitespace runs collapsed to a
/// single space. Applied at index time so the query side never rebuilds it per
/// row.
pub fn normalize(text: &str) -> String {
    normalize_with_map(text).norm
}

/// [`normalize`] plus the normalized→original char map. The sparse index stores
/// the original chunk text exactly so a normalized offset found by `LIKE` /
/// `offsets()` can be turned back into a position in the row.
pub fn normalize_with_map(text: &str) -> NormText {
    let mut norm = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len());
    let mut orig_chars = 0usize;
    let mut prev_space = false;
    for raw in text.chars() {
        let idx = orig_chars;
        orig_chars += 1;
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
            norm.push(' ');
            map.push(idx);
            continue;
        }
        prev_space = false;
        for lower in ch.to_lowercase() {
            norm.push(lower);
            map.push(idx);
        }
    }
    NormText {
        norm,
        map,
        orig_chars,
    }
}

/// Map a normalized char range `[nstart, nend)` back to original chars.
fn map_span(n: &NormText, nstart: usize, nend: usize) -> (usize, usize) {
    let Some(&start) = n.map.get(nstart) else {
        return (n.orig_chars, n.orig_chars);
    };
    let end = n
        .map
        .get(nend.saturating_sub(1))
        .map(|&i| i + 1)
        .unwrap_or(start + 1);
    (start, end.max(start + 1))
}

/// The exact substring span of a `LIKE` hit. `pos` is `instr`'s 1-based char
/// offset inside the normalized text.
fn like_match_span(text: &str, needle: &str, pos: usize) -> (usize, usize) {
    let n = normalize_with_map(text);
    let start = pos.saturating_sub(1);
    map_span(&n, start, start + needle.chars().count())
}

/// Window used to pick the densest cluster of matched tokens, in original chars.
const CLUSTER_WINDOW: usize = 300;

/// Densest cluster of match spans: the window with the most matches wins, and
/// the span is its `min..max`.
fn cluster_spans(mut spans: Vec<(usize, usize)>) -> Option<(usize, usize)> {
    spans.sort_unstable();
    let mut best: Option<(usize, usize, usize)> = None; // (count, lo, hi)
    let mut lo = 0usize;
    for hi in 0..spans.len() {
        while spans[hi].0.saturating_sub(spans[lo].0) > CLUSTER_WINDOW {
            lo += 1;
        }
        let count = hi - lo + 1;
        if best.map(|(c, _, _)| count > c).unwrap_or(true) {
            best = Some((count, lo, hi));
        }
    }
    let (_, lo, hi) = best?;
    let start = spans[lo..=hi].iter().map(|s| s.0).min()?;
    let end = spans[lo..=hi].iter().map(|s| s.1).max()?;
    Some((start, end.max(start + 1)))
}

/// The densest cluster of `highlight()` marks, in original coordinates.
///
/// `marked` is the stored normalized text with `\u{2}`/`\u{3}` wrapped around
/// every matched token; the marks are dropped while counting chars, so the
/// resulting offsets are normalized char indices and map back through
/// [`normalize_with_map`].
fn highlight_span(text: &str, marked: &str) -> (usize, usize) {
    const OPEN: char = '\u{2}';
    const CLOSE: char = '\u{3}';
    let n = normalize_with_map(text);
    debug_assert_eq!(
        marked
            .chars()
            .filter(|c| *c != OPEN && *c != CLOSE)
            .count(),
        n.norm.chars().count(),
        "highlight() output must be normalize(text) with marks"
    );
    let mut spans = Vec::new();
    let mut pos = 0usize;
    let mut start: Option<usize> = None;
    for ch in marked.chars() {
        match ch {
            OPEN => {
                if start.is_none() {
                    start = Some(pos);
                }
            }
            CLOSE => {
                if let Some(s) = start.take()
                    && pos > s
                {
                    spans.push(map_span(&n, s, pos));
                }
            }
            _ => pos += 1,
        }
    }
    cluster_spans(spans).unwrap_or((0, 0))
}

/// CJK ranges that `unicode61` cannot segment into words. Han, Hiragana,
/// Katakana, Hangul syllables, and the CJK compatibility ideographs.
pub fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32,
            0x3000..=0x303F   // CJK symbols and punctuation
            | 0x3040..=0x30FF // Hiragana + Katakana
            | 0x3400..=0x4DBF // CJK ext A
            | 0x4E00..=0x9FFF // CJK unified
            | 0xAC00..=0xD7AF // Hangul syllables
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFF00..=0xFFEF // halfwidth/fullwidth forms
        )
    })
}

/// Trigram `MATCH`: the query's 3-char n-grams, OR-ed and quoted. FTS5's
/// trigram tokenizer ANDs the terms of a bare query, so an OR is what expresses
/// "any of these grams"; `ORDER BY rank` then sorts by BM25.
///
/// OR is the intended form here, not a fallback: an AND over the grams is just
/// exact substring search, which `like` already does and which reaches 51/121
/// against this lane's 84/121.
pub fn trigram_query(needle: &str) -> Option<String> {
    let chars: Vec<char> = needle.chars().collect();
    if chars.len() < TRIGRAM_LEN {
        return None;
    }
    let mut grams: BTreeSet<String> = BTreeSet::new();
    for w in chars.windows(TRIGRAM_LEN) {
        let gram: String = w.iter().collect();
        if gram.trim().is_empty() {
            continue;
        }
        grams.insert(quote_term(&gram));
    }
    if grams.is_empty() {
        return None;
    }
    Some(grams.into_iter().collect::<Vec<_>>().join(" OR "))
}

/// `unicode61` word tokens of the query, deduped and sorted so the generated
/// query is deterministic. The tokenizer already drops punctuation and folds
/// case, so splitting on non-alphanumerics reproduces its segmentation.
fn word_terms(needle: &str) -> Vec<String> {
    let mut tokens: BTreeSet<String> = BTreeSet::new();
    for tok in needle.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !tok.is_empty() {
            tokens.insert(tok.to_string());
        }
    }
    tokens.into_iter().collect()
}

/// A phrase for an FTS5 query: quoted so punctuation reads as text, with
/// embedded quotes doubled (FTS5's own escape).
fn quote_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// `unicode61` `MATCH`: the query's word tokens, OR-ed.
pub fn word_query(needle: &str) -> Option<String> {
    let terms = word_terms(needle);
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|t| quote_term(t))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// FTS5's documented proximity query: all query terms within `NEAR_DISTANCE`
/// tokens of each other, in any order. This is the standard span-near tier
/// (Lucene `SpanNearQuery`, ES `span_near`), not a local invention.
///
/// Skipped for CJK — the trigram path already matches substrings, and `NEAR`
/// over trigram tokens is not meaningful — and for single-term queries, which
/// have nothing to be near.
fn near_query(needle: &str) -> Option<String> {
    if has_cjk(needle) {
        return None;
    }
    let terms = word_terms(needle);
    if terms.len() < 2 {
        return None;
    }
    let list = terms
        .iter()
        .take(NEAR_MAX_TERMS)
        .map(|t| quote_term(t))
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!("NEAR({list}, {NEAR_DISTANCE})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_fullwidth_and_space() {
        assert_eq!(normalize("Ａ B\t\tC"), "a b c");
        assert_eq!(normalize("你好   世界"), "你好 世界");
    }

    #[test]
    fn normalize_with_map_points_back_at_originals() {
        let text = "Ａ B\t\tC 中";
        let n = normalize_with_map(text);
        assert_eq!(n.norm, "a b c 中");
        let orig: Vec<char> = text.chars().collect();
        let got: String = n.map.iter().map(|&i| orig[i]).collect();
        assert_eq!(got, "Ａ B\tC 中");
    }

    #[test]
    fn map_span_lands_on_the_original_literal() {
        let text = "前缀 ＡBC 后缀";
        let n = normalize_with_map(text);
        let needle = normalize("abc");
        let chars: Vec<char> = n.norm.chars().collect();
        let width = needle.chars().count();
        let pos = chars
            .windows(width)
            .position(|w| w.iter().collect::<String>() == needle)
            .expect("normalized literal");
        let (s, e) = map_span(&n, pos, pos + width);
        let got: String = text.chars().skip(s).take(e - s).collect();
        assert_eq!(got, "ＡBC");
    }

    #[test]
    fn like_and_match_spans_land_on_the_literal() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        let text = "开头 plain filler 重试三次 ＡBC 结尾";
        conn.execute(
            "INSERT INTO rows(session_id, seq, chunk, single, char_start, char_end,
                              text_norm, text)
             VALUES ('s', 1, 0, 1, 0, 0, ?1, ?2)",
            params![normalize(text), text],
        )
        .unwrap();
        conn.execute_batch(FTS_DDL).unwrap();
        conn.execute_batch(
            "INSERT INTO tri(tri) VALUES('rebuild'); INSERT INTO uni(uni) VALUES('rebuild');",
        )
        .unwrap();

        // LIKE: `instr` reaches the substring; the span maps back to `ＡBC`.
        let needle = normalize("ＡBC");
        let pos: i64 = conn
            .query_row(
                "SELECT instr(text_norm, ?1) FROM tri
                  WHERE text_norm LIKE ?2 AND instr(text_norm, ?1) > 0",
                params![needle, format!("%{needle}%")],
                |r| r.get(0),
            )
            .unwrap();
        let (s, e) = like_match_span(text, &needle, pos as usize);
        let got: String = text.chars().skip(s).take(e - s).collect();
        assert_eq!(got, "ＡBC");

        // MATCH: `highlight` marks cluster onto the CJK literal.
        let query = trigram_query(&normalize("重试三次")).unwrap();
        let marked: String = conn
            .query_row(
                "SELECT highlight(tri, 0, char(2), char(3)) FROM tri WHERE tri MATCH ?1",
                params![query],
                |r| r.get(0),
            )
            .unwrap();
        let (ms, me) = highlight_span(text, &marked);
        let got: String = text.chars().skip(ms).take(me - ms).collect();
        assert!(got.contains("重试三次"), "cluster landed on {got:?}");
    }

    #[test]
    fn cjk_detection() {
        assert!(has_cjk("会话检索"));
        assert!(has_cjk("abc 世界"));
        assert!(!has_cjk("session search"));
    }

    #[test]
    fn trigram_query_needs_three_chars() {
        assert!(trigram_query("ab").is_none());
        assert_eq!(trigram_query("abc").unwrap(), "\"abc\"");
        assert_eq!(trigram_query("abcd").unwrap(), "\"abc\" OR \"bcd\"");
    }

    #[test]
    fn word_query_drops_punctuation() {
        assert_eq!(word_query("hello, world!").unwrap(), "\"hello\" OR \"world\"");
        assert!(word_query("!!!").is_none());
    }

    #[test]
    fn tier_score_keeps_tiers_apart_and_order_intact() {
        assert_eq!(tier_score(0.85, 0, 50), 0.85);
        assert!(tier_score(0.85, 0, 50) > tier_score(0.85, 1, 50));
        // Bands never overlap: near > ranked > fallback, at any rank.
        assert!(tier_score(0.90, 49, 50) > tier_score(0.85, 0, 50));
        assert!(tier_score(0.85, 49, 50) > SCORE_FALLBACK);
        assert!(tier_score(0.72, 49, 50) < SCORE_FALLBACK);
    }

    #[test]
    fn alternatives_split_like_the_product() {
        assert_eq!(split_alternatives("a|b"), vec!["a", "b"]);
        assert_eq!(split_alternatives(" a | b | a |"), vec!["a", "b"]);
        assert!(split_alternatives("|").is_empty());
        assert_eq!(split_alternatives("only"), vec!["only"]);
    }

    #[test]
    fn alternatives_match_any_branch() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        conn.execute(
            "INSERT INTO rows(session_id, seq, chunk, single, char_start, char_end,
                              text_norm, text)
             VALUES ('s', 1, 0, 1, 0, 0, ?1, ?2)",
            params![normalize("alpha beta gamma"), "alpha beta gamma"],
        )
        .unwrap();
        conn.execute_batch(FTS_DDL).unwrap();
        conn.execute_batch(
            "INSERT INTO tri(tri) VALUES('rebuild'); INSERT INTO uni(uni) VALUES('rebuild');",
        )
        .unwrap();
        let index = SparseIndex {
            conn,
            path: PathBuf::from(":memory:"),
            scope: None,
        };
        // The second branch matches: the first must not mask it.
        let hits = index.search(Lane::Like, "zzz_nope|beta", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].seq, 1);
        assert!(index.contains(Lane::Like, "zzz_nope|beta", "s", 1).unwrap());
        assert!(!index.contains(Lane::Like, "zzz_nope|nope", "s", 1).unwrap());
        // Both branches hit the same row: one hit survives (best score).
        let both = index.search(Lane::Like, "alpha|beta", 10).unwrap();
        assert_eq!(both.len(), 1, "{both:?}");
    }

    #[test]
    fn near_query_is_proximity_over_word_terms() {
        assert_eq!(
            near_query("session search").unwrap(),
            "NEAR(\"search\" \"session\", 10)"
        );
        assert!(near_query("single").is_none());
        assert!(near_query("会话检索").is_none(), "CJK takes the trigram path");
    }

    #[test]
    fn near_parses_and_matches_in_the_bundled_sqlite() {
        // Proves the bundled FTS5 supports `NEAR` and pins the distance
        // semantics the lane relies on: the number counts the tokens *between*
        // the phrases, so `alpha`…`delta` (two in between) needs ≥2.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        let text = "alpha beta gamma delta epsilon";
        conn.execute(
            "INSERT INTO rows(session_id, seq, chunk, single, char_start, char_end,
                              text_norm, text)
             VALUES ('s', 1, 0, 1, 0, 0, ?1, ?2)",
            params![normalize(text), text],
        )
        .unwrap();
        conn.execute_batch(FTS_DDL).unwrap();
        conn.execute_batch(
            "INSERT INTO tri(tri) VALUES('rebuild'); INSERT INTO uni(uni) VALUES('rebuild');",
        )
        .unwrap();
        let count = |q: &str| -> i64 {
            conn.query_row(
                "SELECT count(*) FROM uni WHERE uni MATCH ?1",
                params![q],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count(&near_query("alpha delta").unwrap()), 1);
        assert_eq!(count("NEAR(\"alpha\" \"delta\", 1)"), 0, "gap is two tokens");
        assert_eq!(count(&near_query("epsilon alpha").unwrap()), 1, "order-free");
    }

    #[test]
    fn hard_cut_is_the_only_cut() {
        let cfg = chunk_cfg(448);
        assert_eq!(cfg.tokens, 448);
        assert!(!cfg.anchor);
    }
}
