//! Session corpus ANN index under `.litecode/session-index/` (ANN-only).
//!
//! No BM25/CC/RRF here — lexical FTS lives in `sessions.db` on the always-on path.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::engines::code_search::{
    EMBED_DIM, Embedder, MODEL_ID, PIPELINE_VERSION, persist_usearch, production_embedder_id,
    restore_usearch,
};
use crate::types::{LitecodeError, Result};

use super::corpus::{self, SessionDoc};
use super::ranking::{ContentRole, HitEvidence, LayerId, RankBand, RankKey};
use super::slots::{Policy, SlotCfg};
use super::{SEMANTIC_WINDOW, SessionHitLane, SessionTextHit, echo};

use crate::session::SessionDataReader;

const EMBED_BATCH: usize = 32;
const SNIPPET_CHARS: usize = 200;
/// Bump when the dense document shape changes: 2 = chunked final corpus with
/// char ranges (was 0/1 = one document per raw row, no offsets).
const DOC_SCHEMA: u32 = 3;
/// Chunk budget of the dense corpus: the same hard cut the sparse lane uses,
/// kept under `EMBED_MAX_LENGTH` so no chunk is ever truncated by the embedder.
const DENSE_CHUNK_TOKENS: usize = 448;

fn dense_chunk_cfg() -> super::chunk::ChunkCfg {
    super::chunk::ChunkCfg {
        tokens: DENSE_CHUNK_TOKENS,
        // A split row also keeps a head+tail projection as an anchor vector: the
        // faithful chunks alone lose ranking, the anchor restores it.
        anchor: true,
    }
}

/// One embeddable unit: a chunk of the locked final corpus, or a split row's
/// head+tail anchor. `key` is the corpus document key (`sid:seq` / `sid:seq#k`)
/// and `char_start..char_end` is the range it covers inside the row's projected
/// text — the renderer turns that range into an `L<a>…L<b>` region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionChunk {
    id: u64,
    key: String,
    session_id: String,
    seq: i64,
    item_type: String,
    text: String,
    char_start: usize,
    char_end: usize,
    anchor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIndexMeta {
    pub model_id: String,
    pub embedder_id: String,
    pub pipeline_version: u32,
    pub embed_dim: usize,
    pub created_at: String,
    pub indexed_chunks: usize,
    #[serde(default)]
    pub last_change_id: i64,
    /// Dense document shape; 0/absent = the pre-chunking row-level corpus.
    #[serde(default)]
    pub doc_schema: u32,
    /// Hash of the live `(session_id, seq)` list at the last reconcile — the
    /// cheap half of "did anything move". A legacy file without it just pays one
    /// full reconcile, which stores it.
    #[serde(default)]
    pub keys_hash: u64,
}

impl SessionIndexMeta {
    fn shell(embedder_id: &str, indexed_chunks: usize) -> Self {
        Self {
            model_id: MODEL_ID.into(),
            embedder_id: embedder_id.into(),
            pipeline_version: PIPELINE_VERSION,
            embed_dim: EMBED_DIM,
            created_at: Utc::now().to_rfc3339(),
            indexed_chunks,
            last_change_id: 0,
            doc_schema: DOC_SCHEMA,
            keys_hash: 0,
        }
    }
}

fn session_index_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("session-index")
}

fn meta_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("meta.json")
}

fn vectors_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("vectors.usearch")
}

fn chunks_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("chunks.jsonl")
}

fn settled_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("source_state.jsonl")
}

/// One settled source row, as the dense corpus last derived it.
///
/// Kept so a reconcile can answer two questions without reading the corpus:
/// which rows are already settled (the key diff), and which calls read the
/// session store (the echo closure's seed). The sparse lane keeps the same facts
/// in its `source_state` table; the dense lane keeps its own, so neither lane
/// depends on the other having been built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SettledRow {
    session_id: String,
    seq: i64,
    /// The call this row is (a `tool_call`) or answers (a `tool_result`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    call_id: Option<String>,
    /// This row is itself a call into the session store.
    #[serde(default)]
    session_read_call: bool,
    /// Corpus document keys this row produced; empty for a row the policy drops.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    doc_keys: Vec<String>,
}

fn needs_rebuild(meta: &SessionIndexMeta) -> bool {
    meta.pipeline_version != PIPELINE_VERSION
        || meta.model_id != MODEL_ID
        || meta.embedder_id != production_embedder_id()
        || meta.embed_dim != EMBED_DIM
        || meta.doc_schema != DOC_SCHEMA
}

fn read_meta(workspace_root: &Path) -> Result<Option<SessionIndexMeta>> {
    let path = meta_path(workspace_root);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let meta: SessionIndexMeta = serde_json::from_str(&content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))?;
    Ok(Some(meta))
}

fn write_meta(workspace_root: &Path, meta: &SessionIndexMeta) -> Result<()> {
    let path = meta_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let body =
        serde_json::to_string_pretty(meta).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(&path, body).map_err(|e| LitecodeError::Config(e.to_string()))
}

fn new_ann_index() -> Result<Index> {
    let mut options = IndexOptions::default();
    options.dimensions = EMBED_DIM;
    options.metric = MetricKind::Cos;
    options.quantization = ScalarKind::BF16;
    Index::new(&options).map_err(|e| LitecodeError::Config(format!("session usearch new: {e}")))
}

pub struct SessionSemanticIndex {
    chunks: HashMap<u64, SessionChunk>,
    /// Corpus document key (`sid:seq` / `sid:seq#k`) → chunk id for reconcile.
    by_key: HashMap<String, u64>,
    ann: Index,
    next_id: u64,
    embedder_id: String,
    last_change_id: i64,
    /// Live key-list hash of the last reconcile; see [`hash_keys`].
    keys_hash: u64,
    /// Every source row this index has settled, with the linkage the echo rule
    /// needs. Empty on an index written before it existed, which is what makes
    /// the first reconcile a one-off full pass.
    settled: HashMap<(String, i64), SettledRow>,
}

impl SessionSemanticIndex {
    pub fn new_empty() -> Result<Self> {
        Ok(Self {
            chunks: HashMap::new(),
            by_key: HashMap::new(),
            ann: new_ann_index()?,
            next_id: 1,
            embedder_id: production_embedder_id().into(),
            last_change_id: 0,
            keys_hash: 0,
            settled: HashMap::new(),
        })
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let chunks_file = chunks_path(workspace_root);
        let ann = new_ann_index()?;
        restore_usearch(&ann, &vectors_path(workspace_root))?;

        let meta_on_disk = read_meta(workspace_root)?;
        let embedder_id = meta_on_disk
            .as_ref()
            .map(|m| m.embedder_id.clone())
            .unwrap_or_else(|| production_embedder_id().into());

        let mut index = Self {
            chunks: HashMap::new(),
            by_key: HashMap::new(),
            ann,
            next_id: 1,
            embedder_id,
            last_change_id: meta_on_disk.as_ref().map(|m| m.last_change_id).unwrap_or(0),
            keys_hash: meta_on_disk.as_ref().map(|m| m.keys_hash).unwrap_or(0),
            settled: HashMap::new(),
        };

        let file = File::open(&chunks_file).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.map_err(|e| LitecodeError::Config(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let chunk: SessionChunk = serde_json::from_str(&line)
                .map_err(|e| LitecodeError::Config(format!("parse session chunk: {e}")))?;
            let id = chunk.id;
            index.next_id = index.next_id.max(id + 1);
            index.by_key.insert(chunk.key.clone(), id);
            index.chunks.insert(id, chunk);
        }

        // A settled state that cannot be read is not a fatal index: dropping it
        // costs one full pass (which rewrites it), while failing the load would
        // cost the corpus its search.
        match read_settled(workspace_root) {
            Ok(settled) => index.settled = settled,
            Err(error) => tracing::warn!(error = %error, "session index settled state unreadable"),
        }
        Ok(index)
    }

    /// Publish everything this index holds.
    ///
    /// A reconcile writes only the artifacts whose own content moved, so a pass
    /// that settles rows without deriving documents does not rewrite the vectors.
    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        self.write_vectors(workspace_root)?;
        self.write_chunks(workspace_root)?;
        self.write_settled(workspace_root)?;
        self.save_meta(workspace_root)
    }

    fn write_vectors(&self, workspace_root: &Path) -> Result<()> {
        let dir = session_index_dir(workspace_root);
        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
        persist_usearch(&self.ann, &vectors_path(workspace_root))
    }

    fn write_chunks(&self, workspace_root: &Path) -> Result<()> {
        let chunks_file = chunks_path(workspace_root);
        let mut file =
            File::create(&chunks_file).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut ids: Vec<u64> = self.chunks.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let chunk = &self.chunks[&id];
            let line =
                serde_json::to_string(chunk).map_err(|e| LitecodeError::Config(e.to_string()))?;
            writeln!(file, "{line}").map_err(|e| LitecodeError::Config(e.to_string()))?;
        }
        Ok(())
    }

    /// Record the watermark without touching the corpus artifacts.
    fn save_meta(&self, workspace_root: &Path) -> Result<()> {
        write_meta(
            workspace_root,
            &SessionIndexMeta {
                last_change_id: self.last_change_id,
                keys_hash: self.keys_hash,
                ..SessionIndexMeta::shell(&self.embedder_id, self.chunks.len())
            },
        )
    }

    /// One line per settled row, ordered so a read-back is stable.
    fn write_settled(&self, workspace_root: &Path) -> Result<()> {
        let path = settled_path(workspace_root);
        let mut file = File::create(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut rows: Vec<&SettledRow> = self.settled.values().collect();
        rows.sort_by(|a, b| (&a.session_id, a.seq).cmp(&(&b.session_id, b.seq)));
        for row in rows {
            let line =
                serde_json::to_string(row).map_err(|e| LitecodeError::Config(e.to_string()))?;
            writeln!(file, "{line}").map_err(|e| LitecodeError::Config(e.to_string()))?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    fn remove_id(&mut self, id: u64) {
        if let Some(chunk) = self.chunks.remove(&id) {
            self.by_key.remove(&chunk.key);
            let _ = self.ann.remove(id);
        }
    }

    fn ann_add(&mut self, key: u64, vector: &[f32]) -> Result<()> {
        let needed = self.chunks.len() + 1;
        if self.ann.capacity() < needed {
            self.ann
                .reserve(needed.max(64))
                .map_err(|e| LitecodeError::Config(format!("session ann reserve: {e}")))?;
        }
        self.ann
            .add(key, vector)
            .map_err(|e| LitecodeError::Config(format!("session ann add: {e}")))?;
        Ok(())
    }

    fn embed_and_add(
        &mut self,
        chunks: Vec<SessionChunk>,
        embedder: &mut dyn Embedder,
    ) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let vectors = embedder.embed_batch(&texts)?;
        if chunks.len() != vectors.len() {
            return Err(LitecodeError::Config(
                "session chunks/vectors length mismatch".into(),
            ));
        }
        for (chunk, vec) in chunks.into_iter().zip(vectors) {
            self.ann_add(chunk.id, &vec)?;
            self.by_key.insert(chunk.key.clone(), chunk.id);
            self.chunks.insert(chunk.id, chunk);
        }
        Ok(())
    }

    fn ann_search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        if self.chunks.is_empty() {
            return Ok(Vec::new());
        }
        let results = self
            .ann
            .search(query, k.min(self.chunks.len()))
            .map_err(|e| LitecodeError::Config(format!("session ann search: {e}")))?;
        Ok(results
            .keys
            .iter()
            .zip(results.distances.iter())
            .map(|(&key, &dist)| (key, dist))
            .collect())
    }

    /// ANN-only → SessionTextHit list. Optional `session_id` filters after ANN.
    pub fn search(
        &self,
        query_vec: &[f32],
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<SessionTextHit>> {
        // Over-fetch when filtering so scoped queries still fill top_k.
        let fetch_k = if session_id.is_some() {
            (top_k.saturating_mul(8))
                .max(SEMANTIC_WINDOW)
                .min(self.chunks.len().max(1))
        } else {
            top_k.clamp(1, SEMANTIC_WINDOW)
        };
        let pairs = self.ann_search(query_vec, fetch_k)?;
        let mut hits = Vec::with_capacity(top_k);
        for (id, dist) in pairs {
            let Some(chunk) = self.chunks.get(&id) else {
                continue;
            };
            if let Some(sid) = session_id
                && chunk.session_id != sid
            {
                continue;
            }
            let summary: String = chunk.text.chars().take(SNIPPET_CHARS).collect();
            let local_rank = hits.len();
            let native = 1.0 / (1.0 + dist as f64);
            let role = ContentRole::from_item_type(&chunk.item_type);
            // No lexical nucleus — Related is rendered entry-level (not fake 0..N bold).
            hits.push(SessionTextHit {
                session_id: chunk.session_id.clone(),
                seq: chunk.seq,
                item_type: chunk.item_type.clone(),
                summary,
                score: native,
                // The chunk's own range: the renderer maps it to the row's
                // physical lines (`L<a>…L<b>`), so a semantic hit is a *region*.
                char_start: chunk.char_start,
                char_end: chunk.char_end,
                lane: SessionHitLane::Semantic,
                role,
                // One piece of evidence, stated in the terms the fusion stage
                // uses: this layer, this rank, this distance — never its score
                // against another lane's.
                evidence: vec![HitEvidence {
                    layer: LayerId::Semantic,
                    branch: 0,
                    matched: 1,
                    total: 1,
                    local_rank,
                    native,
                }],
                // The band is where the *fusion* may put it; the strength is
                // filled in by the fusion itself, from the rank, so that no
                // dense score is ever compared with a BM25 one.
                rank: RankKey {
                    band: RankBand::Fusion,
                    strength: 0,
                    role: role.preference(),
                    local_rank: local_rank as u32,
                },
            });
            if hits.len() >= top_k {
                break;
            }
        }
        Ok(hits)
    }

    /// Reconcile against the live store.
    ///
    /// Incremental by construction, the way the sparse lane is: the live key set
    /// is diffed against the rows this index has already settled, and only the
    /// delta is read, projected and embedded. The one cross-row relation the
    /// projection has — the echo closure — is resolved from the settled state
    /// instead of by re-deriving the corpus, which is what removes the full pass.
    ///
    /// The assumption, shared with the sparse lane: a settled row's derivation
    /// never changes, because final rows are immutable. A row either enters the
    /// settled set or leaves it, and both show up in the key diff.
    ///
    /// An index written before the settled state existed has no diff basis: its
    /// first pass derives the corpus once (reusing every vector whose text is
    /// unchanged, so nothing is re-embedded) and writes the state. Every later
    /// pass is a delta.
    pub fn reconcile(
        &mut self,
        reader: &SessionDataReader,
        workspace_root: &Path,
        embedder: &mut dyn Embedder,
    ) -> Result<bool> {
        // WP-135 M2: a `running` job.json must never outlive its run. The guard
        // marks this root in flight for stale-record recovery; any error inside
        // turns the record into `failed` instead of leaving `running` behind.
        let _in_flight = InFlightGuard::enter(workspace_root);
        match self.reconcile_inner(reader, workspace_root, embedder) {
            Ok(changed) => Ok(changed),
            Err(error) => {
                mark_session_job_failed(workspace_root, &error.to_string());
                Err(error)
            }
        }
    }

    fn reconcile_inner(
        &mut self,
        reader: &SessionDataReader,
        workspace_root: &Path,
        embedder: &mut dyn Embedder,
    ) -> Result<bool> {
        let live = reader.searchable_keys_blocking(None)?;
        let latest = reader.latest_change_id_blocking().unwrap_or(0);
        let keys_hash = hash_keys(&live);
        if self.last_change_id == latest && self.keys_hash == keys_hash {
            clear_session_job(workspace_root);
            write_session_queue_counts(workspace_root, 0, 0);
            return Ok(false);
        }

        let legacy = self.settled.is_empty() && !self.chunks.is_empty();
        let live_set: HashSet<(String, i64)> = live.iter().cloned().collect();
        let to_add: Vec<(String, i64)> = live
            .iter()
            .filter(|key| !self.settled.contains_key(*key))
            .cloned()
            .collect();
        let to_remove: Vec<(String, i64)> = self
            .settled
            .keys()
            .filter(|key| !live_set.contains(*key))
            .cloned()
            .collect();

        let settled_changed = !to_add.is_empty() || !to_remove.is_empty();
        let mut changed = false;
        let mut derived_keys: HashSet<String> = HashSet::new();
        // A row the corpus no longer holds gives its documents back, and its
        // settled state goes with them. The row said what it owned when it was
        // derived, so this needs no corpus read.
        for key in &to_remove {
            if let Some(previous) = self.settled.remove(key) {
                for doc_key in &previous.doc_keys {
                    changed |= self.remove_doc(doc_key);
                }
            }
        }

        // WP-135: scanning — denominator = the whole index, but the new rows
        // are not cut yet, so the total is unknown (None), never estimated.
        // `done` = kept chunks only (deleted rows' chunks are already gone
        // above), the same number `queue.json` reported as `kept_chunks`, so
        // `done` never steps down across queued → scanning (M1).
        write_session_running(workspace_root, self.chunks.len() as u64, None);

        if !to_add.is_empty() {
            let rows = reader.searchable_rows_for_blocking(&to_add)?;
            let mut staged: Vec<SessionChunk> = Vec::new();
            let tk = super::tokenizer::shared()?;
            let chunk_cfg = dense_chunk_cfg();
            let slot_cfg = SlotCfg::default();
            // The echo closure's seed: calls this index already settled as
            // session reads. Without it a result whose call arrived in an earlier
            // batch would be re-admitted as ordinary content.
            let known: HashSet<(String, String)> = self
                .settled
                .values()
                .filter(|row| row.session_read_call)
                .filter_map(|row| {
                    row.call_id
                        .as_ref()
                        .map(|call_id| (row.session_id.clone(), call_id.clone()))
                })
                .collect();
            let echo_keys = echo::result_keys_with(&rows, reader.data_root(), &known)?;

            // Dense whitelist (WP-130): Who/Said/Thought only — no Did/Outcome.
            // Echo removal, budget trim, then 448-token hard-cut (+ head+tail
            // anchor on splits). Chunk grid stays aligned with the sparse lane.
            let mut last_session = String::new();
            let mut last_tool: Option<String> = None;
            for row in &rows {
                if row.session_id != last_session {
                    last_session = row.session_id.clone();
                    last_tool = None;
                }
                let echo_excluded = echo_keys.contains(&(row.session_id.clone(), row.seq));
                let derived = corpus::derive_doc_row(
                    row,
                    reader.data_root(),
                    Policy::Final,
                    &slot_cfg,
                    &chunk_cfg,
                    Some(&tk),
                    echo_excluded,
                    &mut last_tool,
                )?;
                let previous = self
                    .settled
                    .get(&(row.session_id.clone(), row.seq))
                    .map(|settled| settled.doc_keys.clone())
                    .unwrap_or_default();
                if legacy {
                    derived_keys.extend(derived.docs.iter().map(|doc| doc.key.clone()));
                }
                changed |= self.stage_row_docs(&previous, &derived.docs, &mut staged);
                self.settled.insert(
                    (row.session_id.clone(), row.seq),
                    SettledRow {
                        session_id: row.session_id.clone(),
                        seq: row.seq,
                        call_id: derived.call_id,
                        session_read_call: derived.session_read_call,
                        doc_keys: derived.docs.iter().map(|doc| doc.key.clone()).collect(),
                    },
                );
            }
            // One embed call per batch, not per document: the delta is small but
            // the first pass over an existing index is not, and a session run per
            // chunk is the difference between minutes and seconds.
            // WP-135: cutting is done, so the total is exact: what the index
            // keeps plus every staged chunk. `done` advances per batch.
            let mut done = self.chunks.len() as u64;
            let total = done + staged.len() as u64;
            write_session_running(workspace_root, done, Some(total));
            for batch in staged.chunks(EMBED_BATCH) {
                self.embed_and_add(batch.to_vec(), embedder)?;
                done += batch.len() as u64;
                write_session_running(workspace_root, done, Some(total));
            }
        }

        if legacy {
            // The pre-`settled` index is not evidence of anything: this pass read
            // every row, so it alone decides which documents exist. Nothing the
            // old index holds survives unless it was just derived.
            let orphans: Vec<String> = self
                .by_key
                .keys()
                .filter(|key| !derived_keys.contains(*key))
                .cloned()
                .collect();
            for key in orphans {
                changed |= self.remove_doc(&key);
            }
        }

        self.last_change_id = latest;
        self.keys_hash = keys_hash;
        self.embedder_id = embedder.embedder_id().into();
        // Nothing derived is not nothing to record: the watermark is what retires
        // the pending hint, and rewriting the vectors for it would be the whole
        // corpus of work for none of the corpus of change.
        // Nothing derived is not nothing to record: a settled row that produced
        // no document (an echo copy, a policy drop) still has to reach disk, and
        // the watermark is what retires the pending hint. Rewriting the vectors
        // for either would be the whole corpus of work for none of the corpus of
        // change, so each artifact is written only when its own content moved.
        if changed {
            self.write_vectors(workspace_root)?;
            self.write_chunks(workspace_root)?;
        }
        if changed || settled_changed {
            self.write_settled(workspace_root)?;
        }
        self.save_meta(workspace_root)?;
        clear_session_job(workspace_root);
        write_session_queue_counts(workspace_root, 0, 0);
        Ok(changed)
    }

    /// Make the index hold exactly these documents for one row, staging the ones
    /// that need a vector for the caller to embed in batches.
    ///
    /// A document whose key and text are unchanged keeps the vector it already
    /// has: that is the whole difference between a reconcile and a rebuild. What
    /// is left is new content only, which is why this can be batched.
    fn stage_row_docs(
        &mut self,
        previous: &[String],
        docs: &[SessionDoc],
        staged: &mut Vec<SessionChunk>,
    ) -> bool {
        let mut changed = false;
        // A row's document set can shrink (a row that used to be split now fits
        // one chunk), so the keys it no longer claims are dropped first.
        let wanted: HashSet<&str> = docs.iter().map(|doc| doc.key.as_str()).collect();
        for key in previous {
            if !wanted.contains(key.as_str()) {
                changed |= self.remove_doc(key);
            }
        }
        for doc in docs {
            let unchanged = self
                .by_key
                .get(&doc.key)
                .and_then(|id| self.chunks.get(id))
                .is_some_and(|chunk| chunk.text == doc.text);
            if unchanged {
                continue;
            }
            self.remove_doc(&doc.key);
            let id = self.next_id;
            self.next_id += 1;
            staged.push(SessionChunk {
                id,
                key: doc.key.clone(),
                session_id: doc.session_id.clone(),
                seq: doc.seq,
                item_type: doc.item_type.clone(),
                text: doc.text.clone(),
                char_start: doc.chunk_start,
                char_end: doc.chunk_end,
                anchor: doc.anchor,
            });
            changed = true;
        }
        changed
    }

    /// Drop one document by key. `false` when it was not there.
    fn remove_doc(&mut self, key: &str) -> bool {
        let Some(id) = self.by_key.remove(key) else {
            return false;
        };
        self.remove_id(id);
        true
    }
}

/// Hash of the live key list.
///
/// `searchable_keys` is already `ORDER BY session_id, seq`, so the hash covers a
/// stable order and means the same thing across processes.
fn hash_keys(keys: &[(String, i64)]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (session_id, seq) in keys {
        session_id.hash(&mut hasher);
        seq.hash(&mut hasher);
    }
    hasher.finish()
}

/// Read the settled state, one row per line. `Ok(empty)` when it was never
/// written; an unreadable file is an error the caller may recover from.
fn read_settled(workspace_root: &Path) -> Result<HashMap<(String, i64), SettledRow>> {
    let path = settled_path(workspace_root);
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let file = File::open(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let mut settled = HashMap::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| LitecodeError::Config(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let row: SettledRow = serde_json::from_str(&line)
            .map_err(|e| LitecodeError::Config(format!("parse settled row: {e}")))?;
        settled.insert((row.session_id.clone(), row.seq), row);
    }
    Ok(settled)
}

fn index_files_exist(workspace_root: &Path) -> bool {
    vectors_path(workspace_root).is_file() && chunks_path(workspace_root).is_file()
}

// ---- WP-135: session job record -------------------------------------------
//
// Same storage as the code corpus (WP-134 `job.json` + chunk counters), kept in
// this corpus's own index dir. Exact counters only: `total == None` ⇔ the new
// rows are not cut yet (scanning). Nothing here is estimated.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SessionJobStatus {
    /// A reconcile is cutting / embedding.
    Running,
    /// The last reconcile failed; cleared by the next successful one.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SessionJobChunks {
    done: u64,
    #[serde(default)]
    total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionJobFile {
    status: SessionJobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chunks: Option<SessionJobChunks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn session_job_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("job.json")
}

fn read_session_job(workspace_root: &Path) -> Option<SessionJobFile> {
    let content = std::fs::read_to_string(session_job_path(workspace_root)).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write-to-temp-then-rename in the same directory, so a poll never reads a
/// half-written record.
fn write_json_atomic(path: &Path, value: &impl Serialize) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = std::fs::create_dir_all(parent);
    let Ok(body) = serde_json::to_string_pretty(value) else {
        return;
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("record");
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = parent.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn write_session_job(workspace_root: &Path, job: &SessionJobFile) {
    write_json_atomic(&session_job_path(workspace_root), job);
}

/// Roots with a reconcile running in this process (the worker that owns the
/// session index). A `running` record for a root not in here has no live
/// worker behind it.
static IN_FLIGHT: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

struct InFlightGuard(PathBuf);

impl InFlightGuard {
    fn enter(workspace_root: &Path) -> Self {
        let root = workspace_root.to_path_buf();
        if let Ok(mut roots) = IN_FLIGHT.lock() {
            roots.push(root.clone());
        }
        Self(root)
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut roots) = IN_FLIGHT.lock()
            && let Some(i) = roots.iter().position(|r| *r == self.0)
        {
            roots.swap_remove(i);
        }
    }
}

fn reconcile_in_flight(workspace_root: &Path) -> bool {
    IN_FLIGHT
        .lock()
        .map(|roots| roots.iter().any(|r| r == workspace_root))
        .unwrap_or(false)
}

/// M2: a `running` record left by a run that died (crash, kill) is cleared
/// when the worker process loads the index — the only process that runs
/// reconciles, so `IN_FLIGHT` is authoritative there. Never called from the
/// main-process tick path. Returns true when it cleared one; the caller's next
/// `queue_session_dirty` re-queues the work from the watermark, which the dead
/// run never advanced.
fn recover_stale_session_job(workspace_root: &Path) -> bool {
    let stale = matches!(
        read_session_job(workspace_root).map(|j| j.status),
        Some(SessionJobStatus::Running)
    ) && !reconcile_in_flight(workspace_root);
    if stale {
        tracing::warn!("session_search: clearing stale running job record");
        clear_session_job(workspace_root);
    }
    stale
}

fn clear_session_job(workspace_root: &Path) {
    let _ = std::fs::remove_file(session_job_path(workspace_root));
}

/// Exact pending-queue record, modelled on the code corpus's `CodeQueue`
/// (`code-index/queue.json`): same shape, same write/remove-at-zero rule, in
/// this corpus's own index dir. Units are sessions (the session corpus's
/// "files"): distinct sessions with change-log entries past the index
/// watermark. Never a chunk count: chunks exist only after cutting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionQueue {
    /// Sessions with changes past the watermark (including deletions).
    pub queued: usize,
    /// Sessions that may need embedding. Unit: sessions, never chunks or rows.
    /// The change log does not tell a pure deletion/truncation apart from an
    /// append without reading the rows, so today this counts every queued
    /// session (== `queued`), an upper bound of sessions, not an estimate of
    /// work size. Mirrors `CodeQueue::embed_dirty` (paths needing embedding).
    pub embed_dirty: usize,
    /// Chunks the index keeps once rows gone from the store are dropped: the
    /// exact `done` a refresh starts from (M1). Session-only field. `None` on
    /// a record written before the field existed: readers fall back to
    /// `meta.indexed_chunks` rather than 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_chunks: Option<u64>,
}

fn session_queue_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("queue.json")
}

fn write_session_queue_counts(workspace_root: &Path, queued: usize, embed_dirty: usize) {
    write_session_queue(
        workspace_root,
        SessionQueue {
            queued,
            embed_dirty,
            kept_chunks: None,
        },
    );
}

fn write_session_queue(workspace_root: &Path, queue: SessionQueue) {
    let path = session_queue_path(workspace_root);
    if queue.queued == 0 {
        let _ = std::fs::remove_file(&path);
        return;
    }
    write_json_atomic(&path, &queue);
}

/// Exact signal for status and the periodic tick: queued work on disk.
pub fn read_session_queue(workspace_root: &Path) -> SessionQueue {
    std::fs::read_to_string(session_queue_path(workspace_root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_session_running(workspace_root: &Path, done: u64, total: Option<u64>) {
    write_session_job(
        workspace_root,
        &SessionJobFile {
            status: SessionJobStatus::Running,
            chunks: Some(SessionJobChunks { done, total }),
            error: None,
        },
    );
}

fn mark_session_job_failed(workspace_root: &Path, error: &str) {
    let done = read_meta(workspace_root)
        .ok()
        .flatten()
        .map_or(0, |m| m.indexed_chunks as u64);
    write_session_job(
        workspace_root,
        &SessionJobFile {
            status: SessionJobStatus::Failed,
            chunks: Some(SessionJobChunks {
                done,
                total: Some(done),
            }),
            error: Some(error.to_string()),
        },
    );
}

/// The store has rows the index has not taken in yet (queued or in flight).
/// A flag, not a count: the exact count only exists once the rows are cut.
pub fn session_has_pending_work(workspace_root: &Path) -> bool {
    read_session_queue(workspace_root).queued > 0
        || matches!(
            read_session_job(workspace_root).map(|j| j.status),
            Some(SessionJobStatus::Running)
        )
}

/// Engine facts the session corpus needs, modelled on `CodeEngineFacts`.
/// Model readiness is reported by the code leg (one shared embedder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionEngineFacts {
    /// Persisted workspace intent (engine enabled).
    pub desired: bool,
    pub state: Option<crate::engines::EngineState>,
}

/// WP-135: this corpus's progress for `retrieval.session`, built only through
/// the WP-134 constructors.
pub fn session_corpus_progress(
    workspace_root: &Path,
    facts: SessionEngineFacts,
) -> crate::engines::progress::CorpusProgress {
    use crate::engines::EngineState;
    use crate::engines::progress::{CorpusError, CorpusProgress, FailReason, ProgressEngine};

    let indexed = read_meta(workspace_root)
        .ok()
        .flatten()
        .map_or(0, |m| m.indexed_chunks as u64);
    let fail = |reason: FailReason, message: String, done: u64| {
        CorpusProgress::failed(
            CorpusError::new(ProgressEngine::Session, reason, message),
            done,
        )
    };
    // L4: engine failures count only when desired, same order and rule as
    // `code_corpus_progress`. desired=false (user turned the engine off):
    // Stopped/Failed are not `failed`; the corpus reports its on-disk state
    // (ready / needs_rebuild / queued scanning) with no error.
    match facts.state.filter(|_| facts.desired) {
        Some(EngineState::Stopped) => {
            return fail(
                FailReason::EngineStopped,
                "search engine stopped".into(),
                indexed,
            );
        }
        Some(EngineState::Failed) => {
            return fail(
                FailReason::EngineUnavailable,
                "search engine unavailable".into(),
                indexed,
            );
        }
        _ => {}
    }
    // files_pending = exact queued sessions (`queue.json`); chunks_pending only
    // once rows are cut and the count is exact (no estimate from rows).
    let queue = read_session_queue(workspace_root);
    let files = queue.queued as u64;
    if let Some(job) = read_session_job(workspace_root) {
        match (job.status, job.chunks) {
            (SessionJobStatus::Failed, _) => {
                return fail(
                    FailReason::IndexFailed,
                    job.error.unwrap_or_else(|| "session index failed".into()),
                    indexed,
                );
            }
            (
                _,
                Some(SessionJobChunks {
                    done,
                    total: Some(total),
                }),
            ) => {
                return CorpusProgress::embedding(done, total, total.saturating_sub(done))
                    .with_files_pending(files);
            }
            // Not cut yet: total unknown, so nothing is claimed pending either
            // (`total == None` is the signal, WP-131 Q1).
            (_, Some(SessionJobChunks { done, total: None })) => {
                return CorpusProgress::scanning(done, 0).with_files_pending(files);
            }
            (_, None) => return CorpusProgress::scanning(indexed, 0).with_files_pending(files),
        }
    }
    if files > 0 && !session_should_rebuild(workspace_root) {
        // Queued, not cut yet ⇒ refreshing + scanning, total unknown (Q1).
        // done = kept chunks (M1): the refresh never reports fewer.
        let kept = queue.kept_chunks.unwrap_or(indexed);
        return CorpusProgress::scanning(kept, 0).with_files_pending(files);
    }
    match session_work_from_disk(workspace_root) {
        crate::engines::code_search::IndexWork::Rebuild { .. } => {
            CorpusProgress::needs_rebuild(indexed, indexed).with_files_pending(files)
        }
        _ => CorpusProgress::ready(indexed, 0).with_files_pending(files),
    }
}

/// Absent / unloadable / embedder mismatch — warmup auto-rebuilds this corpus only.
pub fn session_should_rebuild(workspace_root: &Path) -> bool {
    let meta = read_meta(workspace_root).ok().flatten();
    let files = index_files_exist(workspace_root);
    match meta {
        None => true,
        Some(m) => needs_rebuild(&m) || !files,
    }
}

pub fn session_index_status(workspace_root: &Path) -> crate::engines::code_search::IndexStatus {
    use crate::engines::code_search::IndexStatus;
    if session_should_rebuild(workspace_root) {
        return if index_files_exist(workspace_root) {
            IndexStatus::NeedsRebuild
        } else {
            IndexStatus::Absent
        };
    }
    if session_has_pending_work(workspace_root) {
        IndexStatus::Stale
    } else {
        IndexStatus::Ready
    }
}

/// Work as seen from disk alone: the job record plus the rebuild criteria.
///
/// The hint is written when an index is loaded or a reconcile finishes, so this
/// view can already be behind a store that has moved since. Callers that can see
/// the store must ask [`session_work_now`]; this one is for status display.
pub fn session_work_from_disk(workspace_root: &Path) -> crate::engines::code_search::IndexWork {
    use crate::engines::code_search::{IndexRebuildReason, IndexWork};
    if session_should_rebuild(workspace_root) {
        let has_vectors = index_files_exist(workspace_root);
        let has_db = workspace_root
            .join(".litecode")
            .join("sessions.db")
            .is_file();
        if !has_vectors && !has_db {
            return IndexWork::None;
        }
        return IndexWork::Rebuild {
            reason: if has_vectors {
                IndexRebuildReason::Incompatible
            } else {
                IndexRebuildReason::FirstDesired
            },
        };
    }
    // Exact queued sessions from `queue.json` (WP-135), never an estimate.
    let queue = read_session_queue(workspace_root);
    if queue.queued == 0 {
        IndexWork::None
    } else {
        IndexWork::Update {
            dirty: queue.queued,
        }
    }
}

/// Work as of *now*: the live store watermark against the index's, not whatever
/// hint some earlier loader happened to leave behind.
///
/// This is the question a caller asks before spending a reconcile, and the hint
/// file cannot answer it: a session written after the last consume writes no
/// hint, so a hint-only check leaves every later row unembedded until the
/// process restarts.
pub fn session_work_now(
    workspace_root: &Path,
    reader: &SessionDataReader,
) -> crate::engines::code_search::IndexWork {
    queue_session_dirty(workspace_root, reader);
    session_work_from_disk(workspace_root)
}

/// Load compatible vectors; empty shell when the library is absent/unloadable.
/// Does not embed or write the index.
pub fn load_session_index(workspace_root: &Path) -> Result<SessionSemanticIndex> {
    recover_stale_session_job(workspace_root);
    let dir = session_index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    if session_should_rebuild(workspace_root) {
        return SessionSemanticIndex::new_empty();
    }
    SessionSemanticIndex::load(workspace_root)
}

/// Compare `sessions.db` watermark to the on-disk session index and write the
/// exact queue record (`queue.json`): distinct sessions changed since the
/// watermark, read from the change log. Never a row- or id-delta estimate.
pub fn queue_session_dirty(workspace_root: &Path, reader: &SessionDataReader) {
    // A rebuild need is read from `meta.json` itself (`session_should_rebuild`).
    if session_should_rebuild(workspace_root) {
        return;
    }
    // No stale-job recovery here: this runs on the main-process tick, which
    // cannot see the worker's in-flight state (WP-135 re-review). Recovery
    // happens only when the worker loads the index (`load_session_index`).
    let latest = reader.latest_change_id_blocking().unwrap_or(0);
    let meta = read_meta(workspace_root).ok().flatten();
    let indexed = meta.as_ref().map_or(0, |m| m.last_change_id);
    let indexed_chunks = meta.as_ref().map_or(0, |m| m.indexed_chunks as u64);
    if latest == indexed {
        write_session_queue_counts(workspace_root, 0, 0);
        return;
    }
    let changes = match reader.change_log_since_blocking(indexed) {
        Ok(changes) => changes,
        Err(error) => {
            // L1: leave the previous record; the next pass retries.
            tracing::warn!(error = %error, "session_search: change log read failed; queue not updated");
            return;
        }
    };
    let sessions: HashSet<&str> = changes.iter().map(|c| c.session_id.as_str()).collect();
    write_session_queue(
        workspace_root,
        SessionQueue {
            queued: sessions.len(),
            embed_dirty: sessions.len(),
            kept_chunks: Some(kept_chunk_count(workspace_root, reader, indexed_chunks)),
        },
    );
}

/// Chunks the index keeps after dropping every settled row the store no longer
/// holds: exactly what `reconcile` removes first. Exact (settled doc keys), not
/// estimated; a legacy index without settled rows keeps `indexed_chunks`.
fn kept_chunk_count(workspace_root: &Path, reader: &SessionDataReader, indexed_chunks: u64) -> u64 {
    let (Ok(live), Ok(settled)) = (
        reader.searchable_keys_blocking(None),
        read_settled(workspace_root),
    ) else {
        return indexed_chunks;
    };
    if settled.is_empty() {
        return indexed_chunks;
    }
    let live: HashSet<(String, i64)> = live.into_iter().collect();
    let gone: HashSet<&String> = settled
        .iter()
        .filter(|(key, _)| !live.contains(*key))
        .flat_map(|(_, row)| row.doc_keys.iter())
        .collect();
    indexed_chunks.saturating_sub(gone.len() as u64)
}

/// Embed + save session drift. Wipe first when the library must rebuild.
pub fn consume_session_index(
    workspace_root: &Path,
    reader: &SessionDataReader,
    embedder: &mut dyn Embedder,
    index: &mut SessionSemanticIndex,
) -> Result<bool> {
    if session_should_rebuild(workspace_root) {
        tracing::info!("session_search rebuilding semantic index");
        // Only this lane's own marker: the directory also holds the sparse
        // lane's `sparse.db`, and deleting the directory took that index with
        // it, leaving a search to rebuild 282MB in its own thread. An absent
        // `meta.json` is what makes `session_should_rebuild` true, the stale
        // `chunks.jsonl` / `vectors.usearch` are never loaded without it
        // (`load_session_index`), and `save` overwrites both.
        let _ = std::fs::remove_file(meta_path(workspace_root));
        let dir = session_index_dir(workspace_root);
        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
        *index = SessionSemanticIndex::new_empty()?;
    }
    // `reconcile` itself turns any failure into a `failed` record (M2).
    index.reconcile(reader, workspace_root, embedder)
}

/// Test/helper: load (or empty) then consume against `sessions.db`.
pub fn ensure_session_index(
    workspace_root: &Path,
    reader: &SessionDataReader,
    embedder: &mut dyn Embedder,
) -> Result<SessionSemanticIndex> {
    let mut index = load_session_index(workspace_root)?;
    consume_session_index(workspace_root, reader, embedder, &mut index)?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::HashEmbedder;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::user_text;
    use tempfile::TempDir;

    #[test]
    fn session_index_round_trip_and_search() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        {
            let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            data.insert_items(&id, &[user_text("alpha session semantic marker omega")])
                .unwrap();
        }
        let reader = crate::session::SessionDataReader::open(&db);

        let mut emb = HashEmbedder;
        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
        assert!(!index.is_empty());

        let q = emb.embed_one("session semantic marker").unwrap();
        let hits = index.search(&q, 8, None).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].summary.contains("semantic marker"));

        index.save(root).unwrap();
        let loaded = SessionSemanticIndex::load(root).unwrap();
        assert_eq!(loaded.len(), index.len());
    }

    #[test]
    fn dense_corpus_chunks_long_rows_with_ranges_and_anchor() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        {
            let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data.create_session("/proj", "default", None).unwrap();
            // CJK prose: 1536 chars is still ≫448 tokens, so the row splits
            // (English prose gets budget-trimmed to one chunk first).
            let long = "这是一段很长的思考过程，需要按字符硬切，不能有任何字符丢失。".repeat(80);
            data.insert_items(&id, &[user_text(&long)]).unwrap();
        }
        let reader = crate::session::SessionDataReader::open(&db);
        let mut emb = HashEmbedder;
        let index = ensure_session_index(root, &reader, &mut emb).unwrap();

        let chunks: Vec<&SessionChunk> = index.chunks.values().collect();
        assert_eq!(
            chunks.iter().filter(|c| c.anchor).count(),
            1,
            "a split row keeps exactly one anchor"
        );
        let mut faithful: Vec<&SessionChunk> =
            chunks.iter().filter(|c| !c.anchor).copied().collect();
        assert!(faithful.len() > 1, "long row must split");
        for c in &chunks {
            assert!(c.char_end > c.char_start, "non-empty range: {c:?}");
        }
        // The faithful chunks tile the row's projected text.
        faithful.sort_by_key(|c| c.char_start);
        assert_eq!(faithful[0].char_start, 0);
        for pair in faithful.windows(2) {
            assert_eq!(pair[0].char_end, pair[1].char_start, "chunks must tile");
        }
        // Semantic hits carry that range (the renderer's `L<a>…L<b>` input).
        let q = emb.embed_one("retry backoff").unwrap();
        let hits = index.search(&q, 4, None).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.char_end > h.char_start));
    }

    #[test]
    fn session_index_reconcile_adds_new_rows() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("first row")]).unwrap();

        let mut emb = HashEmbedder;
        let reader = crate::session::SessionDataReader::open(&db);
        let mut index = ensure_session_index(root, &reader, &mut emb).unwrap();
        assert_eq!(index.len(), 1);

        data.insert_items(&id, &[user_text("second row")]).unwrap();
        let reloaded = load_session_index(root).unwrap();
        assert_eq!(reloaded.len(), 1, "load must not digest new session rows");
        queue_session_dirty(root, &reader);
        assert!(
            session_has_pending_work(root),
            "watermark lag must be queued, not embedded"
        );
        index.reconcile(&reader, root, &mut emb).unwrap();
        assert_eq!(index.len(), 2);
        assert!(!session_has_pending_work(root));
    }

    #[test]
    fn session_work_none_when_hint_cleared() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_session_queue_counts(root, 3, 3);
        assert_eq!(
            session_work_from_disk(root),
            crate::engines::code_search::IndexWork::None,
            "a job record without sessions.db or vectors is not engine work"
        );
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("seed")]).unwrap();
        let reader = crate::session::SessionDataReader::open(&db);
        let mut emb = HashEmbedder;
        let _ = ensure_session_index(root, &reader, &mut emb).unwrap();
        assert_eq!(
            session_work_from_disk(root),
            crate::engines::code_search::IndexWork::None
        );
        data.insert_items(&id, &[user_text("later")]).unwrap();
        queue_session_dirty(root, &reader);
        assert!(matches!(
            session_work_from_disk(root),
            crate::engines::code_search::IndexWork::Update { .. }
        ));
    }

    #[test]
    fn an_unchanged_store_is_not_reconciled_again() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("first row")]).unwrap();

        let mut emb = HashEmbedder;
        let reader = crate::session::SessionDataReader::open(&db);
        let mut index = ensure_session_index(root, &reader, &mut emb).unwrap();
        assert!(!session_has_pending_work(root));

        let dir = root.join(".litecode").join("session-index");
        let before = (
            std::fs::read(dir.join("chunks.jsonl")).unwrap(),
            std::fs::read(dir.join("meta.json")).unwrap(),
        );

        // Nothing moved: the second reconcile stops at the key list, so the
        // projection never runs and neither index file is touched.
        assert!(!index.reconcile(&reader, root, &mut emb).unwrap());
        assert_eq!(
            (
                std::fs::read(dir.join("chunks.jsonl")).unwrap(),
                std::fs::read(dir.join("meta.json")).unwrap(),
            ),
            before,
            "an unchanged store must not rewrite the index"
        );

        // The store moves: the same call picks the row up.
        data.insert_items(&id, &[user_text("second row")]).unwrap();
        assert!(index.reconcile(&reader, root, &mut emb).unwrap());
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn a_write_after_the_last_consume_is_work_without_a_hint() {
        use crate::engines::code_search::IndexWork;
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("first row")]).unwrap();

        let mut emb = HashEmbedder;
        let reader = crate::session::SessionDataReader::open(&db);
        let _ = ensure_session_index(root, &reader, &mut emb).unwrap();
        assert_eq!(
            session_work_now(root, &reader),
            IndexWork::None,
            "a consumed index is up to date"
        );

        data.insert_items(&id, &[user_text("later row")]).unwrap();
        assert!(
            !session_has_pending_work(root),
            "the writer never touches the job record, so nothing else would see this row"
        );
        assert!(
            matches!(session_work_now(root, &reader), IndexWork::Update { .. }),
            "the live watermark, not the hint file, decides the work"
        );
    }

    #[test]
    fn a_dense_rebuild_keeps_the_sparse_index() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("first row")]).unwrap();

        let reader = crate::session::SessionDataReader::open(&db);
        crate::engines::session_search::ensure_sparse_index(&reader).unwrap();
        let sparse = crate::engines::session_search::sparse_index_path(reader.data_root());
        let before = std::fs::read(&sparse).unwrap();

        // No meta.json yet, so this is the dense rebuild path: the one that used
        // to `remove_dir_all` the shared directory and take the sparse lane's
        // index with it, leaving the next search to rebuild it inline.
        let mut emb = HashEmbedder;
        let mut index = SessionSemanticIndex::new_empty().unwrap();
        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
        assert!(meta_path(root).is_file(), "the dense lane did rebuild");
        assert_eq!(
            std::fs::read(&sparse).unwrap(),
            before,
            "a dense rebuild must not touch the sparse lane's index"
        );
    }

    // ---- WP-135 progress -------------------------------------------------

    use crate::engines::EngineState;
    use crate::engines::progress::{CorpusPhase, CorpusProgress, CorpusState, FailReason};

    fn warm() -> SessionEngineFacts {
        SessionEngineFacts {
            desired: true,
            state: Some(EngineState::Warm),
        }
    }

    #[test]
    fn wp135_undesired_stopped_engine_is_not_failed() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for state in [EngineState::Stopped, EngineState::Failed] {
            let p = session_corpus_progress(
                root,
                SessionEngineFacts {
                    desired: false,
                    state: Some(state),
                },
            );
            assert_invariants(&p);
            assert_ne!(
                p.state,
                CorpusState::Failed,
                "user turned it off: {state:?}"
            );
            assert!(p.error.is_none());
        }
    }

    #[test]
    fn wp135_files_pending_is_exact_queued_sessions() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (data, id, db, _lease) = seeded(root, 2);
        let reader = crate::session::SessionDataReader::open(&db);
        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        assert_eq!(session_corpus_progress(root, warm()).files_pending, 0);
        let other = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("a")]).unwrap();
        data.insert_items(&id, &[user_text("b")]).unwrap();
        data.insert_items(&other, &[user_text("c")]).unwrap();
        queue_session_dirty(root, &reader);
        let p = session_corpus_progress(root, warm());
        assert_invariants(&p);
        assert_eq!(p.files_pending, 2, "two sessions changed, not three rows");
        assert_eq!(p.chunks_pending, 0, "nothing cut yet");
        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        assert_eq!(session_corpus_progress(root, warm()).files_pending, 0);
    }

    #[test]
    fn wp135_total_excludes_chunks_of_deleted_sessions() {
        use crate::session::data::command::{MutationId, SessionMutation};
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (data, keep, db, _lease) = seeded(root, 3);
        let gone = data.create_session("/proj", "default", None).unwrap();
        for i in 0..5 {
            data.insert_items(&gone, &[user_text(&format!("doomed {i}"))])
                .unwrap();
        }
        let reader = crate::session::SessionDataReader::open(&db);
        let mut index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        let before = index.len() as u64;
        let revision = reader
            .change_log_since_blocking(0)
            .unwrap()
            .iter()
            .filter(|c| c.session_id == gone)
            .map(|c| c.revision)
            .max()
            .unwrap();
        data.mutate_blocking(SessionMutation::Delete {
            session_id: gone.clone(),
            expected_revision: revision,
            operation_id: MutationId("wp135-delete".into()),
        })
        .unwrap();
        data.insert_items(&keep, &[user_text("one more")]).unwrap();
        queue_session_dirty(root, &reader);
        let queued = session_corpus_progress(root, warm());
        assert_eq!(queued.phase, Some(CorpusPhase::Scanning));
        assert!(
            queued.chunks_done < before,
            "queued done = kept chunks only"
        );

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut emb = ObservingEmbedder {
            root: root.to_path_buf(),
            seen: seen.clone(),
            fail: false,
        };
        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
        let seen = seen.lock().unwrap().clone();
        let end = session_corpus_progress(root, warm());
        assert!(index.len() as u64 > 0 && (index.len() as u64) < before);
        assert_eq!(end, CorpusProgress::ready(index.len() as u64, 0));
        for p in &seen {
            assert_eq!(
                p.chunks_total,
                Some(index.len() as u64),
                "total counts only live sessions' chunks"
            );
        }
        assert!(!seen.is_empty());

        // M1: deletion run is monotonic from the very first report.
        let mut run = vec![queued];
        run.extend(seen.iter().cloned());
        run.push(end);
        let mut last = 0;
        for p in &run {
            assert_invariants(p);
            assert!(p.chunks_done >= last, "done decreased: {run:?}");
            last = p.chunks_done;
        }
    }

    #[test]
    fn wp135_reconcile_failure_never_leaves_running() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_data, _id, db, _lease) = seeded(root, 3);
        let reader = crate::session::SessionDataReader::open(&db);
        let mut index = load_session_index(root).unwrap();
        let mut emb = ObservingEmbedder {
            root: root.to_path_buf(),
            seen: Default::default(),
            fail: true,
        };
        // Direct reconcile (not via consume): the failure is still recorded.
        assert!(index.reconcile(&reader, root, &mut emb).is_err());
        let job = read_session_job(root).unwrap();
        assert_eq!(job.status, SessionJobStatus::Failed);
        assert!(!reconcile_in_flight(root), "guard released on error");
        assert_eq!(
            session_corpus_progress(root, warm()).state,
            CorpusState::Failed
        );
    }

    #[test]
    fn wp135_stale_running_is_cleared_on_load_and_requeued() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (data, id, db, _lease) = seeded(root, 2);
        let reader = crate::session::SessionDataReader::open(&db);
        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        data.insert_items(&id, &[user_text("written before the crash")])
            .unwrap();
        // A run died mid-embedding: `running` on disk, no worker in flight.
        write_session_running(root, 1, Some(9));
        assert_eq!(
            session_corpus_progress(root, warm()).phase,
            Some(CorpusPhase::Embedding)
        );
        // Worker start-up / index load is the only place that recovers.
        let _ = load_session_index(root).unwrap();
        assert!(
            read_session_job(root).is_none(),
            "stale running cleared on worker load"
        );
        queue_session_dirty(root, &reader);
        assert_eq!(read_session_queue(root).queued, 1, "work re-queued");
        let p = session_corpus_progress(root, warm());
        assert_invariants(&p);
        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
        assert_eq!(p.files_pending, 1);
    }

    #[test]
    fn wp135_main_process_tick_does_not_touch_worker_running_job() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (data, id, db, _lease) = seeded(root, 2);
        let reader = crate::session::SessionDataReader::open(&db);
        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        // Worker (another process) is mid-run: `running` on disk, nothing in
        // this process's IN_FLIGHT — exactly what the main process sees.
        write_session_running(root, 2, Some(7));
        let before = std::fs::read(session_job_path(root)).unwrap();
        data.insert_items(&id, &[user_text("arrives mid-run")])
            .unwrap();
        // The main-process tick path: engines/mod.rs consume_session_index_work
        // → session_work_now → queue_session_dirty; plus status polls.
        for _ in 0..3 {
            let _ = session_work_now(root, &reader);
            let _ = session_work_from_disk(root);
            let _ = session_corpus_progress(root, warm());
        }
        assert_eq!(
            std::fs::read(session_job_path(root)).unwrap(),
            before,
            "tick must not change the worker's job.json"
        );
        assert_eq!(
            session_corpus_progress(root, warm()).phase,
            Some(CorpusPhase::Embedding)
        );
    }

    #[test]
    fn wp135_legacy_queue_without_kept_chunks_falls_back_to_indexed() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_data, _id, db, _lease) = seeded(root, 3);
        let reader = crate::session::SessionDataReader::open(&db);
        let index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        // A queue.json from before `kept_chunks` existed.
        std::fs::write(
            session_queue_path(root),
            r#"{"queued": 2, "embed_dirty": 2}"#,
        )
        .unwrap();
        let p = session_corpus_progress(root, warm());
        assert_invariants(&p);
        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
        assert_eq!(p.chunks_done, index.len() as u64, "not 0");
        assert_eq!(p.files_pending, 2);
    }

    #[test]
    fn wp135_live_running_is_not_cleared() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_session_running(root, 1, Some(9));
        let _guard = InFlightGuard::enter(root);
        assert!(!recover_stale_session_job(root));
        assert!(read_session_job(root).is_some());
    }

    #[test]
    fn wp135_records_are_written_atomically() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_session_running(root, 2, None);
        write_session_queue_counts(root, 3, 3);
        let names: Vec<String> = std::fs::read_dir(session_index_dir(root))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        assert!(read_session_job(root).is_some());
        assert_eq!(read_session_queue(root).queued, 3);
    }

    fn assert_invariants(p: &CorpusProgress) {
        assert_eq!(
            p.phase.is_some(),
            p.state == CorpusState::Refreshing,
            "{p:?}"
        );
        assert_eq!(
            p.chunks_total.is_none(),
            p.phase == Some(CorpusPhase::Scanning),
            "{p:?}"
        );
        if p.phase == Some(CorpusPhase::Embedding) {
            let total = p.chunks_total.unwrap();
            assert!(total > 0 && p.chunks_done <= total, "{p:?}");
        }
        assert_eq!(p.error.is_some(), p.state == CorpusState::Failed, "{p:?}");
    }

    fn seeded(
        root: &Path,
        rows: usize,
    ) -> (
        std::sync::Arc<SessionData>,
        String,
        std::path::PathBuf,
        WorkspaceWriteLease,
    ) {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        for i in 0..rows {
            data.insert_items(
                &id,
                &[user_text(&format!("row number {i} about topic {i}"))],
            )
            .unwrap();
        }
        (data, id, db, lease)
    }

    /// Records every progress the corpus reports while embedding, by peeking
    /// at the job record from inside the embedder (one call per batch).
    struct ObservingEmbedder {
        root: std::path::PathBuf,
        seen: std::sync::Arc<std::sync::Mutex<Vec<CorpusProgress>>>,
        fail: bool,
    }

    impl Embedder for ObservingEmbedder {
        fn embedder_id(&self) -> &'static str {
            HashEmbedder.embedder_id()
        }
        fn inference_device(&self) -> &'static str {
            "hash"
        }
        fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.seen
                .lock()
                .unwrap()
                .push(session_corpus_progress(&self.root, warm()));
            if self.fail {
                return Err(LitecodeError::Config("embed boom".into()));
            }
            HashEmbedder.embed_batch(texts)
        }
    }

    #[test]
    fn wp135_rebuild_reports_embedding_then_ready_exact() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_data, _id, db, _lease) = seeded(root, 80);
        let reader = crate::session::SessionDataReader::open(&db);
        assert_eq!(
            session_corpus_progress(root, warm()).state,
            CorpusState::NeedsRebuild
        );
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut emb = ObservingEmbedder {
            root: root.to_path_buf(),
            seen: seen.clone(),
            fail: false,
        };
        let index = ensure_session_index(root, &reader, &mut emb).unwrap();
        let seen = seen.lock().unwrap().clone();
        assert!(seen.len() >= 2, "80 rows must embed in several batches");
        let total = seen[0].chunks_total.unwrap();
        let mut last = 0;
        for p in &seen {
            assert_invariants(p);
            assert_eq!(p.phase, Some(CorpusPhase::Embedding));
            assert_eq!(p.chunks_total, Some(total), "exact total, fixed once cut");
            assert!(p.chunks_done >= last, "done is monotonic");
            assert_eq!(p.chunks_pending, total - p.chunks_done);
            last = p.chunks_done;
        }
        assert_eq!(seen[0].chunks_done, 0, "rebuild starts from an empty index");
        let done = session_corpus_progress(root, warm());
        assert_invariants(&done);
        assert_eq!(done, CorpusProgress::ready(total, 0));
        assert_eq!(index.len() as u64, total, "ready: done == total == indexed");
    }

    #[test]
    fn wp135_incremental_scans_then_embeds_from_kept_chunks() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (data, id, db, _lease) = seeded(root, 3);
        let reader = crate::session::SessionDataReader::open(&db);
        let mut index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        let kept = index.len() as u64;
        assert_eq!(
            session_corpus_progress(root, warm()),
            CorpusProgress::ready(kept, 0)
        );

        for i in 0..40 {
            data.insert_items(&id, &[user_text(&format!("later {i} news"))])
                .unwrap();
        }
        queue_session_dirty(root, &reader);
        // Q1: new rows not cut yet ⇒ refreshing + scanning, total None.
        let queued = session_corpus_progress(root, warm());
        assert_invariants(&queued);
        assert_eq!(
            queued,
            CorpusProgress::scanning(kept, 0).with_files_pending(1)
        );

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut emb = ObservingEmbedder {
            root: root.to_path_buf(),
            seen: seen.clone(),
            fail: false,
        };
        consume_session_index(root, &reader, &mut emb, &mut index).unwrap();
        let seen = seen.lock().unwrap().clone();
        assert!(!seen.is_empty());
        assert_eq!(seen[0].chunks_done, kept, "denominator = whole index");
        for p in &seen {
            assert_invariants(p);
            assert!(p.chunks_done >= kept);
        }
        let end = session_corpus_progress(root, warm());
        assert_eq!(end, CorpusProgress::ready(index.len() as u64, 0));
        assert_eq!(end.chunks_total, seen[0].chunks_total);

        // Whole run, pass 1 (queued/scanning) → pass 2 (embedding) → ready:
        // done never decreases, done ≤ total whenever total is known, and
        // done == total at the end.
        let mut run = vec![queued.clone()];
        run.extend(seen.iter().cloned());
        run.push(end.clone());
        let mut last = 0;
        for p in &run {
            assert_invariants(p);
            assert!(p.chunks_done >= last, "done decreased: {run:?}");
            if let Some(total) = p.chunks_total {
                assert!(p.chunks_done <= total, "{p:?}");
            }
            last = p.chunks_done;
        }
        assert_eq!(run[0].phase, Some(CorpusPhase::Scanning));
        assert!(
            run[1..run.len() - 1]
                .iter()
                .all(|p| p.phase == Some(CorpusPhase::Embedding))
        );
        assert_eq!(Some(end.chunks_done), end.chunks_total);
    }

    #[test]
    fn wp135_reconcile_writes_scanning_before_cutting() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_data, _id, db, _lease) = seeded(root, 2);
        let reader = crate::session::SessionDataReader::open(&db);
        let index = load_session_index(root).unwrap();
        // The scanning record is what a poll sees while rows are being cut.
        write_session_running(root, index.len() as u64, None);
        let p = session_corpus_progress(root, warm());
        assert_invariants(&p);
        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
        assert_eq!(p.chunks_total, None);
        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        assert_eq!(
            session_corpus_progress(root, warm()).state,
            CorpusState::Ready
        );
    }

    #[test]
    fn wp135_index_error_is_failed_index_failed_and_clears_on_success() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_data, _id, db, _lease) = seeded(root, 3);
        let reader = crate::session::SessionDataReader::open(&db);
        let mut emb = ObservingEmbedder {
            root: root.to_path_buf(),
            seen: Default::default(),
            fail: true,
        };
        assert!(ensure_session_index(root, &reader, &mut emb).is_err());
        let p = session_corpus_progress(root, warm());
        assert_invariants(&p);
        assert_eq!(p.state, CorpusState::Failed);
        let err = p.error.unwrap();
        assert_eq!(
            err.engine,
            crate::engines::progress::ProgressEngine::Session
        );
        assert_eq!(err.reason, FailReason::IndexFailed);
        assert!(err.message.contains("embed boom"));
        assert!(!session_has_pending_work(root), "failed is not pending");

        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        assert_eq!(
            session_corpus_progress(root, warm()).state,
            CorpusState::Ready
        );
    }

    #[test]
    fn wp135_engine_state_maps_to_fail_reason() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for (state, reason) in [
            (EngineState::Stopped, FailReason::EngineStopped),
            (EngineState::Failed, FailReason::EngineUnavailable),
        ] {
            let p = session_corpus_progress(
                root,
                SessionEngineFacts {
                    desired: true,
                    state: Some(state),
                },
            );
            assert_invariants(&p);
            assert_eq!(p.state, CorpusState::Failed);
            assert_eq!(p.error.unwrap().reason, reason);
        }
        for state in [None, Some(EngineState::Idle), Some(EngineState::Warming)] {
            let p = session_corpus_progress(
                root,
                SessionEngineFacts {
                    desired: true,
                    state,
                },
            );
            assert_invariants(&p);
            assert_eq!(
                p,
                CorpusProgress::ready(0, 0),
                "no history is ready, not work"
            );
        }
    }

    #[test]
    fn wp135_no_pending_hint_file_is_written() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (data, id, db, _lease) = seeded(root, 2);
        let reader = crate::session::SessionDataReader::open(&db);
        let _ = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
        data.insert_items(&id, &[user_text("more")]).unwrap();
        queue_session_dirty(root, &reader);
        assert!(
            !session_index_dir(root).join("pending_hint.json").exists(),
            "the estimate source is gone"
        );
        let queue = read_session_queue(root);
        assert_eq!(
            (queue.queued, queue.embed_dirty),
            (1, 1),
            "exact sessions, not a chunk or change-id count"
        );
        assert_eq!(
            queue.kept_chunks.unwrap(),
            read_meta(root).unwrap().unwrap().indexed_chunks as u64
        );
        let p = session_corpus_progress(root, warm());
        assert_eq!(p.chunks_total, None, "no total before cutting");
        assert_eq!(p.chunks_pending, 0, "no chunk estimate from rows");
    }
}
