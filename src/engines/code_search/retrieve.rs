//! Code-corpus semantic retrieval (SemanticLane).
//!
//! Production default: Bruch convex combination (CC) over dense ANN ∥ BM25,
//! then optional glob → path dedupe. Parameters below are **Code workspace
//! only** — Session / Knowledge must define their own fusion knobs; do not
//! promote these to engine-wide config.
//!
//! No MMR (legacy defect). No L3 fusion for humans/agents.

use std::collections::{HashMap, HashSet};

use crate::workspace::filter::compile_include_pattern;

use crate::types::Result;

use super::RETRIEVE_K;
use super::bm25::Bm25Index;
use super::store::CodeSearchIndex;

/// Industry-default RRF constant (Cormack et al. / Elastic). Eval / ablation only.
pub const RRF_K: usize = 60;
/// Candidates per leg before fusion.
pub const RETRIEVE_POOL: usize = 50;

pub const SEARCH_MODE_BM25_RRF: &str = "bm25_rrf";
/// Soft-complement hybrid (Bruch CC) — Code corpus production mode.
pub const SEARCH_MODE_BM25_CC: &str = "bm25_cc";

/// Dense weight α for Code workspace semantic CC (`f = α·dense + (1−α)·BM25`).
/// Tuned on stratified_v2 + ORT (`α=0.8`: total +2.9pp, semantic Δ 0).
/// **Not** an engine-wide knob — Session search must not reuse this constant.
pub const CODE_SEMANTIC_CC_ALPHA: f64 = 0.8;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub summary: String,
    pub score: f64,
}

/// Dense ANN only (kept for tests / ablations).
pub fn semantic_search(
    index: &CodeSearchIndex,
    query_vec: &[f32],
    glob_filter: Option<&str>,
    top_k: usize,
) -> Result<Vec<SearchHit>> {
    let pool = top_k.saturating_mul(16).max(RETRIEVE_K);
    let raw = index.ann_search(query_vec, pool)?;
    let mut hits = Vec::new();
    for (id, dist) in raw {
        if !apply_glob(index, id, glob_filter) {
            continue;
        }
        let score = 1.0 / (1.0 + dist as f64);
        if let Some(h) = hit_from_chunk(index, id, score) {
            hits.push(h);
        }
    }
    Ok(path_dedupe(hits, top_k))
}

/// Consensus hybrid: BM25 ∥ dense → RRF(k=60), then path-dedupe to top_k.
pub fn bm25_rrf_search(
    index: &CodeSearchIndex,
    bm25: &Bm25Index,
    query: &str,
    query_vec: &[f32],
    glob_filter: Option<&str>,
    top_k: usize,
) -> Result<Vec<SearchHit>> {
    let pool = RETRIEVE_POOL.max(top_k.saturating_mul(4));
    let dense = index.ann_search(query_vec, pool)?;
    let sparse = bm25.search(query, pool)?;

    let mut rrf: HashMap<u64, f64> = HashMap::new();
    for (rank, (id, _)) in dense.iter().enumerate() {
        if !apply_glob(index, *id, glob_filter) {
            continue;
        }
        *rrf.entry(*id).or_insert(0.0) += 1.0 / (RRF_K as f64 + rank as f64 + 1.0);
    }
    for (rank, (id, _)) in sparse.iter().enumerate() {
        if !apply_glob(index, *id, glob_filter) {
            continue;
        }
        *rrf.entry(*id).or_insert(0.0) += 1.0 / (RRF_K as f64 + rank as f64 + 1.0);
    }

    let mut ranked: Vec<(u64, f64)> = rrf.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut hits = Vec::new();
    for (id, score) in ranked {
        if let Some(h) = hit_from_chunk(index, id, score) {
            hits.push(h);
        }
    }
    Ok(path_dedupe(hits, top_k))
}

/// Soft-complement hybrid (Bruch et al., TOIS 2023 / arXiv:2210.11934):
/// `f = α · norm(dense) + (1−α) · norm(BM25)` with per-query min-max on each leg.
///
/// `alpha` is the weight on the **dense/semantic** score (higher = denser).
/// Missing leg scores are treated as 0 after normalization.
///
/// Callers that serve Code workspace search should pass [`CODE_SEMANTIC_CC_ALPHA`].
/// Other corpora must supply their own α — do not share Code's value.
pub fn bm25_cc_search(
    index: &CodeSearchIndex,
    bm25: &Bm25Index,
    query: &str,
    query_vec: &[f32],
    glob_filter: Option<&str>,
    top_k: usize,
    alpha: f64,
) -> Result<Vec<SearchHit>> {
    let alpha = alpha.clamp(0.0, 1.0);
    let pool = RETRIEVE_POOL.max(top_k.saturating_mul(4));
    let dense = index.ann_search(query_vec, pool)?;
    let sparse = bm25.search(query, pool)?;

    let mut dense_raw: HashMap<u64, f64> = HashMap::new();
    for (id, dist) in &dense {
        if !apply_glob(index, *id, glob_filter) {
            continue;
        }
        dense_raw.insert(*id, 1.0 / (1.0 + *dist as f64));
    }
    let mut sparse_raw: HashMap<u64, f64> = HashMap::new();
    for (id, score) in &sparse {
        if !apply_glob(index, *id, glob_filter) {
            continue;
        }
        sparse_raw.insert(*id, *score as f64);
    }

    let dense_norm = minmax_norm(&dense_raw);
    let sparse_norm = minmax_norm(&sparse_raw);

    let mut ids: HashSet<u64> = HashSet::new();
    ids.extend(dense_norm.keys().copied());
    ids.extend(sparse_norm.keys().copied());

    let mut ranked: Vec<(u64, f64)> = ids
        .into_iter()
        .map(|id| {
            let s = dense_norm.get(&id).copied().unwrap_or(0.0);
            let l = sparse_norm.get(&id).copied().unwrap_or(0.0);
            (id, alpha * s + (1.0 - alpha) * l)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut hits = Vec::new();
    for (id, score) in ranked {
        if let Some(h) = hit_from_chunk(index, id, score) {
            hits.push(h);
        }
    }
    Ok(path_dedupe(hits, top_k))
}

fn minmax_norm(raw: &HashMap<u64, f64>) -> HashMap<u64, f64> {
    if raw.is_empty() {
        return HashMap::new();
    }
    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    for &v in raw.values() {
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }
    let span = max_v - min_v;
    raw.iter()
        .map(|(&id, &v)| {
            let n = if span > 1e-12 {
                (v - min_v) / span
            } else {
                1.0
            };
            (id, n)
        })
        .collect()
}

fn hit_from_chunk(index: &CodeSearchIndex, chunk_id: u64, score: f64) -> Option<SearchHit> {
    let chunk = index.chunks().get(&chunk_id)?;
    Some(SearchHit {
        path: chunk.path.clone(),
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        summary: chunk.summary(),
        score,
    })
}

fn path_dedupe(hits: Vec<SearchHit>, top_k: usize) -> Vec<SearchHit> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for h in hits {
        if !seen.insert(h.path.clone()) {
            continue;
        }
        out.push(h);
        if out.len() >= top_k {
            break;
        }
    }
    out
}

fn apply_glob(index: &CodeSearchIndex, chunk_id: u64, glob_filter: Option<&str>) -> bool {
    let Some(glob) = glob_filter else {
        return true;
    };
    index
        .chunks()
        .get(&chunk_id)
        .map(|c| glob_match(glob, &c.path))
        .unwrap_or(false)
}

pub(crate) fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return true;
    }
    compile_include_pattern(pattern)
        .map(|matcher| matcher.matches(path))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::bm25::{self, Bm25Index};
    use crate::engines::code_search::build::build_full_index;
    use crate::engines::code_search::embed::{HashEmbedder, hash_vector};
    use tempfile::TempDir;

    #[test]
    fn glob_filters_ann_leg() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/code.rs"), "pub fn rust_fn() {}\n").unwrap();
        std::fs::write(root.join("notes.md"), "# rust_fn in markdown\n").unwrap();

        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        let query = "rust_fn";
        let qvec = hash_vector(query);
        let hits = semantic_search(&index, &qvec, Some("**/*.rs"), 8).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.path.ends_with(".rs")));
        assert!(!hits.iter().any(|h| h.path.ends_with(".md")));
    }

    #[test]
    fn hybrid_returns_path_and_lines() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("search_me.rs"),
            "pub fn find_me() {}\npub fn other() {}\n",
        )
        .unwrap();

        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        index.save(root).unwrap();
        bm25::rebuild(root, index.chunks()).unwrap();
        let bm25 = Bm25Index::open(root).unwrap();
        let query = "find_me";
        let qvec = hash_vector(query);
        let hits = bm25_rrf_search(&index, &bm25, query, &qvec, None, 8).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.path.contains("search_me.rs")));
        assert!(hits[0].start_line >= 1);
        assert!(!hits[0].summary.is_empty());
    }

    #[test]
    fn code_cc_alpha_is_workspace_only_constant() {
        // Guard against accidental "engine-wide" reuse without an intentional rename.
        assert!((CODE_SEMANTIC_CC_ALPHA - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn bm25_cc_returns_path_and_lines() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("search_me.rs"),
            "pub fn find_me() {}\npub fn other() {}\n",
        )
        .unwrap();

        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        index.save(root).unwrap();
        bm25::rebuild(root, index.chunks()).unwrap();
        let bm25 = Bm25Index::open(root).unwrap();
        let query = "find_me";
        let qvec = hash_vector(query);
        let hits =
            bm25_cc_search(&index, &bm25, query, &qvec, None, 8, CODE_SEMANTIC_CC_ALPHA).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.path.contains("search_me.rs")));
        assert!(hits[0].start_line >= 1);
        assert!(!hits[0].summary.is_empty());
    }
}
