//! Session corpus ANN index under `.litecode/session-index/` (ANN-only).
//!
//! No BM25/CC/RRF here — lexical FTS lives in `sessions.db` on the always-on path.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::engines::code_search::{
    AtomicPart, EMBED_DIM, Embedder, MODEL_ID, PIPELINE_VERSION, append_durable,
    atomic_write_bytes, atomic_write_set, production_embedder_id, restore_usearch,
    sweep_atomic_tmp, truncate_to_committed,
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
    /// WP-144 on-disk layout: 1 = append-only `vectors.log` / `chunks.jsonl` /
    /// `source_state.jsonl` with this meta as the checkpoint. 0/absent = the
    /// old whole-file layout, rebuilt once (resumably) on upgrade.
    #[serde(default)]
    pub layout: u32,
    /// Committed byte lengths of the three append logs. A resume truncates each
    /// log back to these, so anything after the last checkpoint is redone.
    #[serde(default)]
    pub vectors_len: u64,
    #[serde(default)]
    pub chunks_len: u64,
    #[serde(default)]
    pub settled_len: u64,
    /// A first build / rebuild is in flight: committed batches are searchable,
    /// the watermark has not advanced yet.
    #[serde(default)]
    pub building: bool,
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
            layout: LAYOUT,
            vectors_len: 0,
            chunks_len: 0,
            settled_len: 0,
            building: false,
        }
    }
}

/// See [`SessionIndexMeta::layout`].
const LAYOUT: u32 = 1;
/// One vector record in `vectors.log`: chunk id (u64 LE) + EMBED_DIM f32 LE.
const VECTOR_RECORD: usize = 8 + 4 * EMBED_DIM;

fn session_index_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("session-index")
}

fn meta_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("meta.json")
}

/// Append-only raw vector log (WP-144). usearch can only save whole, so the
/// durable form is this log and the ANN graph is rebuilt in RAM on load.
fn vectors_path(workspace_root: &Path) -> PathBuf {
    session_index_dir(workspace_root).join("vectors.log")
}

/// Pre-WP-144 whole-file ANN dump; removed when a rebuild starts.
fn legacy_vectors_path(workspace_root: &Path) -> PathBuf {
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
        || meta.layout != LAYOUT
}

fn read_meta(workspace_root: &Path) -> Result<Option<SessionIndexMeta>> {
    let path = meta_path(workspace_root);
    if !path.exists() {
        return Ok(None);
    }
    #[cfg(test)]
    if FAIL_META_READS.with(|c| c.get()) {
        return Err(LitecodeError::Config(
            "io: injected meta read failure".into(),
        ));
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let meta: SessionIndexMeta = serde_json::from_str(&content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))?;
    Ok(Some(meta))
}

/// The checkpoint: replaced whole and atomically, after the logs it describes
/// are appended and fsynced.
fn write_meta(workspace_root: &Path, meta: &SessionIndexMeta) -> Result<()> {
    let body = serde_json::to_vec_pretty(meta).map_err(|e| LitecodeError::Config(e.to_string()))?;
    atomic_write_bytes(&meta_path(workspace_root), &body)
}

// ---- WP-144 persistence: IDE's WP-143 atomic/append API ------------------
//
// Logs go through `append_durable` (cuts any uncommitted tail first, M2),
// whole files through `atomic_write_bytes`/`atomic_write_set`, temp leftovers
// through `sweep_atomic_tmp`, load-time truncation through
// `truncate_to_committed`. When the checkpoint cannot be honoured (a short
// file, a torn line), `salvage_log` keeps the longest whole prefix instead of
// rebuilding (M1, WP-140).

/// Stream a JSONL log line by line (`read_until` + `from_utf8`, R2-1): a
/// line that is not UTF-8 is skipped and counted like any corrupt line,
/// blank lines are ignored. A missing file reads as empty; a real I/O error
/// is returned (and never costs the meta).
fn for_each_line(path: &Path, mut f: impl FnMut(&str)) -> Result<usize> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(LitecodeError::Config(format!(
                "io: {}: {e}",
                path.display()
            )));
        }
    };
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut bad = 0usize;
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| LitecodeError::Config(format!("io: {}: {e}", path.display())))?;
        if n == 0 {
            return Ok(bad);
        }
        match std::str::from_utf8(&buf) {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => f(line.trim_end_matches(['\n', '\r'])),
            Err(_) => bad += 1,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Fail the next N `load`s with an injected I/O error.
    static FAIL_LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Fail every `read_meta` with an injected I/O error while set.
    static FAIL_META_READS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Bring a log to its committed length; if the file is shorter than the
/// checkpoint (or missing), keep its longest prefix of whole records
/// (`record` bytes) or whole lines instead. Returns the length kept and
/// whether salvage was needed.
fn salvage_log(path: &Path, committed: u64, record: Option<usize>) -> Result<(u64, bool)> {
    if truncate_to_committed(path, committed).is_ok() {
        return Ok((committed, false));
    }
    let io = |e: std::io::Error| LitecodeError::Config(format!("salvage {}: {e}", path.display()));
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let keep = match record {
        Some(r) => len - len % r as u64,
        None => {
            // Last newline, streamed (never the whole file in memory).
            let mut keep = 0u64;
            let mut pos = 0u64;
            if let Ok(file) = File::open(path) {
                let mut reader = BufReader::new(file);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    let n = reader.read_until(b'\n', &mut buf).map_err(io)?;
                    if n == 0 {
                        break;
                    }
                    pos += n as u64;
                    if buf.last() == Some(&b'\n') {
                        keep = pos;
                    }
                }
            }
            keep
        }
    };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(io)?;
    file.set_len(keep).map_err(io)?;
    file.sync_data().map_err(io)?;
    tracing::warn!(
        path = %path.display(),
        committed,
        kept = keep,
        "session index log shorter than its checkpoint; salvaged"
    );
    Ok((keep, true))
}

/// One line of the append-only settled log: a row entering the settled set,
/// or (`op: "del"`) leaving it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettledDel {
    op: String,
    session_id: String,
    seq: i64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrashAt {
    /// After the vector append of commit N (1-based).
    AfterVectors(usize),
    /// After the chunk append of commit N.
    AfterChunks(usize),
    /// After the settled append, before the checkpoint of commit N.
    BeforeCheckpoint(usize),
    /// After commit N's checkpoint.
    AfterCommit(usize),
    /// Migration: logs rewritten, layout-0 meta not yet replaced.
    MidMigration,
}

#[cfg(test)]
thread_local! {
    static CRASH: std::cell::Cell<Option<CrashAt>> = const { std::cell::Cell::new(None) };
    static COMMITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn crash_point(at: fn(usize) -> CrashAt) -> Result<()> {
    let n = COMMITS.with(|c| c.get());
    if CRASH.with(|c| c.get()) == Some(at(n)) {
        return Err(LitecodeError::Config(format!(
            "injected crash at {:?}",
            at(n)
        )));
    }
    Ok(())
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
    /// Committed log lengths (the last checkpoint).
    vectors_len: u64,
    chunks_len: u64,
    settled_len: u64,
    /// A first build / rebuild has not reached its final checkpoint yet.
    building: bool,
    /// Memory ran ahead of disk (a failed commit whose reload failed too):
    /// the next use reloads from the checkpoint before doing anything.
    needs_reload: bool,
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
            vectors_len: 0,
            chunks_len: 0,
            settled_len: 0,
            building: false,
            needs_reload: false,
        })
    }

    /// Load the last checkpoint: truncate every log to its committed length,
    /// replay the settled log, keep only the chunks a settled row claims, and
    /// rebuild the ANN graph in RAM from their logged vectors.
    pub fn load(workspace_root: &Path) -> Result<Self> {
        Self::load_tagged(workspace_root).map_err(|e| e.error)
    }

    /// `load` with the failure's reason kept as a type (`LoadFailure`), not
    /// guessed from the message.
    fn load_tagged(workspace_root: &Path) -> std::result::Result<Self, LoadError> {
        #[cfg(test)]
        if FAIL_LOADS.with(|c| {
            let n = c.get();
            c.set(n.saturating_sub(1));
            n > 0
        }) {
            return Err(LitecodeError::Config("io: injected load failure".into()).into());
        }
        let meta = read_meta(workspace_root)?
            .ok_or_else(|| LitecodeError::Config("session index meta missing".into()))?;
        let (vectors_len, v_salvaged) = salvage_log(
            &vectors_path(workspace_root),
            meta.vectors_len,
            Some(VECTOR_RECORD),
        )?;
        let (chunks_len, c_salvaged) =
            salvage_log(&chunks_path(workspace_root), meta.chunks_len, None)?;
        let (settled_len, s_salvaged) =
            salvage_log(&settled_path(workspace_root), meta.settled_len, None)?;
        let mut salvaged = v_salvaged || c_salvaged || s_salvaged;

        let mut index = Self::new_empty().map_err(LoadError::ann)?;
        index.embedder_id = meta.embedder_id.clone();
        index.last_change_id = meta.last_change_id;
        index.keys_hash = meta.keys_hash;
        index.vectors_len = vectors_len;
        index.chunks_len = chunks_len;
        index.settled_len = settled_len;
        index.building = meta.building;
        let (settled, bad_settled) = read_settled_lenient(workspace_root)?;
        index.settled = settled;
        salvaged |= bad_settled > 0;

        // No settled rows at all = no diff basis (a migrated pre-settled index,
        // or a build stopped before its first row settled): keep every logged
        // chunk so the legacy pass can reuse vectors by text instead of
        // re-embedding.
        let keep_all = index.settled.is_empty();
        let live_docs: HashSet<&str> = index
            .settled
            .values()
            .flat_map(|row| row.doc_keys.iter().map(String::as_str))
            .collect();
        let mut by_key: HashMap<String, SessionChunk> = HashMap::new();
        let mut max_id = 0;
        // A missing chunks log reads as empty (M1): its rows are redone.
        let mut bad_chunks = 0usize;
        let bad_utf8 = for_each_line(&chunks_path(workspace_root), |line| {
            // A torn or corrupt line is skipped, not fatal (M1); rows that
            // claimed it are redone below.
            let Ok(chunk) = serde_json::from_str::<SessionChunk>(line) else {
                bad_chunks += 1;
                return;
            };
            // Ids are never reused, even those of chunks no row claims anymore.
            max_id = max_id.max(chunk.id);
            if keep_all || live_docs.contains(chunk.key.as_str()) {
                by_key.insert(chunk.key.clone(), chunk);
            }
        })?;
        salvaged |= bad_chunks + bad_utf8 > 0;
        index.next_id = max_id + 1;
        let wanted: HashSet<u64> = by_key.values().map(|c| c.id).collect();

        // Streamed record by record: peak memory is the wanted vectors only,
        // never the whole log plus a copy.
        let mut vectors: HashMap<u64, Vec<f32>> = HashMap::with_capacity(wanted.len());
        if let Ok(file) = File::open(vectors_path(workspace_root)) {
            let mut reader = BufReader::with_capacity(1 << 16, file);
            let mut record = vec![0u8; VECTOR_RECORD];
            while std::io::Read::read_exact(&mut reader, &mut record).is_ok() {
                let id = u64::from_le_bytes(record[..8].try_into().expect("8 bytes"));
                if wanted.contains(&id) {
                    let vector = record[8..]
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().expect("4 bytes")))
                        .collect();
                    vectors.insert(id, vector);
                }
            }
        }
        index.ann.reserve(by_key.len().max(64)).map_err(|e| {
            LoadError::ann(LitecodeError::Config(format!("session ann reserve: {e}")))
        })?;
        for (key, chunk) in by_key {
            // A claimed chunk without its vector is not searchable: leave it
            // out, and drop the row's settled state so the next pass redoes it.
            let Some(vector) = vectors.get(&chunk.id) else {
                index.settled.retain(|_, row| !row.doc_keys.contains(&key));
                continue;
            };
            index.ann_add(chunk.id, vector).map_err(LoadError::ann)?;
            index.by_key.insert(key, chunk.id);
            index.chunks.insert(chunk.id, chunk);
        }
        if !keep_all {
            // A row whose chunk line was lost (salvaged log) is redone too.
            let before = index.settled.len();
            let by_key = &index.by_key;
            index
                .settled
                .retain(|_, row| row.doc_keys.iter().all(|k| by_key.contains_key(k)));
            salvaged |= index.settled.len() != before;
        }
        if salvaged {
            // Re-checkpoint the kept prefix and resume as a build, so the
            // dropped rows are diffed and redone, never a whole rebuild (M1).
            index.building = true;
            index.save_meta(workspace_root)?;
        }
        Ok(index)
    }

    /// Append one batch and checkpoint it (WP-144): vectors, chunks and settled
    /// lines are appended and fsynced, then the meta is replaced atomically
    /// with the new committed lengths. The cost is the batch's own bytes plus
    /// the small meta, whatever the index already holds. `watermark` is set
    /// only on the final commit of a pass.
    fn commit(
        &mut self,
        workspace_root: &Path,
        vectors: &[(u64, Vec<f32>)],
        chunk_ids: &[u64],
        settled_lines: &[String],
        watermark: Option<(i64, u64)>,
    ) -> Result<()> {
        // M2: a failed commit must not leave its lengths behind; the next
        // commit's `append_durable` then cuts the uncommitted tail.
        let committed = (
            self.vectors_len,
            self.chunks_len,
            self.settled_len,
            self.last_change_id,
            self.keys_hash,
            self.building,
        );
        let result =
            self.commit_inner(workspace_root, vectors, chunk_ids, settled_lines, watermark);
        if result.is_err() {
            // The in-memory index already holds this batch (chunks, settled
            // rows, removals) but the disk does not: reload the last checkpoint
            // so memory and disk agree and the next pass redoes the batch.
            // `load` cuts the uncommitted tails; `append_durable` would too.
            match Self::load_tagged(workspace_root) {
                Ok(reloaded) => *self = reloaded,
                Err(error) => {
                    tracing::error!(
                        reason = error.reason.as_str(),
                        error = %error.error,
                        "session index reload after a failed commit failed; forcing reload on next use"
                    );
                    self.needs_reload = true;
                    (
                        self.vectors_len,
                        self.chunks_len,
                        self.settled_len,
                        self.last_change_id,
                        self.keys_hash,
                        self.building,
                    ) = committed;
                }
            }
        }
        result
    }

    /// Forced reload after memory ran ahead of disk (see `needs_reload`).
    fn reload_if_needed(&mut self, workspace_root: &Path) -> Result<()> {
        if self.needs_reload {
            *self = Self::load(workspace_root)?;
        }
        Ok(())
    }

    /// Whether the next use must reload from disk first.
    pub fn needs_reload(&self) -> bool {
        self.needs_reload
    }

    fn commit_inner(
        &mut self,
        workspace_root: &Path,
        vectors: &[(u64, Vec<f32>)],
        chunk_ids: &[u64],
        settled_lines: &[String],
        watermark: Option<(i64, u64)>,
    ) -> Result<()> {
        #[cfg(test)]
        COMMITS.with(|c| c.set(c.get() + 1));
        let mut vbuf = Vec::with_capacity(vectors.len() * VECTOR_RECORD);
        for (id, vector) in vectors {
            vbuf.extend_from_slice(&id.to_le_bytes());
            for v in vector {
                vbuf.extend_from_slice(&v.to_le_bytes());
            }
        }
        if !vbuf.is_empty() {
            self.vectors_len =
                append_durable(&vectors_path(workspace_root), self.vectors_len, &vbuf)?;
        }
        #[cfg(test)]
        crash_point(CrashAt::AfterVectors)?;
        let mut cbuf = String::new();
        for id in chunk_ids {
            if let Some(chunk) = self.chunks.get(id) {
                cbuf.push_str(
                    &serde_json::to_string(chunk)
                        .map_err(|e| LitecodeError::Config(e.to_string()))?,
                );
                cbuf.push('\n');
            }
        }
        if !cbuf.is_empty() {
            self.chunks_len = append_durable(
                &chunks_path(workspace_root),
                self.chunks_len,
                cbuf.as_bytes(),
            )?;
        }
        #[cfg(test)]
        crash_point(CrashAt::AfterChunks)?;
        if !settled_lines.is_empty() {
            let mut sbuf = settled_lines.join("\n");
            sbuf.push('\n');
            self.settled_len = append_durable(
                &settled_path(workspace_root),
                self.settled_len,
                sbuf.as_bytes(),
            )?;
        }
        #[cfg(test)]
        crash_point(CrashAt::BeforeCheckpoint)?;
        if let Some((latest, keys_hash)) = watermark {
            self.last_change_id = latest;
            self.keys_hash = keys_hash;
            self.building = false;
        }
        self.save_meta(workspace_root)?;
        #[cfg(test)]
        crash_point(CrashAt::AfterCommit)?;
        Ok(())
    }

    fn save_meta(&self, workspace_root: &Path) -> Result<()> {
        write_meta(
            workspace_root,
            &SessionIndexMeta {
                last_change_id: self.last_change_id,
                keys_hash: self.keys_hash,
                vectors_len: self.vectors_len,
                chunks_len: self.chunks_len,
                settled_len: self.settled_len,
                building: self.building,
                ..SessionIndexMeta::shell(&self.embedder_id, self.chunks.len())
            },
        )
    }

    /// One-time upgrade of a pre-WP-144 index (whole-file `vectors.usearch`,
    /// layout 0) to the append-only layout without re-embedding: vectors are
    /// read back out of the old ANN dump. Idempotent until the new meta lands.
    fn migrate_legacy_layout(workspace_root: &Path) -> Result<bool> {
        let Some(meta) = read_meta(workspace_root)? else {
            return Ok(false);
        };
        let compatible_otherwise = meta.pipeline_version == PIPELINE_VERSION
            && meta.model_id == MODEL_ID
            && meta.embedder_id == production_embedder_id()
            && meta.embed_dim == EMBED_DIM
            && meta.doc_schema == DOC_SCHEMA;
        let legacy = legacy_vectors_path(workspace_root);
        if meta.layout != 0 || !compatible_otherwise || !legacy.is_file() {
            return Ok(false);
        }
        let ann = new_ann_index()?;
        restore_usearch(&ann, &legacy)?;
        let body = std::fs::read_to_string(chunks_path(workspace_root)).unwrap_or_default();
        let mut vbuf = Vec::new();
        let mut cbuf = String::new();
        let mut kept = 0usize;
        let mut vector = vec![0f32; EMBED_DIM];
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let chunk: SessionChunk = serde_json::from_str(line)
                .map_err(|e| LitecodeError::Config(format!("parse session chunk: {e}")))?;
            if ann.get(chunk.id, &mut vector).unwrap_or(0) == 0 {
                continue;
            }
            vbuf.extend_from_slice(&chunk.id.to_le_bytes());
            for v in &vector {
                vbuf.extend_from_slice(&v.to_le_bytes());
            }
            cbuf.push_str(line);
            cbuf.push('\n');
            kept += 1;
        }
        // The old settled file is whole rows, which the log format reads as-is.
        let sbody = std::fs::read(settled_path(workspace_root)).unwrap_or_default();
        let (vp, cp, sp) = (
            vectors_path(workspace_root),
            chunks_path(workspace_root),
            settled_path(workspace_root),
        );
        let put = |bytes: &[u8], w: &mut dyn std::io::Write| {
            w.write_all(bytes)
                .map_err(|e| LitecodeError::Config(e.to_string()))
        };
        atomic_write_set(&[
            AtomicPart {
                target: &vp,
                write: &|w| put(&vbuf, w),
            },
            AtomicPart {
                target: &cp,
                write: &|w| put(cbuf.as_bytes(), w),
            },
            AtomicPart {
                target: &sp,
                write: &|w| put(&sbody, w),
            },
        ])?;
        // The layout-0 meta and the legacy file still stand: a crash here
        // reruns the (idempotent) migration on the next load.
        #[cfg(test)]
        crash_point(|_| CrashAt::MidMigration)?;
        write_meta(
            workspace_root,
            &SessionIndexMeta {
                last_change_id: meta.last_change_id,
                keys_hash: meta.keys_hash,
                vectors_len: vbuf.len() as u64,
                chunks_len: cbuf.len() as u64,
                settled_len: sbody.len() as u64,
                building: false,
                ..SessionIndexMeta::shell(&meta.embedder_id, kept)
            },
        )?;
        let _ = std::fs::remove_file(&legacy);
        tracing::info!(
            chunks = kept,
            "session index migrated to append-only layout"
        );
        Ok(true)
    }

    /// Start a first build / rebuild (WP-144): empty logs plus a compatible
    /// meta with the watermark at zero and `building` set. From here the index
    /// is formal and searchable, and an interrupted build resumes from its last
    /// checkpoint instead of starting over.
    fn begin_build(workspace_root: &Path) -> Result<Self> {
        let dir = session_index_dir(workspace_root);
        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let _ = std::fs::remove_file(legacy_vectors_path(workspace_root));
        for path in [
            vectors_path(workspace_root),
            chunks_path(workspace_root),
            settled_path(workspace_root),
        ] {
            atomic_write_bytes(&path, b"")?;
        }
        let mut index = Self::new_empty()?;
        index.building = true;
        index.save_meta(workspace_root)?;
        Ok(index)
    }

    /// Test view of what the index *holds* (the append logs also carry
    /// retired lines until the next build): one JSON line per live chunk and
    /// per settled row.
    #[cfg(test)]
    pub(crate) fn live_artifact_lines(workspace_root: &Path) -> (Vec<String>, Vec<String>) {
        let index = Self::load(workspace_root).expect("load");
        let chunks = index
            .chunks
            .values()
            .map(|c| serde_json::to_string(c).unwrap())
            .collect();
        let settled = index
            .settled
            .values()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        (chunks, settled)
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
    ) -> Result<Vec<(u64, Vec<f32>)>> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let vectors = embedder.embed_batch(&texts)?;
        if chunks.len() != vectors.len() {
            return Err(LitecodeError::Config(
                "session chunks/vectors length mismatch".into(),
            ));
        }
        let mut added = Vec::with_capacity(chunks.len());
        for (chunk, vec) in chunks.into_iter().zip(vectors) {
            self.ann_add(chunk.id, &vec)?;
            self.by_key.insert(chunk.key.clone(), chunk.id);
            added.push((chunk.id, vec));
            self.chunks.insert(chunk.id, chunk);
        }
        Ok(added)
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
        self.reload_if_needed(workspace_root)?;
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
        if !self.building && self.last_change_id == latest && self.keys_hash == keys_hash {
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
        let mut settled_lines: Vec<String> = Vec::new();
        for key in &to_remove {
            if let Some(previous) = self.settled.remove(key) {
                for doc_key in &previous.doc_keys {
                    changed |= self.remove_doc(doc_key);
                }
                settled_lines.push(
                    serde_json::to_string(&SettledDel {
                        op: "del".into(),
                        session_id: key.0.clone(),
                        seq: key.1,
                    })
                    .map_err(|e| LitecodeError::Config(e.to_string()))?,
                );
            }
        }

        // WP-135: scanning — denominator = the whole index, but the new rows
        // are not cut yet, so the total is unknown (None), never estimated.
        // `done` = kept chunks only (deleted rows' chunks are already gone
        // above), the same number `queue.json` reported as `kept_chunks`, so
        // `done` never steps down across queued → scanning (M1).
        write_session_running(workspace_root, self.chunks.len() as u64, None);

        // Rows waiting for their chunks to commit: (staged chunks up to and
        // including this row's, row). Committed strictly in row order.
        let mut pending: Vec<(usize, SettledRow)> = Vec::new();
        let mut next_pending = 0;
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
                // WP-144: a row enters the settled set (the checkpoint) only
                // with the commit that holds its last staged chunk, so a
                // resume never finds a half-settled row.
                pending.push((
                    staged.len(),
                    SettledRow {
                        session_id: row.session_id.clone(),
                        seq: row.seq,
                        call_id: derived.call_id,
                        session_read_call: derived.session_read_call,
                        doc_keys: derived.docs.iter().map(|doc| doc.key.clone()).collect(),
                    },
                ));
            }
            // One embed call per batch, not per document: the delta is small but
            // the first pass over an existing index is not, and a session run per
            // chunk is the difference between minutes and seconds.
            // WP-135: cutting is done, so the total is exact: what the index
            // keeps plus every staged chunk. `done` advances per batch.
            let mut done = self.chunks.len() as u64;
            let total = done + staged.len() as u64;
            write_session_running(workspace_root, done, Some(total));
            let mut batch_end = 0;
            for batch in staged.chunks(EMBED_BATCH) {
                let vectors = self.embed_and_add(batch.to_vec(), embedder)?;
                batch_end += batch.len();
                let ids: Vec<u64> = batch.iter().map(|c| c.id).collect();
                while next_pending < pending.len() && pending[next_pending].0 <= batch_end {
                    let row = pending[next_pending].1.clone();
                    settled_lines.push(
                        serde_json::to_string(&row)
                            .map_err(|e| LitecodeError::Config(e.to_string()))?,
                    );
                    self.settled.insert((row.session_id.clone(), row.seq), row);
                    next_pending += 1;
                }
                // WP-144: one append + checkpoint per embed batch.
                self.commit(workspace_root, &vectors, &ids, &settled_lines, None)?;
                settled_lines.clear();
                done += batch.len() as u64;
                write_session_running(workspace_root, done, Some(total));
            }
        }
        // Rows with nothing to embed (policy drops, echo copies) and any
        // removals not yet checkpointed go with the final commit.
        for (_, row) in pending.drain(next_pending..) {
            settled_lines.push(
                serde_json::to_string(&row).map_err(|e| LitecodeError::Config(e.to_string()))?,
            );
            self.settled.insert((row.session_id.clone(), row.seq), row);
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

        self.embedder_id = embedder.embedder_id().into();
        // Nothing derived is not nothing to record: the watermark is what retires
        // the pending hint, and rewriting the vectors for it would be the whole
        // corpus of work for none of the corpus of change.
        // Nothing derived is not nothing to record: a settled row that produced
        // no document (an echo copy, a policy drop) still has to reach disk, and
        // the watermark is what retires the pending hint. Rewriting the vectors
        // for either would be the whole corpus of work for none of the corpus of
        // change, so each artifact is written only when its own content moved.
        // WP-144: the final checkpoint advances the watermark (and ends a
        // build); everything before it is already durable batch by batch.
        let _ = settled_changed;
        self.commit(
            workspace_root,
            &[],
            &[],
            &settled_lines,
            Some((latest, keys_hash)),
        )?;
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
    read_settled_lenient(workspace_root).map(|(settled, _)| settled)
}

/// Replay the settled log; an unparsable line is skipped and counted (M1),
/// its row simply redone.
fn read_settled_lenient(
    workspace_root: &Path,
) -> Result<(HashMap<(String, i64), SettledRow>, usize)> {
    let mut settled = HashMap::new();
    let mut bad = 0usize;
    let path = settled_path(workspace_root);
    // Append-only log (WP-144): a row line settles the row, a `del` line
    // retires it; replayed in order.
    let bad_utf8 = for_each_line(&path, |line| {
        if line.contains("\"op\":\"del\"") {
            match serde_json::from_str::<SettledDel>(line) {
                Ok(del) => {
                    settled.remove(&(del.session_id, del.seq));
                }
                Err(_) => bad += 1,
            }
            return;
        }
        match serde_json::from_str::<SettledRow>(line) {
            Ok(row) => {
                settled.insert((row.session_id.clone(), row.seq), row);
            }
            Err(_) => bad += 1,
        }
    })?;
    Ok((settled, bad + bad_utf8))
}

fn index_files_exist(workspace_root: &Path) -> bool {
    vectors_path(workspace_root).is_file() && chunks_path(workspace_root).is_file()
        || legacy_vectors_path(workspace_root).is_file()
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

fn write_session_job(workspace_root: &Path, job: &SessionJobFile) {
    if let Ok(body) = serde_json::to_vec_pretty(job) {
        let _ = atomic_write_bytes(&session_job_path(workspace_root), &body);
    }
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
    if let Ok(body) = serde_json::to_vec_pretty(&queue) {
        let _ = atomic_write_bytes(&path, &body);
    }
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
    let files = index_files_exist(workspace_root);
    // R3-1: an unreadable meta is not a missing one. A parse failure was
    // already turned into a deleted meta by `check_meta_or_drop` (-> None ->
    // rebuild); a pure I/O error must never send `begin_build` to wipe the
    // logs, so it reads as "no rebuild" and the load retries.
    match read_meta(workspace_root) {
        Err(_) => false,
        Ok(None) => true,
        Ok(Some(m)) =>
        // Layout 1 logs may be missing or short: `load` salvages them (M1).
        // Only a legacy index needs its whole-file vectors to exist.
        {
            needs_rebuild(&m) || (m.layout == 0 && !files)
        }
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
    // Worker load only: leftovers of an interrupted atomic write.
    sweep_atomic_tmp(&dir);
    check_meta_or_drop(workspace_root)?;
    if let Err(error) = SessionSemanticIndex::migrate_legacy_layout(workspace_root) {
        tracing::warn!(error = %error, "session index layout migration failed; kept legacy layout; retrying");
    }
    if session_should_rebuild(workspace_root) {
        return SessionSemanticIndex::new_empty();
    }
    match SessionSemanticIndex::load_tagged(workspace_root) {
        Ok(index) => Ok(index),
        Err(LoadError { reason, error }) => {
            // R2-1: only an unparsable meta (handled above) costs the meta.
            // Anything else (I/O, ANN) keeps the checkpoint and is retried on
            // the next load.
            tracing::error!(
                reason = reason.as_str(),
                error = %error,
                "session index load failed; checkpoint kept, retrying next load"
            );
            Err(error)
        }
    }
}

/// Why a load failed (R2-1). `MetaCorrupt` is decided before `load`, from
/// the meta bytes themselves (`check_meta_or_drop`); only it costs the meta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadFailure {
    Io,
    Ann,
}

impl LoadFailure {
    fn as_str(self) -> &'static str {
        match self {
            LoadFailure::Io => "io",
            LoadFailure::Ann => "ann",
        }
    }
}

#[derive(Debug)]
struct LoadError {
    reason: LoadFailure,
    error: LitecodeError,
}

impl LoadError {
    fn ann(error: LitecodeError) -> Self {
        LoadError {
            reason: LoadFailure::Ann,
            error,
        }
    }
}

/// Any untagged failure inside `load_tagged` is a read/write one.
impl From<LitecodeError> for LoadError {
    fn from(error: LitecodeError) -> Self {
        LoadError {
            reason: LoadFailure::Io,
            error,
        }
    }
}

/// R2-1: the meta is deleted only when its bytes do not parse. A read error
/// keeps it (returned as Err, retried next load).
fn check_meta_or_drop(workspace_root: &Path) -> Result<()> {
    let path = meta_path(workspace_root);
    match std::fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            tracing::error!(reason = "io", error = %e, "session index meta unreadable; kept");
            Err(LitecodeError::Config(format!(
                "io: {}: {e}",
                path.display()
            )))
        }
        Ok(bytes) => {
            if let Err(e) = serde_json::from_slice::<SessionIndexMeta>(&bytes) {
                tracing::error!(
                    reason = "meta_corrupt",
                    error = %e,
                    "session index meta unparsable; dropping it and rebuilding"
                );
                let _ = std::fs::remove_file(&path);
            }
            Ok(())
        }
    }
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
    // R4-1: an unreadable meta has no watermark; treating it as 0 would mark
    // every session dirty (an estimate). Keep the previous queue.json.
    let meta = match read_meta(workspace_root) {
        Ok(meta) => meta,
        Err(error) => {
            tracing::warn!(error = %error, "session index meta unreadable; queue.json kept");
            return;
        }
    };
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
    // An unreadable meta says nothing about what is indexed: reconciling an
    // (empty) in-memory index against it would re-embed everything. Stop and
    // let the next load retry; only `check_meta_or_drop` may drop a meta.
    if let Err(error) = read_meta(workspace_root) {
        tracing::error!(
            reason = "io",
            error = %error,
            "session index meta unreadable; consume skipped, retrying next load"
        );
        return Err(error);
    }
    if session_should_rebuild(workspace_root) {
        // WP-144: no more "delete meta, recompute everything in one go". The
        // new formal index (empty logs + compatible meta, watermark 0,
        // `building`) is written first; the build then commits batch by batch
        // and an interruption resumes from the last checkpoint. The sparse
        // lane's `sparse.db` in the same directory is untouched.
        tracing::info!("session_search building semantic index (resumable)");
        *index = SessionSemanticIndex::begin_build(workspace_root)?;
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

    // ---- WP-135 progress -------------------------------------------------

    use crate::engines::EngineState;
    use crate::engines::progress::{CorpusPhase, CorpusProgress, CorpusState, FailReason};

    fn warm() -> SessionEngineFacts {
        SessionEngineFacts {
            desired: true,
            state: Some(EngineState::Warm),
        }
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

    // ---- WP-144: resumable build ------------------------------------------



    /// Acceptance 3 (session): bytes written for one batch do not depend on
    /// how much the index already holds.
    #[test]
    fn wp144_bytes_per_batch_independent_of_index_size() {
        fn bytes_for_one_batch(existing: usize) -> u64 {
            let dir = TempDir::new().unwrap();
            let root = dir.path();
            let (data, id, db, _lease) = seeded(root, existing);
            let reader = crate::session::SessionDataReader::open(&db);
            let mut index = ensure_session_index(root, &reader, &mut HashEmbedder).unwrap();
            for i in 0..EMBED_BATCH {
                data.insert_items(&id, &[user_text(&format!("fresh row {i:04} text"))])
                    .unwrap();
            }
            crate::engines::code_search::WRITTEN_BYTES.with(|b| b.set(0));
            consume_session_index(root, &reader, &mut HashEmbedder, &mut index).unwrap();
            crate::engines::code_search::WRITTEN_BYTES.with(|b| b.get())
        }
        let small = bytes_for_one_batch(5);
        let large = bytes_for_one_batch(400);
        let payload = (EMBED_BATCH * VECTOR_RECORD) as u64;
        assert!(small >= payload, "the batch's vectors are written: {small}");
        assert!(
            small.abs_diff(large) * 20 < small,
            "per-batch bytes must not grow with the index: small={small} large={large}"
        );
        assert!(
            large < payload * 2,
            "about the batch's own size: {large} vs {payload}"
        );
    }

}
