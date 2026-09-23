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

use super::{SEMANTIC_WINDOW, SessionHitLane, SessionTextHit, echo};
use super::corpus::{self, SessionDoc};
use super::slots::{Policy, SlotCfg};

use crate::session::SessionDataReader;

const EMBED_BATCH: usize = 32;
const SNIPPET_CHARS: usize = 200;
/// Bump when the dense document shape changes: 2 = chunked final corpus with
/// char ranges (was 0/1 = one document per raw row, no offsets).
const DOC_SCHEMA: u32 = 2;
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
            // No lexical nucleus — Related is rendered entry-level (not fake 0..N bold).
            hits.push(SessionTextHit {
                session_id: chunk.session_id.clone(),
                seq: chunk.seq,
                item_type: chunk.item_type.clone(),
                summary,
                score: 1.0 / (1.0 + dist as f64),
                // The chunk's own range: the renderer maps it to the row's
                // physical lines (`L<a>…L<b>`), so a semantic hit is a *region*.
                char_start: chunk.char_start,
                char_end: chunk.char_end,
                lane: SessionHitLane::Semantic,
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
        let live = reader.searchable_keys_blocking(None)?;
        let latest = reader.latest_change_id_blocking().unwrap_or(0);
        let keys_hash = hash_keys(&live);
        if self.last_change_id == latest && self.keys_hash == keys_hash {
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

            // The dense corpus is the locked final policy: slot projection (人话 +
            // 工具调用 + 工具产出，压缩总结剔除), echo removal, budget trim, then
            // 448-token hard-cut chunks with a head+tail anchor for split rows —
            // the same grid the sparse lane uses, so both agree on coordinates.
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
            for batch in staged.chunks(EMBED_BATCH) {
                self.embed_and_add(batch.to_vec(), embedder)?;
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
        write_session_pending_hint(workspace_root, 0);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionPendingHintFile {
    pending_updates: usize,
}

fn session_pending_hint_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("pending_hint.json")
}

pub fn write_session_pending_hint(workspace_root: &Path, pending_updates: usize) {
    let path = session_pending_hint_path(workspace_root);
    if pending_updates == 0 {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string_pretty(&SessionPendingHintFile { pending_updates }) {
        let _ = std::fs::write(&path, body);
    }
}

pub fn read_session_pending_hint(workspace_root: &Path) -> usize {
    let Ok(content) = std::fs::read_to_string(session_pending_hint_path(workspace_root)) else {
        return 0;
    };
    serde_json::from_str::<SessionPendingHintFile>(&content)
        .map(|h| h.pending_updates)
        .unwrap_or(0)
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
    if read_session_pending_hint(workspace_root) > 0 {
        IndexStatus::Stale
    } else {
        IndexStatus::Ready
    }
}

/// Work as seen from disk alone: the pending hint plus the rebuild criteria.
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
    let dirty = read_session_pending_hint(workspace_root);
    if dirty == 0 {
        IndexWork::None
    } else {
        IndexWork::Update { dirty }
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
    let dir = session_index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    if session_should_rebuild(workspace_root) {
        return SessionSemanticIndex::new_empty();
    }
    SessionSemanticIndex::load(workspace_root)
}

/// Compare `sessions.db` watermark to the on-disk session index; write hint only.
pub fn queue_session_dirty(workspace_root: &Path, reader: &SessionDataReader) {
    if session_should_rebuild(workspace_root) {
        write_session_pending_hint(workspace_root, 1);
        return;
    }
    let latest = reader.latest_change_id_blocking().unwrap_or(0);
    let indexed = read_meta(workspace_root)
        .ok()
        .flatten()
        .map(|m| m.last_change_id)
        .unwrap_or(0);
    if latest == indexed {
        write_session_pending_hint(workspace_root, 0);
        return;
    }
    let dirty = latest.abs_diff(indexed).max(1) as usize;
    write_session_pending_hint(workspace_root, dirty);
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
    let dirty = index.reconcile(reader, workspace_root, embedder)?;
    write_session_pending_hint(workspace_root, 0);
    Ok(dirty)
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
            read_session_pending_hint(root) > 0,
            "watermark lag must be queued, not embedded"
        );
        index.reconcile(&reader, root, &mut emb).unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(read_session_pending_hint(root), 0);
    }

    #[test]
    fn session_work_none_when_hint_cleared() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write_session_pending_hint(root, 3);
        assert_eq!(
            session_work_from_disk(root),
            crate::engines::code_search::IndexWork::None,
            "hint without sessions.db or vectors is not engine work"
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
        assert_eq!(read_session_pending_hint(root), 0);

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
        assert_eq!(
            read_session_pending_hint(root),
            0,
            "the writer never touches the hint, so nothing else would see this row"
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
}
