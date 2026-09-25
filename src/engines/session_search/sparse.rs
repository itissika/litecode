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
//! 5. queries shorter than 3 chars have no trigram, so the index cannot serve
//!    them and the rows are scanned directly instead;
//! 6. `LIKE` needs a pattern of ≥3 literal chars to be index-served, so the
//!    normalization applied here must be the *same* fold the query gets;
//! 7. proximity → `NEAR(...)`, FTS5's spelling of the span-near tier every
//!    engine has (Lucene `SpanNearQuery`, ES `span_near`);
//! 8. typo tolerance → n-gram overlap (the `trigram` OR), the documented
//!    stand-in for the fuzzy matching FTS5 does not have (`pg_trgm`-style);
//!    it is a **last-resort tier**, only consulted when 3/4/7 found nothing;
//! 9. `|` separates alternatives and any branch may match — the contract the
//!    `session_search` tool documents to the agent (`split_alternatives`):
//!    split, trim, dedupe, search each branch, keep the best hit per key.
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
//! |-----------|----------------------------------------------------|
//! | `like`    | indexed `LIKE` — the exact-substring path            |
//! | `trigram` | CJK `MATCH` + BM25                                   |
//! | `unicode` | non-CJK `MATCH` + BM25                               |
//! | `recipe`  | routed `MATCH`, short-CJK `LIKE` fallback            |
//! | `hybrid`  | `like` ∪ `recipe`, exact substring ranked first      |
//! | `near`    | `NEAR(...)` proximity over non-CJK word tokens       |
//! | `final`   | the product layer: the four declared leaf layers,    |
//! |           | coverage-gated, merged and ranked explicitly         |
//!
//! The leaf lanes are mechanisms; `final` is the *policy*. It enters the same
//! `like` → `near` → routed `MATCH` → n-gram mechanisms, but as declared layers
//! ([`ranking::LAYERS`]) with an explicit goal, priority and gate, then merges
//! every layer's evidence, folds chunks into rows and ranks on an explicit key.
//! Two properties the old score ladder did not have:
//!
//! * a layer that ran and then lost its hits to a truncation cannot happen any
//!   more: every layer's output is merged *before* anything is truncated, and a
//!   layer that was skipped records why ([`ranking::StopReason`]);
//! * `score` is a presentation value derived from the final rank, not the thing
//!   the order was computed from.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use crate::session::SessionDataReader;
use crate::session::transcript_file::SearchableRow;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};


use super::chunk::ChunkCfg;
use super::derive;
use super::echo;
use super::query_plan::{self, QueryBranch};
use super::ranking::{self, ContentRole, HitEvidence, LayerId, LayerTrace, RankKey, StopReason};
use super::trimmer;

/// Bump when the index layout changes; an old cache is then rebuilt.
/// v4: `rows` stores the original chunk text (for offset mapping) and the
/// compacted summaries are no longer indexed.
/// v5: single-pass token-boundary chunker (cuts shifted by ≤1 token).
/// v6: session-echo result rows dropped (intent-only).
/// v7: product lifecycle — `row_meta` text hashes, FTS5 triggers, `change_id`.
/// v8: single derive pipeline — `source_state` (kind/item_type/hash/call_id/
/// session_read_call/in_chunks) replaces the hash-only `row_meta`.
/// v9: settled-prefix projection — `source_state` loses its content hash and
/// indexes `(session_id, call_id)`. A row is identified by `(session_id, seq)`
/// alone, and a settled row is never rewritten in place, so there is nothing
/// left for a hash to notice. Old files are rebuilt rather than migrated.
/// v10: pure key-set reconciliation — no outbox/`change_id` freshness, no
/// forced keys, no content window. Every search reconciles the current final
/// source key set against the indexed key ledger. Old files are rebuilt rather
/// than migrated.
/// v11: every indexed chunk carries the row's retrieval `role` (人话/动作/产出),
/// so ranking can prefer intent over output without re-reading the source.
/// v12: the role separates the assistant's own reasoning from what was said, so
/// the four tiers the product orders by (said → thought → call → result) are in
/// the index instead of being reconstructed at query time.`v11`'s three
/// groups merged the first two, so old files are rebuilt rather than migrated.
/// v13: an indexed result keeps only its head (see `trimmer`), so a machine
/// listing no longer beats what a person said by repeating the query once per
/// line. Old files are rebuilt rather than migrated.
const INDEX_SCHEMA: i64 = 13;

/// Presentation scores. These are **derived** from the final rank
/// ([`ranking::RankKey`]) so that every existing consumer keeps seeing the
/// product ladder it already knew — exact 1.0 > proximity 0.9 > matched 0.85 >
/// n-gram fallback 0.72 — while the order itself is computed from evidence.
/// They are not comparable across queries, and nothing ranks on them.
const SCORE_LIKE: f64 = 1.0;
const SCORE_MATCH: f64 = 0.90;
const SCORE_RANKED: f64 = 0.85;
const SCORE_NEAR: f64 = 0.90;
const SCORE_FALLBACK: f64 = 0.72;

/// The ladder value a band is shown with.
fn band_score(band: ranking::RankBand) -> f64 {
    match band {
        ranking::RankBand::Exact => SCORE_LIKE,
        ranking::RankBand::Proximity => SCORE_NEAR,
        ranking::RankBand::Fusion => SCORE_RANKED,
        ranking::RankBand::Fuzzy => SCORE_FALLBACK,
    }
}

/// Below this many chars a query has no trigram at all: the index cannot serve
/// it, and the `LIKE` lane scans the rows directly.
const TRIGRAM_LEN: usize = query_plan::TRIGRAM_LEN;

/// `NEAR` window: FTS5's default distance, counted as tokens *between* the two
/// phrases. A whole-sentence query usually exceeds it, which is the point —
/// this is the precision layer; the routed `MATCH` below it is the recall one.
const NEAR_DISTANCE: usize = 10;
/// More terms than this would make `NEAR` unsatisfiable. The window keeps the
/// query's *informative* terms, in the order they were written
/// ([`query_plan::QueryBranch::near_terms`]).
const NEAR_MAX_TERMS: usize = query_plan::NEAR_MAX_TERMS;

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
    /// Presentation value derived from [`Self::rank`]: the product ladder. Never
    /// compare it across queries, and never rank on it.
    pub score: f64,
    /// The row's item type (`function_call`, `message`, …) — callers label hits
    /// with it.
    pub item_type: String,
    /// What question the row's text answers (人话 / 动作 / 产出). Ranking only;
    /// nothing is filtered by it.
    pub role: ContentRole,
    /// Every layer that returned this chunk, with the query coverage it proved.
    /// Merged across chunks when the hit is folded to its row, so "three
    /// mechanisms agree on this row" survives into the final ranking.
    pub evidence: Vec<HitEvidence>,
    /// The lane's own ordering key. The final layer rewrites it once the sparse
    /// and semantic lists are fused.
    pub rank: RankKey,
    /// Snippet around the match, taken from the chunk text. The renderer prefers
    /// the physical line it resolves from `char_start`; this is the fallback.
    pub summary: String,
    /// Raw `bm25()` for ranked hits; `None` for `LIKE` hits. Negative is a
    /// better match (SQLite returns the negated score). Kept for diagnosis: it
    /// is what explains a poor rank on a corpus of very long rows.
    #[allow(dead_code)]
    pub bm25: Option<f64>,
}

impl SparseHit {
    /// The layers that independently found this hit, deduped and in priority
    /// order.
    pub fn layers(&self) -> Vec<LayerId> {
        let mut out: Vec<LayerId> = Vec::new();
        for evidence in &self.evidence {
            if !out.contains(&evidence.layer) {
                out.push(evidence.layer);
            }
        }
        out.sort();
        out
    }

    /// How many layers found it. Agreement is evidence.
    pub fn layer_count(&self) -> usize {
        self.layers().len()
    }

    /// The coverage the hit proved for one layer, if that layer found it.
    pub fn coverage_of(&self, layer: LayerId) -> Option<f64> {
        self.evidence
            .iter()
            .filter(|e| e.layer == layer)
            .map(|e| e.coverage())
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }
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
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hits for one lane, best first. `limit` caps the list; the recipe's own
    /// order is preserved (exact substring first, then BM25).
    ///
    /// `|` separates alternatives and any branch may match — the contract the
    /// tool documents to the agent. Each branch is searched and the best hit per
    /// key wins, so "A|B" is the union of what A and B each find.
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
                        // The better-ranked branch owns the chunk: a later branch
                        // cannot demote a hit an earlier one proved more strongly.
                        if ranking::cmp_rank(&hit.rank, &slot.get().rank) == Ordering::Less {
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
            // The declared rank first, for the same reason the full layer uses
            // it: two branches can disagree about coverage, and the display score
            // only carries the band and the position within it.
            ranking::cmp_rank(&a.rank, &b.rank)
                .then_with(|| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
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
            Lane::Unicode => match word_query_gated(&QueryBranch::parse(&needle)) {
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
                // The product layer's membership question is the product
                // layer's gate: a row the coverage rule would reject is not a
                // row the agent could have been shown, so it is not a hit.
                if self.contains_like(&needle, session_id, seq)? {
                    return Ok(true);
                }
                let branch = QueryBranch::parse(&needle);
                if branch.proximity_applies()
                    && let Some(mq) = near_query(&needle)
                    && self.contains_match("uni", &mq, session_id, seq)?
                {
                    return Ok(true);
                }
                self.contains_gate(&branch, session_id, seq)
            }
        }
    }

    /// Is this pinned row a `Lexical` hit under the branch's coverage gate?
    ///
    /// Answered per chunk, from the chunk's own normalized text: the ranker
    /// works per chunk, so the gate has to as well — a row is a hit when *one*
    /// of its chunks clears the gate, not when its chunks collectively do.
    fn contains_gate(&self, branch: &QueryBranch, session_id: &str, seq: i64) -> Result<bool> {
        if branch.cjk {
            if !branch.grams_apply() {
                return Ok(false);
            }
            let Some(mq) = trigram_query(&branch.normalized) else {
                return Ok(false);
            };
            // The `AND`-over-every-gram form *is* the literal; a subset is what
            // the count below answers.
            let semantics = ranking::semantics(LayerId::Lexical);
            let min = branch.gram_min(semantics.min_coverage);
            if min >= branch.grams.len() && self.contains_match("tri", &mq, session_id, seq)? {
                return Ok(true);
            }
            let mut stmt = self.conn.prepare_cached(
                "SELECT text_norm FROM rows WHERE session_id = ?1 AND seq = ?2",
            )?;
            let texts = stmt
                .query_map(params![session_id, seq], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(texts
                .iter()
                .any(|text| query_plan::count_grams(text, &branch.grams) >= min));
        }
        if !branch.words_apply() {
            return Ok(false);
        }
        let min = branch.word_min();
        if min >= branch.content_terms.len()
            && let Some(mq) = word_query_gated(branch)
            && self.contains_match("uni", &mq, session_id, seq)?
        {
            return Ok(true);
        }
        let mut stmt = self
            .conn
            .prepare_cached("SELECT text_norm FROM rows WHERE session_id = ?1 AND seq = ?2")?;
        let texts = stmt
            .query_map(params![session_id, seq], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(texts
            .iter()
            .any(|text| query_plan::count_word_terms(text, &branch.content_terms) >= min))
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
        // Below the trigram floor the index cannot answer, so the pinned row is
        // checked against its own text directly.
        if needle.chars().count() < TRIGRAM_LEN {
            let hit: Option<i64> = self
                .conn
                .query_row(
                    "SELECT 1 FROM rows
                      WHERE session_id = ?1 AND seq = ?2 AND instr(text_norm, ?3) > 0
                      LIMIT 1",
                    params![session_id, seq, needle],
                    |row| row.get(0),
                )
                .optional()?;
            return Ok(hit.is_some());
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
    ///
    /// Below the trigram floor there is no index to give up: the rows are scanned
    /// directly. A two-character CJK term is an ordinary query, not an error.
    fn like_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        if needle.chars().count() < TRIGRAM_LEN {
            return self.scan_like_hits(needle, limit);
        }
        // The `ORDER BY` picks *which* literal hits survive about to be ranked,
        // so it is a relevance decision, not cosmetics. It used to be newest
        // first, which meant a common literal's top-`limit` was simply the last
        // `limit` rows of the corpus and every later layer was truncated away
        // behind it. It is now the shortest chunk first — the documented
        // length prior, and the one thing a `LIKE` probe can see about how much
        // a chunk is *about* the literal. The full ranking happens above this
        // layer, where coverage, role and recency are all available.
        //
        // `instr` is asked for the same literal alongside the LIKE, so the hit
        // carries where the substring landed (normalized chars; `r.text` maps it
        // back to the original row coordinates).
        let branch = QueryBranch::parse(needle);
        let scope_clause = if self.scope.is_some() {
            " AND r.session_id = ?4"
        } else {
            ""
        };
        let sql = format!(
            "SELECT r.session_id, r.seq, r.chunk, r.text, r.single, r.item_type,
                    instr(t.text_norm, ?2) AS pos, r.char_start AS chunk_start, r.role
               FROM tri t
               JOIN rows r ON r.rowid = t.rowid
              WHERE t.text_norm LIKE ?1
                AND instr(t.text_norm, ?2) > 0{scope_clause}
              ORDER BY length(t.text_norm) ASC, r.session_id DESC, r.seq DESC, r.chunk DESC
              LIMIT ?3"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let pattern = format!("%{needle}%");
        let limit_i64 = limit as i64;
        let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&pattern, &needle, &limit_i64];
        if let Some(scope) = &self.scope {
            bind.push(scope);
        }
        let total = branch.terms.len().max(1);
        let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
            let text: String = row.get(3)?;
            let pos: i64 = row.get(6)?;
            let chunk_start: i64 = row.get(7)?;
            let role = ContentRole::parse(&row.get::<_, String>(8)?);
            let (span, occurrences) = like_span_and_occurrences(&text, needle, pos.max(1) as usize);
            let mut hit = row_hit(
                row,
                &text,
                span,
                SCORE_LIKE,
                None,
                chunk_start.max(0) as usize,
                role,
            )?;
            // The literal is present in full, so coverage is complete by
            // construction; the occurrence count is what varies.
            hit.evidence
                .push(HitEvidence::new(LayerId::Exact, 0, total, total));
            hit.evidence[0].native = occurrences as f64;
            hit.rank = rank_key_for(&hit);
            Ok(hit)
        })?;
        finish_exact_hits(rows, limit)
    }

    /// `LIKE` without the index: a direct scan of the normalized row text, for
    /// needles the trigram table cannot serve. The same `instr` predicate the
    /// indexed path uses to confirm a hit is the only one needed here.
    fn scan_like_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let branch = QueryBranch::parse(needle);
        let scope_clause = if self.scope.is_some() {
            " AND r.session_id = ?3"
        } else {
            ""
        };
        let sql = format!(
            "SELECT r.session_id, r.seq, r.chunk, r.text, r.single, r.item_type,
                    instr(r.text_norm, ?1) AS pos, r.char_start AS chunk_start, r.role
               FROM rows r
              WHERE instr(r.text_norm, ?1) > 0{scope_clause}
              ORDER BY length(r.text_norm) ASC, r.session_id DESC, r.seq DESC, r.chunk DESC
              LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let limit_i64 = limit as i64;
        let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&needle, &limit_i64];
        if let Some(scope) = &self.scope {
            bind.push(scope);
        }
        let total = branch.terms.len().max(1);
        let rows = stmt.query_map(rusqlite::params_from_iter(bind), |row| {
            let text: String = row.get(3)?;
            let pos: i64 = row.get(6)?;
            let chunk_start: i64 = row.get(7)?;
            let role = ContentRole::parse(&row.get::<_, String>(8)?);
            let (span, occurrences) = like_span_and_occurrences(&text, needle, pos.max(1) as usize);
            let mut hit = row_hit(
                row,
                &text,
                span,
                SCORE_LIKE,
                None,
                chunk_start.max(0) as usize,
                role,
            )?;
            hit.evidence
                .push(HitEvidence::new(LayerId::Exact, 0, total, total));
            hit.evidence[0].native = occurrences as f64;
            hit.rank = rank_key_for(&hit);
            Ok(hit)
        })?;
        finish_exact_hits(rows, limit)
    }

    /// CJK path: trigram `MATCH` over the whole normalized query, BM25 order.
    fn trigram_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        self.trigram_hits_scored(needle, limit, SCORE_MATCH)
    }

    fn trigram_hits_scored(&self, needle: &str, limit: usize, score: f64) -> Result<Vec<SparseHit>> {
        let Some(match_query) = trigram_query(needle) else {
            return Ok(Vec::new());
        };
        self.leaf_hits(Leaf::ungated(LayerId::Fuzzy, "tri", &match_query, limit, score))
    }

    /// Non-CJK path: `unicode61` `MATCH` over the query's word tokens.
    fn unicode_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        self.unicode_hits_scored(needle, limit, SCORE_MATCH)
    }

    fn unicode_hits_scored(&self, needle: &str, limit: usize, score: f64) -> Result<Vec<SparseHit>> {
        let Some(match_query) = word_query(needle) else {
            return Ok(Vec::new());
        };
        self.leaf_hits(Leaf::ungated(LayerId::Lexical, "uni", &match_query, limit, score))
    }

    /// Proximity layer: FTS5's `NEAR(...)` over the query's informative words.
    fn near_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let Some(match_query) = near_query(needle) else {
            return Ok(Vec::new());
        };
        self.leaf_hits(Leaf::ungated(
            LayerId::Proximity,
            "uni",
            &match_query,
            limit,
            SCORE_NEAR,
        ))
    }

    /// The routed recipe: script decides the tokenizer, and a query too short
    /// for a trigram falls back to `LIKE` (documented trigram limitation).
    fn recipe_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        self.routed_hits(needle, limit, SCORE_MATCH)
    }

    /// Routed `MATCH` with the caller's score. The product layer calls it for
    /// its lexical layer; the eval lane uses it for the routed mechanism as a
    /// whole.
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

    /// The product layer: the four declared leaf layers, each gated by the
    /// intent its hits prove, then merged, folded to rows and ranked on
    /// [`ranking::RankKey`].
    ///
    /// The difference from the ladder this replaces is not the mechanisms — it
    /// is that nothing is decided by *position in a vector* any more:
    ///
    /// * every layer's hits land in one pool before any truncation, so a layer
    ///   cannot be computed and then silently dropped by an earlier layer's
    ///   `LIMIT`;
    /// * a layer that does not run records why (`StopReason`), so "no proximity
    ///   hits" is distinguishable from "proximity never ran";
    /// * the ranking is a tuple over evidence (band → coverage → role → the
    ///   producing layer's own rank), not a set of pre-baked floats to be
    ///   appended to.
    fn compose_sparse(&self, needle: &str, limit: usize) -> Result<(Vec<SparseHit>, Vec<LayerTrace>)> {
        let branch = QueryBranch::parse(needle);
        if branch.is_empty() || limit == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut traces: Vec<LayerTrace> = Vec::new();
        let mut pool: Vec<SparseHit> = Vec::new();

        // 1. Exact: the literal the user typed, verbatim. Always entered, and
        //    never gated: an exact substring is the strongest evidence there is.
        let exact = self.like_hits(needle, semantics_depth(LayerId::Exact, limit))?;
        traces.push(LayerTrace {
            layer: LayerId::Exact,
            produced: exact.len(),
            rejected: 0,
            accepted: exact.len(),
            stop: StopReason::Ran,
        });
        pool.extend(exact);

        // 2. Proximity: the query's informative words, inside one window.
        //
        // From here down, a layer is only entered when the layers above it have
        // not already filled the pool. A lower band can never outrank a higher
        // one, so once the higher band alone can fill the caller's list, the work
        // below it is wasted — and saying so explicitly is what keeps "no
        // results from this layer" from meaning "this layer never ran".
        let proximity = if distinct_rows(&pool) >= limit {
            traces.push(skipped(LayerId::Proximity, StopReason::EnoughHighConfidence));
            Vec::new()
        } else if branch.proximity_applies() {
            let terms = branch.near_terms();
            match near_query(needle) {
                Some(match_query) => {
                    let semantics = ranking::semantics(LayerId::Proximity);
                    let (hits, rejected) = self.match_hits(Leaf {
                        layer: LayerId::Proximity,
                        table: "uni",
                        match_query: &match_query,
                        gate_terms: &terms,
                        gate_min: semantics.min_matches(terms.len()),
                        coverage: Coverage::Words,
                        min_run: 0,
                        limit: semantics.depth(limit),
                        display: SCORE_NEAR,
                    })?;
                    traces.push(LayerTrace {
                        layer: LayerId::Proximity,
                        produced: hits.len() + rejected,
                        rejected,
                        accepted: hits.len(),
                        stop: StopReason::Ran,
                    });
                    hits
                }
                None => {
                    traces.push(skipped(LayerId::Proximity, StopReason::NotApplicable));
                    Vec::new()
                }
            }
        } else {
            traces.push(skipped(LayerId::Proximity, StopReason::NotApplicable));
            Vec::new()
        };
        pool.extend(proximity);

        // 3. Lexical: the coverage gate that stops one common word from filling
        //    the list. Words for Latin, trigrams for CJK, same declared layer.
        let semantics = ranking::semantics(LayerId::Lexical);
        let lexical = if distinct_rows(&pool) >= limit {
            traces.push(skipped(LayerId::Lexical, StopReason::EnoughHighConfidence));
            Vec::new()
        } else if branch.cjk {
            match trigram_query(&branch.normalized) {
                Some(match_query) => {
                    let (hits, rejected) = self.match_hits(Leaf {
                        layer: LayerId::Lexical,
                        table: "tri",
                        match_query: &match_query,
                        gate_terms: &branch.grams,
                        gate_min: branch.gram_min(semantics.min_coverage),
                        coverage: Coverage::Grams,
                        min_run: 0,
                        limit: semantics.depth(limit),
                        display: SCORE_RANKED,
                    })?;
                    traces.push(LayerTrace {
                        layer: LayerId::Lexical,
                        produced: hits.len() + rejected,
                        rejected,
                        accepted: hits.len(),
                        stop: StopReason::Ran,
                    });
                    hits
                }
                None => {
                    // Below the trigram floor the index cannot answer, so the
                    // literal is the only mechanism left.
                    traces.push(skipped(LayerId::Lexical, StopReason::NotApplicable));
                    Vec::new()
                }
            }
        } else if branch.words_apply() {
            match word_query_gated(&branch) {
                Some(match_query) => {
                    let (hits, rejected) = self.match_hits(Leaf {
                        layer: LayerId::Lexical,
                        table: "uni",
                        match_query: &match_query,
                        gate_terms: &branch.content_terms,
                        gate_min: branch.word_min(),
                        coverage: Coverage::Words,
                        min_run: 0,
                        limit: semantics.depth(limit),
                        display: SCORE_RANKED,
                    })?;
                    traces.push(LayerTrace {
                        layer: LayerId::Lexical,
                        produced: hits.len() + rejected,
                        rejected,
                        accepted: hits.len(),
                        stop: StopReason::Ran,
                    });
                    hits
                }
                None => {
                    traces.push(skipped(LayerId::Lexical, StopReason::NotApplicable));
                    Vec::new()
                }
            }
        } else {
            traces.push(skipped(LayerId::Lexical, StopReason::NotApplicable));
            Vec::new()
        };
        pool.extend(lexical);

        // 4. Fuzzy: last resort, and only when the layers above did not already
        //    answer with enough distinct rows. The decision is explicit here
        //    instead of an implicit "the pool happened to be empty".
        if distinct_rows(&pool) >= limit {
            traces.push(skipped(
                LayerId::Fuzzy,
                StopReason::EnoughHighConfidence,
            ));
        } else if let Some(match_query) = trigram_query(&branch.normalized) {
            let semantics = ranking::semantics(LayerId::Fuzzy);
            let (hits, rejected) = self.match_hits(Leaf {
                layer: LayerId::Fuzzy,
                table: "tri",
                match_query: &match_query,
                gate_terms: &branch.grams,
                gate_min: branch.gram_min(semantics.min_coverage),
                coverage: Coverage::Grams,
                min_run: branch.min_fuzzy_run(semantics.min_run_fraction),
                limit: semantics.depth(limit),
                display: SCORE_FALLBACK,
            })?;
            traces.push(LayerTrace {
                layer: LayerId::Fuzzy,
                produced: hits.len() + rejected,
                rejected,
                accepted: hits.len(),
                stop: StopReason::RecallShortfall,
            });
            pool.extend(hits);
        } else {
            traces.push(skipped(LayerId::Fuzzy, StopReason::NotApplicable));
        }

        // Every layer's evidence is merged per chunk, then per row, before
        // anything is truncated: a chunk can no longer consume a row's slot,
        // and a row supported by three mechanisms says so.
        let mut rows = merge_and_fold(pool);
        for hit in rows.iter_mut() {
            hit.rank = rank_key_for(hit);
        }
        rows.sort_by(|a, b| {
            ranking::cmp_rank(&a.rank, &b.rank)
                .then_with(|| b.session_id.cmp(&a.session_id))
                .then_with(|| b.seq.cmp(&a.seq))
        });
        rows.truncate(limit);
        // Presentation only: the ladder value the band is shown with, decayed by
        // the hit's final position so the existing board keeps its shape.
        for (index, hit) in rows.iter_mut().enumerate() {
            hit.score = tier_score(band_score(hit.rank.band), index, limit);
        }
        Ok((rows, traces))
    }

    /// The product ladder's entry point.
    fn final_hits(&self, needle: &str, limit: usize) -> Result<Vec<SparseHit>> {
        let (hits, traces) = self.compose_sparse(needle, limit)?;
        if tracing::enabled!(tracing::Level::DEBUG) {
            for trace in &traces {
                tracing::debug!(
                    layer = trace.layer.as_str(),
                    produced = trace.produced,
                    rejected = trace.rejected,
                    accepted = trace.accepted,
                    stop = ?trace.stop,
                    "sparse layer"
                );
            }
        }
        Ok(hits)
    }

    /// Run one leaf layer and collect what its hits prove.
    fn leaf_hits(&self, leaf: Leaf<'_>) -> Result<Vec<SparseHit>> {
        Ok(self.match_hits(leaf)?.0)
    }

    /// Run one leaf layer's `MATCH` and prove what each hit covers.
    ///
    /// Returns the accepted hits (ranked, with evidence) and how many the gate
    /// rejected — the count that says whether the gate did anything at all.
    fn match_hits(&self, leaf: Leaf<'_>) -> Result<(Vec<SparseHit>, usize)> {
        // `table` is one of two hard-coded literals, never user input.
        //
        // `highlight()` wraps every matched token in the stored (normalized)
        // text; the hit turns the densest cluster of those marks into one span
        // in original row coordinates, and the marks themselves into the
        // coverage count. (FTS5 has no `offsets()`.)
        let table = leaf.table;
        let scope_clause = if self.scope.is_some() {
            " AND r.session_id = ?3"
        } else {
            ""
        };
        let sql = format!(
            "SELECT r.session_id, r.seq, r.chunk, r.text, r.single, r.item_type,
                    highlight({table}, 0, char(2), char(3)) AS hl,
                    bm25({table}) AS rank,
                    r.char_start AS chunk_start, r.role
               FROM {table} t
               JOIN rows r ON r.rowid = t.rowid
              WHERE {table} MATCH ?1{scope_clause}
              ORDER BY rank
              LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let limit_i64 = leaf.limit as i64;
        let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&leaf.match_query, &limit_i64];
        if let Some(scope) = &self.scope {
            bind.push(scope);
        }
        let terms = leaf.gate_terms.to_vec();
        let total = terms.len();
        let gate_min = leaf.gate_min;
        let coverage = leaf.coverage;
        let min_run = leaf.min_run;
        let layer = leaf.layer;
        let display = leaf.display;
        let depth = leaf.limit.max(1);
        let rows = stmt.query_map(rusqlite::params_from_iter(bind), move |row| {
            let text: String = row.get(3)?;
            let marked: Option<String> = row.get(6)?;
            let bm25 = row.get(7).ok();
            let chunk_start: i64 = row.get(8)?;
            let role = ContentRole::parse(&row.get::<_, String>(9)?);
            let marked = marked.unwrap_or_default();
            let span = highlight_span(&text, &marked);
            // Coverage is counted from the row's own text, not from the
            // `highlight()` output: FTS5 merges adjacent and overlapping token
            // marks into one run, so per-token marks cannot be recovered from it
            // (and a 3-char trigram query marks one long run). The text is the
            // ground truth anyway, and this way the gate means exactly what it
            // says: how much of the query's intent this row contains.
            let normalized_chunk = normalize(&text);
            let matched = match coverage {
                Coverage::None => 0,
                Coverage::Words => query_plan::count_word_terms(&normalized_chunk, &terms),
                Coverage::Grams => query_plan::count_grams(&normalized_chunk, &terms),
            };
            let mut evidence = HitEvidence::new(layer, 0, matched, total);
            evidence.native = bm25.unwrap_or(0.0);
            // The gate is computed here and answered by the caller's loop so
            // rejections are counted. Counted coverage, never a score threshold:
            // `the auth refactor token` cannot be satisfied by `the` alone,
            // however good its BM25.
            let clears = evidence.clears(gate_min);
            // Typo tolerance is not "some grams match": a long enough fragment
            // of the query must be there unbroken, or the row is a coincidence
            // of common characters rather than a near miss of the phrase.
            let adjacent = min_run == 0
                || query_plan::longest_contiguous_run(&normalized_chunk, &terms) >= min_run;
            let mut hit = row_hit(
                row,
                &text,
                span,
                display,
                bm25,
                chunk_start.max(0) as usize,
                role,
            )?;
            hit.evidence.push(evidence);
            Ok((hit, clears && adjacent, depth))
        })?;
        let mut out = Vec::new();
        let mut rejected = 0usize;
        for row in rows {
            let (mut hit, accepted, depth) = row?;
            if !accepted {
                rejected += 1;
                continue;
            }
            let rank = out.len();
            hit.evidence[0].local_rank = rank;
            // Keep the layer's own order *inside* the layer: the final ranking
            // breaks ties by it, so BM25 is never replaced by recency.
            hit.score = tier_score(display, rank, depth);
            hit.rank = RankKey {
                band: ranking::RankBand::from_layer(layer),
                strength: strength_of(&hit),
                role: hit.role.preference(),
                local_rank: rank as u32,
            };
            out.push(hit);
        }
        Ok((out, rejected))
    }
}

/// Finish the `Exact` layer's rows.
///
/// The layer's own order is the length prior the SQL applied — a chunk that is
/// mostly about the literal before one that mentions it once in passing — so its
/// position in that order **is** its rank. Without this every exact hit reports
/// rank 0, which is how a flat score band erases the difference between them.
fn finish_exact_hits(
    rows: impl Iterator<Item = rusqlite::Result<SparseHit>>,
    limit: usize,
) -> Result<Vec<SparseHit>> {
    let mut hits = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for (index, hit) in hits.iter_mut().enumerate() {
        if let Some(evidence) = hit.evidence.first_mut() {
            evidence.local_rank = index;
        }
        hit.rank.local_rank = index as u32;
        hit.score = tier_score(SCORE_LIKE, index, limit.max(1));
    }
    Ok(hits)
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
    role: ContentRole,
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
    // The span is in the chunk's own coordinates — every producer runs over the
    // chunk text (`highlight_span`, `like_match_span`). A hit carries row
    // coordinates, because that is what the renderer resolves to a physical
    // line, so shift the span up by the chunk's start. The snippet is taken
    // from the chunk text, which is exactly the span's own coordinates.
    let summary = super::snippet_from_span(text, span.0, span.1);
    Ok(SparseHit {
        key,
        row_key,
        session_id,
        seq,
        chunk: chunk as usize,
        char_start: span.0 + chunk_start,
        char_end: span.1 + chunk_start,
        score,
        item_type,
        role,
        evidence: Vec::new(),
        rank: RankKey::default(),
        summary,
        bm25,
    })
}

/// The characters FTS5's `highlight()` wraps around each matched token.

/// What a layer's coverage counts against: the query's words, or its trigrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coverage {
    /// No gate: every row the table returns counts (the eval lanes).
    None,
    /// Whole words, so `session` does not count for `sessions`.
    Words,
    /// Substrings of three chars, for scripts `unicode61` cannot segment.
    Grams,
}

/// One leaf layer's request: which table and query to run, how the lane values
/// it, and the gate a hit must clear before it counts.
///
/// This is where a layer's declared semantics
/// ([`ranking::LayerSemantics`]) meet its mechanism. A layer is a mechanism plus
/// a gate; the gate is what the old ladder never had.
struct Leaf<'a> {
    layer: LayerId,
    table: &'a str,
    match_query: &'a str,
    /// Terms the coverage gate counts, and how many must be present. An empty
    /// list is a bare mechanism: no gate, every row the table returns counts.
    gate_terms: &'a [String],
    gate_min: usize,
    coverage: Coverage,
    /// The fuzzy layer additionally requires an unbroken fragment of the query:
    /// at least this many chars of it, copied verbatim.
    min_run: usize,
    limit: usize,
    display: f64,
}

impl<'a> Leaf<'a> {
    /// A layer run as a bare mechanism, ungated: what the eval lanes ask for.
    fn ungated(
        layer: LayerId,
        table: &'a str,
        match_query: &'a str,
        limit: usize,
        display: f64,
    ) -> Self {
        Self {
            layer,
            table,
            match_query,
            gate_terms: &[],
            gate_min: 0,
            coverage: Coverage::None,
            min_run: 0,
            limit,
            display,
        }
    }
}

/// A layer's declared candidate depth for one query.
fn semantics_depth(layer: LayerId, limit: usize) -> usize {
    ranking::semantics(layer).depth(limit)
}

/// The trace entry for a layer that did not run.
fn skipped(layer: LayerId, stop: StopReason) -> LayerTrace {
    LayerTrace {
        layer,
        produced: 0,
        rejected: 0,
        accepted: 0,
        stop,
    }
}

/// Distinct rows in a pool of chunk hits.
fn distinct_rows(hits: &[SparseHit]) -> usize {
    hits.iter()
        .map(|h| (h.session_id.as_str(), h.seq))
        .collect::<HashSet<_>>()
        .len()
}

/// Within-band strength for a hit, from its best evidence.
///
/// `Exact`: how many times the literal occurs — a row that says it five times is
/// more about it than a row that says it once. Everything else: coverage, the
/// fraction of the query's intent this hit proved. Both are multiplied by a
/// small agreement bonus: the number of layers that independently found the
/// row. Agreement is evidence, and it is the one signal that only exists because
/// provenance was kept.
///
/// The total is then scaled by the hit's [`ContentRole`] weight. That is the one
/// place a role reaches the ordering, and it is deliberately inside this number:
/// the band keeps exactly one comparable scale, and the tier reorders equal
/// quality without overriding it.
fn strength_of(hit: &SparseHit) -> u32 {
    let primary = primary_evidence(hit);
    let base = match primary.layer {
        LayerId::Exact => primary.native.max(0.0).min(999.0) as u32,
        _ => primary.coverage_permille(),
    };
    let earned = base
        .saturating_mul(100)
        .saturating_add(hit.layer_count().min(99) as u32);
    hit.role.weigh(earned)
}

/// The evidence that explains a hit best: the highest band that found it, then
/// the most coverage, then the producing layer's own order.
fn primary_evidence(hit: &SparseHit) -> HitEvidence {
    hit.evidence
        .iter()
        .min_by(|a, b| {
            cmp_evidence(a, b)
        })
        .cloned()
        .unwrap_or_else(|| HitEvidence::new(LayerId::Exact, 0, 0, 0))
}

/// Order two pieces of evidence, best first. Only ever compares evidence *of
/// one hit*, so the scale question (which native score is bigger) never arises.
fn cmp_evidence(a: &HitEvidence, b: &HitEvidence) -> Ordering {
    ranking::cmp_rank(
        &RankKey {
            band: a.band(),
            strength: a.coverage_permille(),
            role: 0,
            local_rank: a.local_rank as u32,
        },
        &RankKey {
            band: b.band(),
            strength: b.coverage_permille(),
            role: 0,
            local_rank: b.local_rank as u32,
        },
    )
}

/// The rank key a hit carries out of the sparse lane, before any fusion.
fn rank_key_for(hit: &SparseHit) -> RankKey {
    let primary = primary_evidence(hit);
    RankKey {
        band: primary.band(),
        strength: strength_of(hit),
        role: hit.role.preference(),
        local_rank: primary.local_rank as u32,
    }
}

/// Merge each chunk's evidence, then fold each row to its best chunk.
///
/// Both steps happen **before** anything is truncated. Otherwise a long row's
/// five matching chunks would take five of the caller's slots, and a row found
/// by three mechanisms would look like a row found by one — the two ways a
/// chunk-level pool silently distorts a row-level answer.
fn merge_and_fold(pool: Vec<SparseHit>) -> Vec<SparseHit> {
    let mut by_chunk: BTreeMap<String, SparseHit> = BTreeMap::new();
    for hit in pool {
        match by_chunk.entry(hit.key.clone()) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(hit);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let existing = slot.get_mut();
                let better = ranking::cmp_rank(&hit.rank, &existing.rank) == Ordering::Less;
                for evidence in &hit.evidence {
                    if !existing.evidence.contains(evidence) {
                        existing.evidence.push(evidence.clone());
                    }
                }
                if better {
                    // The better producer owns the span: an exact literal points
                    // at the literal, a fuzzy gram at a cluster of them.
                    let evidence = std::mem::take(&mut existing.evidence);
                    let mut replacement = hit;
                    replacement.evidence = evidence;
                    *existing = replacement;
                }
            }
        }
    }

    let mut by_row: BTreeMap<(String, i64), SparseHit> = BTreeMap::new();
    for hit in by_chunk.into_values() {
        match by_row.entry((hit.session_id.clone(), hit.seq)) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(hit);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let existing = slot.get_mut();
                let better = ranking::cmp_rank(&hit.rank, &existing.rank) == Ordering::Less;
                for evidence in &hit.evidence {
                    if !existing.evidence.contains(evidence) {
                        existing.evidence.push(evidence.clone());
                    }
                }
                if better {
                    let evidence = std::mem::take(&mut existing.evidence);
                    let mut replacement = hit;
                    replacement.evidence = evidence;
                    *existing = replacement;
                }
            }
        }
    }
    by_row.into_values().collect()
}

/// Split a query on `|` into trimmed, non-empty, deduped alternatives — the
/// contract the tool documents (`|` = alternatives, any may match), mirrored
/// exactly so the lane answers "A|B" the way production does.
fn split_alternatives(query: &str) -> Vec<&str> {
    query_plan::split_branches(query)
}

/// Score for the `index`-th hit of a band: a 2%-of-band decay that keeps the
/// band strictly below the one above and above the one below, while leaving the
/// layer's own order intact through the product's score sort.
///
/// Presentation only. The order was already decided by [`ranking::RankKey`].
fn tier_score(base: f64, index: usize, limit: usize) -> f64 {
    base - 0.02 * (index as f64 / limit.max(1) as f64)
}

fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create sparse index dir {}", parent.display()))?;
    }
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
/// [`SOURCE_STATE_DDL`] carries the per-row derived state.
const ROWS_DDL: &str = "CREATE TABLE IF NOT EXISTS rows (
        rowid      INTEGER PRIMARY KEY,
        session_id TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        chunk      INTEGER NOT NULL DEFAULT 0,
        single     INTEGER NOT NULL DEFAULT 1,
        item_type  TEXT NOT NULL DEFAULT '',
        role       TEXT NOT NULL DEFAULT 'unknown',
        char_start INTEGER NOT NULL DEFAULT 0,
        char_end   INTEGER NOT NULL DEFAULT 0,
        text_norm  TEXT NOT NULL,
        text       TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS rows_key ON rows(session_id, seq);";

/// `source_state` is the reconciliation ledger: one row per settled source row,
/// carrying its allocation into the index plus the call linkage the echo rule
/// needs. It deliberately keeps metadata for rows that never reach `rows`
/// (echo results, compacted summaries) — they are part of the settled key set
/// too, and a rewritten call has to be able to find its result without a
/// corpus rescan.
///
/// There is no content hash here on purpose. A hash would have to cover every
/// input to the derivation to be a sound change detector, and one of those
/// inputs is *another row* — so the reconcile compares keys instead and
/// re-derives anything it does not already have.
const SOURCE_STATE_DDL: &str = "CREATE TABLE IF NOT EXISTS source_state (
        session_id        TEXT NOT NULL,
        seq               INTEGER NOT NULL,
        kind              TEXT NOT NULL,
        item_type         TEXT NOT NULL,
        call_id           TEXT,
        session_read_call INTEGER NOT NULL DEFAULT 0,
        in_chunks         INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (session_id, seq)
     );
     CREATE INDEX IF NOT EXISTS source_state_session_call
        ON source_state(session_id, call_id) WHERE call_id IS NOT NULL;";

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

/// True when the file cannot be used as it stands: absent, unopenable, or written
/// by another schema version.
///
/// An unopenable file is a rebuild, not an error to report. Nothing else can make
/// it readable, so handing the open failure back leaves the lane failed for good
/// — and the caller that could have replaced the file is the one being told to
/// give up.
pub fn needs_rebuild(path: &Path) -> bool {
    if !path.is_file() {
        return true;
    }
    let Ok(conn) = open(path) else {
        return true;
    };
    meta(&conn, "schema")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        != Some(INDEX_SCHEMA)
}

/// Open the index for a rebuild, replacing a file that cannot be read.
///
/// A database this build cannot open holds nothing to preserve, and the WAL
/// sidecars go with it: they describe a file that is being replaced. A file that
/// opens is left exactly where it is — the rebuild writes over it in one
/// transaction, so a failure still leaves the previous index live.
fn open_for_rebuild(path: &Path) -> Result<Connection> {
    if let Ok(conn) = open(path) {
        return Ok(conn);
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
    open(path)
}

/// Full build: make the index equal this corpus, atomically.
///
/// Everything happens inside **one** transaction, so a rebuild is published in a
/// single step or not at all. A crash, a kill, or a failure part-way through
/// leaves the previous index exactly as it was and rolls the new one back. There
/// is no window in which the file exists but holds a partial index, and a reader
/// never sees one.
///
/// The previous version deleted the file and rebuilt into the gap. That made
/// every rebuild briefly make the index *absent*: a search landing in the window
/// found nothing, and a crash in the window left nothing behind at all — the
/// worst possible pairing, because a rebuild is exactly when somebody is likely
/// to be searching.
///
/// The derivation is [`derive::derive_row`] — the same entry the incremental
/// updater uses. The two paths are allowed to differ in *what they write*, never
/// in *what they derive*.
pub fn build_index(rows: &[SearchableRow], data_root: &Path) -> Result<()> {
    build_index_at(rows, data_root, &sparse_index_path(data_root))
}

/// Build an index at an explicit path, deriving the rows against `data_root`.
///
/// The two are normally the same directory — an index belongs beside the corpus
/// it was derived from — but a *probe* over a live workspace needs to read that
/// workspace's blobs while writing its index somewhere else, so the derivation
/// root and the output path are separate arguments here.
pub fn build_index_at(rows: &[SearchableRow], data_root: &Path, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create sparse index dir {}", parent.display()))?;
    }
    let tk = super::tokenizer::shared()?;
    let chunk_cfg = chunk_cfg(CHUNK_TOKENS);
    let cfg = derive::DeriveCfg {
        tk: &tk,
        chunk: &chunk_cfg,
    };
    // Derive first, then take the write lock. Tokenizing is the slow part and
    // there is no reason to hold the file against readers while doing it.
    let derived = derive_rows(rows, data_root, &cfg, &HashSet::new())?;

    let mut conn = open_for_rebuild(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
    let tx = conn.transaction()?;
    // A rebuild is not a merge: the tables are recreated rather than cleared, so
    // a row that has left the settled set cannot survive because this pass
    // happened not to mention it, and a file left behind by an older schema
    // cannot be reused by accident.
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS rows_ai;
         DROP TRIGGER IF EXISTS rows_ad;
         DROP TABLE IF EXISTS rows;
         DROP TABLE IF EXISTS source_state;
         DROP TABLE IF EXISTS tri;
         DROP TABLE IF EXISTS uni;",
    )?;
    tx.execute_batch(&format!(
        "{ROWS_DDL} {SOURCE_STATE_DDL} {FTS_DDL} {TRIGGER_DDL}"
    ))?;

    let mut indexed = 0usize;
    for row in &derived {
        upsert_source_state(&tx, row)?;
        if row.in_chunks {
            insert_row_chunks(&tx, row)?;
            indexed += 1;
        }
    }

    // Publish nothing but a complete index. If anything went missing, the
    // transaction rolls back and the old index stays live, rather than being
    // replaced by one with a hole in it that looks perfectly healthy.
    let written: i64 = tx.query_row("SELECT COUNT(*) FROM source_state", [], |r| r.get(0))?;
    if written != derived.len() as i64 {
        return Err(anyhow::anyhow!(
            "rebuild wrote {written} of {} settled rows; refusing to publish",
            derived.len()
        ));
    }

    // Written last, in the same transaction as the rows: a reader that sees the
    // new schema is guaranteed to be looking at the index that goes with it.
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema', ?1)",
        params![INDEX_SCHEMA.to_string()],
    )?;
    tx.commit()?;
    tracing::info!(
        rows = indexed,
        settled = derived.len(),
        "sparse session index rebuilt"
    );
    Ok(())
}

/// Derive rows through the one pipeline. Echo admission is resolved first (it is
/// a cross-row relation), then each row is derived exactly once.
///
/// `known_echo_calls` seeds the closure with `(session_id, call_id)` pairs the
/// index already recorded.
fn derive_rows(
    rows: &[SearchableRow],
    data_root: &Path,
    cfg: &derive::DeriveCfg<'_>,
    known_echo_calls: &HashSet<(String, String)>,
) -> Result<Vec<derive::DerivedRow>> {
    let echo_keys = echo::result_keys_with(rows, data_root, known_echo_calls)?;
    rows.iter()
        .map(|row| {
            let excluded = echo_keys.contains(&(row.session_id.clone(), row.seq));
            derive::derive_row(row, data_root, cfg, excluded)
        })
        .collect()
}

/// What a reconciliation decided to do, in terms of source keys only.
struct Plan {
    to_add: Vec<(String, i64)>,
    to_remove: Vec<(String, i64)>,
}

/// Compare the final source keys against what the index holds.
///
/// This is the whole change detector. There is no content hash, because a hash
/// would have to cover every input to the derivation — and one of those inputs
/// is another row, so a per-row hash cannot be sound. Keys are enough precisely
/// because final rows never change: a row either enters the final set or leaves
/// it (revert, delete), and both show up here. A final row is never rewritten in
/// place, so nothing else has to be noticed.
fn plan_reconcile(conn: &Connection, source_keys: &[(String, i64)]) -> Result<Plan> {
    let source: HashSet<(String, i64)> = source_keys.iter().cloned().collect();
    let mut stmt = conn.prepare("SELECT session_id, seq FROM source_state")?;
    let indexed: HashSet<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;

    let mut to_add: Vec<(String, i64)> = source.difference(&indexed).cloned().collect();
    let mut to_remove: Vec<(String, i64)> = indexed.difference(&source).cloned().collect();
    // Deterministic order keeps the derived rows, and therefore the index, stable.
    to_add.sort();
    to_remove.sort();
    Ok(Plan { to_add, to_remove })
}

/// Calls the index already knows to be session reads, keyed by session.
fn known_echo_calls(conn: &Connection) -> Result<HashSet<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, call_id FROM source_state
         WHERE session_read_call = 1 AND call_id IS NOT NULL",
    )?;
    let pairs = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(pairs.collect::<rusqlite::Result<HashSet<_>>>()?)
}

/// Write one reconciliation. Rows must already be derived, and must correspond
/// exactly to `plan.to_add`: anything else would be a lie about the key set.
fn apply_plan(
    conn: &mut Connection,
    plan: &Plan,
    rows: &[SearchableRow],
    data_root: &Path,
) -> Result<usize> {
    let tk = super::tokenizer::shared()?;
    let chunk_cfg = chunk_cfg(CHUNK_TOKENS);
    let cfg = derive::DeriveCfg {
        tk: &tk,
        chunk: &chunk_cfg,
    };
    // A source row that cannot be read fails the whole batch here, before any
    // write: skipping it would leave a hole while the cursor moved on.
    let known = known_echo_calls(conn)?;
    let derived = derive_rows(rows, data_root, &cfg, &known)?;
    let derived_keys: HashSet<(String, i64)> = derived
        .iter()
        .map(|r| (r.session_id.clone(), r.seq))
        .collect();
    let planned: HashSet<(String, i64)> = plan.to_add.iter().cloned().collect();
    if derived_keys != planned {
        return Err(anyhow::anyhow!(
            "reconcile gave {} rows to add but derived {} bodies",
            planned.len(),
            derived_keys.len()
        ));
    }

    let tx = conn.transaction()?;
    let mut changed = 0usize;
    for row in &derived {
        // The row is new or newly final: write what it derives to. `in_chunks`
        // decides both halves, so a row that is not admitted stores its metadata
        // and contributes nothing.
        delete_row_chunks(&tx, &row.session_id, row.seq)?;
        if row.in_chunks {
            insert_row_chunks(&tx, row)?;
        }
        upsert_source_state(&tx, row)?;
        changed += 1;
    }
    for (sid, seq) in &plan.to_remove {
        delete_row_chunks(&tx, sid, *seq)?;
        tx.execute(
            "DELETE FROM source_state WHERE session_id = ?1 AND seq = ?2",
            params![sid, seq],
        )?;
        changed += 1;
    }
    tx.commit()?;
    Ok(changed)
}

/// Incremental refresh.
///
/// Reads the final source key set (integers, no bodies), diffs it against the
/// index, and fetches bodies only for the rows that are actually missing. Nothing
/// is decoded twice and nothing is guessed. This runs before every search, so a
/// key scan per query is expected; the cost is the diff, never the corpus.
///
/// The key scan and the body read are two separate reads of a store a writer may
/// be mutating, so a row can vanish between them — a revert the user asked for
/// while a search was in flight. That is the source moving, not a broken index,
/// so the cycle is replayed once against a fresh scan before it is reported as a
/// failure. A mismatch that survives the replay is real and stays loud.
pub fn refresh_from_source(reader: &SessionDataReader, data_root: &Path) -> Result<usize> {
    let path = sparse_index_path(data_root);
    let mut conn = open(&path)?;
    // Reconciliation can only move an index that already has the right shape. A
    // stale schema is a rebuild, not something to paper over row by row.
    ensure_schema(&conn)?;
    if meta(&conn, "schema").ok() != Some(INDEX_SCHEMA.to_string()) {
        return Err(anyhow::anyhow!(
            "sparse index schema is not current; rebuild before reconciling"
        ));
    }

    let mut source_keys = reader.searchable_keys_blocking(None)?;
    for attempt in 0..2 {
        let plan = plan_reconcile(&conn, &source_keys)?;
        // Only the bodies that are actually going to be written are read.
        let rows = reader.searchable_rows_for_blocking(&plan.to_add)?;
        match apply_plan(&mut conn, &plan, &rows, data_root) {
            Ok(changed) => return Ok(changed),
            Err(err) if attempt == 0 => {
                let fresh = reader.searchable_keys_blocking(None)?;
                if fresh == source_keys {
                    // The source did not move, so this is not a race: a body the
                    // plan promised could not be derived. Report it.
                    return Err(err);
                }
                source_keys = fresh;
            }
            Err(err) => return Err(err),
        }
    }
    unreachable!("the loop either returns a count or an error")
}

/// Refresh against an already-read corpus.
///
/// Same reconciliation, but the bodies are supplied instead of fetched — used by
/// the equivalence gate, which must be able to present a corpus that is not just
/// "whatever the store currently holds".
#[cfg(test)]
pub fn refresh_index(rows: &[SearchableRow], data_root: &Path) -> Result<usize> {
    let path = sparse_index_path(data_root);
    let source_keys: Vec<(String, i64)> = rows
        .iter()
        .map(|r| (r.session_id.clone(), r.seq))
        .collect();
    let mut conn = open(&path)?;
    let plan = plan_reconcile(&conn, &source_keys)?;
    let wanted: HashSet<(String, i64)> = plan.to_add.iter().cloned().collect();
    let rows: Vec<SearchableRow> = rows
        .iter()
        .filter(|r| wanted.contains(&(r.session_id.clone(), r.seq)))
        .cloned()
        .collect();
    let changed = apply_plan(&mut conn, &plan, &rows, data_root)?;
    Ok(changed)
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         {ROWS_DDL}
         {SOURCE_STATE_DDL}
         {FTS_DDL}
         {TRIGGER_DDL}"
    ))?;
    Ok(())
}

fn upsert_source_state(conn: &Connection, row: &derive::DerivedRow) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO source_state
            (session_id, seq, kind, item_type, call_id, session_read_call, in_chunks)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.session_id,
            row.seq,
            row.kind,
            row.item_type,
            row.call_id,
            i64::from(row.session_read_call),
            i64::from(row.in_chunks),
        ],
    )?;
    Ok(())
}

fn insert_row_chunks(conn: &Connection, row: &derive::DerivedRow) -> Result<usize> {
    if row.chunks.is_empty() {
        return Ok(0);
    }
    // The retrieval role is derived here from the same classification the corpus
    // policy already uses, so ranking can prefer intent over tool output without
    // a second source read at query time.
    let role = ContentRole::from_slot(super::slots::classify(&row.kind, &row.item_type));
    let total = row.chunks.len();
    let mut ins = conn.prepare_cached(
        "INSERT INTO rows(session_id, seq, chunk, single, item_type, role, char_start,
                          char_end, text_norm, text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for c in &row.chunks {
        // A trimmed chunk keeps its own `start`: the span shrinks from the tail
        // only, so the line a hit renders from is still the line it points at.
        let (end, text) = trimmer::trim_span(&c.text, c.start, c.end, role);
        ins.execute(params![
            row.session_id,
            row.seq,
            c.index as i64,
            i64::from(total == 1),
            row.item_type,
            role.as_str(),
            c.start as i64,
            end as i64,
            normalize(text),
            text,
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
    like_span_and_occurrences(text, needle, pos).0
}

/// The span **and** how many times the literal occurs in the chunk.
///
/// Two literal hits are equally exact, so what separates them is how much of the
/// row is about the literal at all: `occurrences` is that signal, and it is the
/// one thing the old "newest session first" ordering threw away.
fn like_span_and_occurrences(text: &str, needle: &str, pos: usize) -> ((usize, usize), usize) {
    let n = normalize_with_map(text);
    let start = pos.saturating_sub(1);
    let occurrences = n.norm.matches(needle).count();
    (
        map_span(&n, start, start + needle.chars().count()),
        occurrences,
    )
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
    let grams: BTreeSet<String> = query_plan::grams_of(needle).into_iter().collect();
    if grams.is_empty() {
        return None;
    }
    Some(
        grams
            .into_iter()
            .map(|gram| quote_term(&gram))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// `unicode61` word tokens of the query, deduped and sorted so the generated
/// query is deterministic. The tokenizer already drops punctuation and folds
/// case, so splitting on non-alphanumerics reproduces its segmentation.
fn word_terms(needle: &str) -> Vec<String> {
    query_plan::terms_sorted(needle)
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

/// The `unicode61` `MATCH` the lexical layer runs, which is the same terms but
/// asked as the gate wants them.
///
/// When the gate demands *every* informative word, the query says so and FTS5
/// does the filtering: no over-fetch and no post-filter. When it accepts a
/// subset, the words are OR-ed and the coverage count decides — FTS5 has no
/// `minimum_should_match`, so the count is how the same contract is expressed.
/// Either way the words are the query's *informative* ones: `the` never becomes
/// a clause of its own here.
fn word_query_gated(branch: &QueryBranch) -> Option<String> {
    let terms = &branch.content_terms;
    if terms.is_empty() {
        return None;
    }
    let joiner = if branch.word_min() >= terms.len() {
        " AND "
    } else {
        " OR "
    };
    Some(
        terms
            .iter()
            .map(|t| quote_term(t))
            .collect::<Vec<_>>()
            .join(joiner),
    )
}

/// FTS5's documented proximity query: all query terms within `NEAR_DISTANCE`
/// tokens of each other, in any order. This is the standard span-near tier
/// (Lucene `SpanNearQuery`, ES `span_near`), not a local invention.
///
/// Skipped for CJK — the trigram path already matches substrings, and `NEAR`
/// over trigram tokens is not meaningful — and for single-term queries, which
/// have nothing to be near. The terms are the query's informative ones in the
/// order they were written: an earlier version took the alphabetically-first
/// six, which meant a window that opened with `a` and `about` and never reached
/// the word the user cared about.
fn near_query(needle: &str) -> Option<String> {
    let branch = QueryBranch::parse(needle);
    if !branch.proximity_applies() {
        return None;
    }
    let list = branch
        .near_terms()
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
    fn chunked_hit_carries_row_coordinates() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        let head = "alpha ".repeat(30);
        let needle = "needle_literal";
        let tail = format!("prefix {needle} suffix");
        let cut = head.chars().count();
        let row_end = cut + tail.chars().count();
        conn.execute(
            "INSERT INTO rows(session_id, seq, chunk, single, char_start, char_end,
                              text_norm, text)
             VALUES ('s', 1, 0, 0, 0, ?1, ?2, ?3)",
            params![cut as i64, normalize(&head), head],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rows(session_id, seq, chunk, single, char_start, char_end,
                              text_norm, text)
             VALUES ('s', 1, 1, 0, ?1, ?2, ?3, ?4)",
            params![cut as i64, row_end as i64, normalize(&tail), tail],
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
        let hits = index.search(Lane::Like, needle, 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].chunk, 1);
        // The span is reported in row coordinates, not in the chunk's own: a
        // hit whose offset ignored `chunk_start` resolved to a line thousands
        // of chars above the match.
        let at = cut + tail.find(needle).unwrap();
        assert_eq!(hits[0].char_start, at, "{hits:?}");
        assert_eq!(hits[0].char_end, at + needle.chars().count(), "{hits:?}");
        // The snippet is cut from the chunk text, so it still holds the match.
        assert!(hits[0].summary.contains(needle), "{hits:?}");
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
    fn a_needle_below_the_trigram_floor_is_scanned_not_dropped() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        conn.execute(
            "INSERT INTO rows(session_id, seq, chunk, single, char_start, char_end,
                              text_norm, text)
             VALUES ('s', 1, 0, 1, 0, 0, ?1, ?2)",
            params![normalize("会话检索测试 alpha"), "会话检索测试 alpha"],
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
        // Two CJK chars have no trigram, so the index cannot serve them — the row
        // scan still answers, and at the exact-substring score.
        let hits = index.search(Lane::Like, "会话", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].score, SCORE_LIKE);
        assert_eq!(hits[0].seq, 1);
        assert!(index.contains(Lane::Like, "会话", "s", 1).unwrap());
        assert!(!index.contains(Lane::Like, "没戏", "s", 1).unwrap());
        // The product ladder reaches the same row, and a short Latin needle too.
        assert_eq!(index.search(Lane::Final, "会话", 10).unwrap().len(), 1);
        assert_eq!(index.search(Lane::Like, "al", 10).unwrap().len(), 1);
        assert!(index.search(Lane::Like, "没戏", 10).unwrap().is_empty());
    }

    #[test]
    fn near_query_is_proximity_over_word_terms() {
        // The window keeps the query's own order: the terms that matter are the
        // ones the user wrote first, not the alphabetically smallest.
        assert_eq!(
            near_query("session search").unwrap(),
            "NEAR(\"session\" \"search\", 10)"
        );
        // Glue is skipped before the window is spent.
        assert_eq!(
            near_query("the session of search").unwrap(),
            "NEAR(\"session\" \"search\", 10)"
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

    /// An in-memory index over `(session_id, seq, text, role)` rows.
    fn memory_index(rows: &[(&str, i64, &str, &str)]) -> SparseIndex {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        for (sid, seq, text, role) in rows {
            conn.execute(
                "INSERT INTO rows(session_id, seq, chunk, single, item_type, role,
                                  char_start, char_end, text_norm, text)
                 VALUES (?1, ?2, 0, 1, 'message', ?3, 0, ?4, ?5, ?6)",
                params![
                    sid,
                    seq,
                    role,
                    text.chars().count() as i64,
                    normalize(text),
                    text
                ],
            )
            .unwrap();
        }
        conn.execute_batch(FTS_DDL).unwrap();
        conn.execute_batch(
            "INSERT INTO tri(tri) VALUES('rebuild'); INSERT INTO uni(uni) VALUES('rebuild');",
        )
        .unwrap();
        SparseIndex {
            conn,
            path: PathBuf::from(":memory:"),
            scope: None,
        }
    }

    #[test]
    fn a_typo_is_found_by_the_fuzzy_layer_and_gated_by_adjacency() {
        let index = memory_index(&[("s", 1, "UNIQUE_SESSION_PHRASE is here", "conversation")]);
        let (hits, traces) = index.compose_sparse("UNIQUE_SESSION_PHRAZE", 10).unwrap();
        assert_eq!(hits.len(), 1, "a typo is what the fuzzy layer exists for: {traces:#?}");
        assert_eq!(hits[0].layers(), vec![LayerId::Fuzzy]);
        let fuzzy = traces
            .iter()
            .find(|t| t.layer == LayerId::Fuzzy)
            .expect("the fuzzy layer was the one that ran");
        assert_eq!(fuzzy.stop, StopReason::RecallShortfall, "{traces:#?}");
        assert_eq!(fuzzy.accepted, 1);
    }

    #[test]
    fn a_partial_fragment_is_not_typo_tolerance() {
        // Two words of a five-word question are not "nearly" the question.
        let index = memory_index(&[
            ("s", 1, "alpha beta only", "conversation"),
            ("s", 2, "alpha beta gamma delta epsilon", "conversation"),
        ]);
        let (hits, _) = index
            .compose_sparse("alpha beta gamma delta epsilon", 10)
            .unwrap();
        let rows: Vec<(String, i64)> = hits
            .iter()
            .map(|h| (h.session_id.clone(), h.seq))
            .collect();
        assert_eq!(rows.len(), 1, "only the row that covers the query");
        assert_eq!(rows[0].1, 2);
        assert_eq!(hits[0].coverage_of(LayerId::Lexical), Some(1.0));
    }

    #[test]
    fn a_row_that_only_shares_a_suffix_is_not_a_typo_match() {
        // The two rows share `_MARKER`; only one shares essentially the whole
        // query. Coverage alone cannot tell them apart — the unbroken fragment
        // can, which is why the fuzzy layer asks for one.
        let index = memory_index(&[
            ("s", 1, "ARCHIVED_OLD_MARKER buried before compact", "conversation"),
            ("s", 2, "LIVE_TAIL_MARKER still in window", "conversation"),
        ]);
        let (hits, _) = index.compose_sparse("LIVE_TAIL_MARKER", 10).unwrap();
        let rows: Vec<i64> = hits.iter().map(|h| h.seq).collect();
        assert_eq!(rows, vec![2], "{hits:#?}");
    }

    #[test]
    fn one_common_word_does_not_answer_a_multi_word_query() {
        let index = memory_index(&[
            ("s", 1, "the final answer", "conversation"),
            ("s", 2, "the auth refactor token", "conversation"),
            // One informative word of three: the OR query finds it, and the
            // coverage gate is what refuses to call it an answer.
            ("s", 3, "auth unrelated notes", "conversation"),
        ]);
        let branch = QueryBranch::parse("the auth refactor token");
        assert_eq!(branch.content_terms, ["auth", "refactor", "token"]);
        assert_eq!(branch.word_min(), 2);
        let query = word_query_gated(&branch).unwrap();
        assert_eq!(query, "\"auth\" OR \"refactor\" OR \"token\"");
        assert!(
            !query.contains("\"the\""),
            "glue is not a clause of its own: {query}"
        );

        let (hits, traces) = index
            .compose_sparse("the auth refactor token", 10)
            .unwrap();
        let rows: Vec<i64> = hits.iter().map(|h| h.seq).collect();
        assert_eq!(rows, vec![2], "{traces:#?}");
        assert_eq!(hits[0].rank.band, ranking::RankBand::Exact);
        assert_eq!(
            hits[0].layer_count(),
            4,
            "every mechanism found it, and the answer says so"
        );
        let lexical = traces
            .iter()
            .find(|t| t.layer == LayerId::Lexical)
            .expect("the lexical layer ran");
        assert!(
            lexical.rejected >= 1,
            "the one-word row is rejected, not ranked: {traces:#?}"
        );
    }

    /// The ladder itself, on four rows that are equal in everything but their
    /// tier: same literal, same length, found by the same layers.
    #[test]
    fn the_role_ladder_orders_equal_match_quality() {
        let index = memory_index(&[
            ("s", 1, "ladder probe aaaa", "outcome"),
            ("s", 2, "ladder probe bbbb", "action"),
            ("s", 3, "ladder probe cccc", "reasoning"),
            ("s", 4, "ladder probe dddd", "conversation"),
        ]);
        let (hits, _) = index.compose_sparse("ladder probe", 10).unwrap();
        let rows: Vec<i64> = hits.iter().map(|h| h.seq).collect();
        assert_eq!(rows, vec![4, 3, 2, 1], "说 → 思考 → 调用 → 结果");
        assert!(
            hits.windows(2).all(|pair| pair[0].rank.band == pair[1].rank.band
                && pair[0].rank.strength > pair[1].rank.strength),
            "one band, four tiers, reaching the order through the number: {hits:#?}"
        );
    }

    /// The tier ladder is a *preference*: at equal quality it decides, and it
    /// never overrules a real difference in quality.
    #[test]
    fn a_role_preference_yields_to_a_better_match() {
        // Both rows hold the literal once and cover every query word, so the only
        // thing between them is who said it. The `LIKE` length prior would put the
        // shorter row first; the tier is what moves it.
        let index = memory_index(&[
            ("s", 1, "auth refactor token here", "reasoning"),
            ("s", 2, "auth refactor token there", "conversation"),
        ]);
        let (hits, _) = index.compose_sparse("auth refactor token", 10).unwrap();
        let rows: Vec<i64> = hits.iter().map(|h| h.seq).collect();
        assert_eq!(rows, vec![2, 1], "what was said outranks what was thought");

        // A row that says the literal twice is more about it than a row that says
        // it once, so the better match wins from the lower tier.
        let index = memory_index(&[
            ("s", 1, "auth refactor token, and auth refactor token again", "reasoning"),
            ("s", 2, "auth refactor token", "conversation"),
        ]);
        let (hits, _) = index.compose_sparse("auth refactor token", 10).unwrap();
        let rows: Vec<i64> = hits.iter().map(|h| h.seq).collect();
        assert_eq!(rows, vec![1, 2], "a better match is not overruled by a tier");
    }

    #[test]
    fn a_common_literal_is_ranked_by_relevance_not_recency() {
        // Both rows contain the literal `session`; only one is about it. The
        // shortest-chunk prior plus the occurrence count is what the layer can
        // see, and recency is only the last tie-break.
        let mut rows: Vec<(String, i64, String, String)> = Vec::new();
        for seq in 0..250 {
            rows.push((
                "01AAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                seq,
                format!("session {seq} {}", "filler ".repeat(60)),
                "conversation".to_string(),
            ));
        }
        rows.push((
            "01ZZZZZZZZZZZZZZZZZZZZZZZZ".to_string(),
            999,
            "session search ranking session coverage".to_string(),
            "conversation".to_string(),
        ));
        let borrowed: Vec<(&str, i64, &str, &str)> = rows
            .iter()
            .map(|(sid, seq, text, role)| (sid.as_str(), *seq, text.as_str(), role.as_str()))
            .collect();
        let index = memory_index(&borrowed);
        let (hits, _) = index.compose_sparse("session", 10).unwrap();
        assert_eq!(hits.len(), 10, "the pool is still the caller's limit");
        assert_eq!(
            hits[0].session_id, "01ZZZZZZZZZZZZZZZZZZZZZZZZ",
            "the row that is *about* the literal wins over 250 newer rows that merely mention it"
        );
        assert!(hits[0].evidence[0].native >= 2.0, "occurrence count is the signal");
    }
}
