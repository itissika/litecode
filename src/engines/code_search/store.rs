//! In-memory index: chunks.jsonl + usearch ANN vectors.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::types::{LitecodeError, Result};

use super::chunk::ChunkRecord;
use super::embed::{Embedder, production_embedder_id};
use super::meta::{self, IndexMeta, index_dir};

pub fn index_files_exist(workspace_root: &Path) -> bool {
    let dir = index_dir(workspace_root);
    dir.join("vectors.usearch").is_file() && dir.join("chunks.jsonl").is_file()
}

pub fn vectors_path(workspace_root: &Path) -> PathBuf {
    index_dir(workspace_root).join("vectors.usearch")
}

pub fn chunks_path(workspace_root: &Path) -> PathBuf {
    index_dir(workspace_root).join("chunks.jsonl")
}

/// WP-143: appended vector records since the last `vectors.usearch` snapshot.
pub fn vectors_log_path(workspace_root: &Path) -> PathBuf {
    index_dir(workspace_root).join("vectors.log")
}

/// WP-143: commit marker for the append-only data files.
pub fn checkpoint_path(workspace_root: &Path) -> PathBuf {
    index_dir(workspace_root).join("checkpoint.json")
}

/// Committed lengths of the append-only files (replaced atomically per batch).
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Checkpoint {
    vectors_log_len: u64,
    chunks_len: u64,
    snapshot_len: u64,
    next_id: u64,
}

/// One store mutation since the last commit, replayed into the append logs.
#[derive(Debug, Clone, Copy)]
enum JournalOp {
    Add(u64),
    Del(u64),
}

#[derive(Default)]
struct Journal {
    ops: Vec<JournalOp>,
    /// Last committed checkpoint for *this* workspace dir (None ⇒ full write).
    committed: Option<(PathBuf, Checkpoint)>,
}

const LOG_ADD: u8 = 1;
const LOG_DEL: u8 = 0;
/// Below this snapshot size a full rewrite is cheap; don't grow logs.
const COMPACT_FLOOR_BYTES: u64 = 1 << 20;

#[cfg(test)]
thread_local! {
    /// WP-143 test counters: completed saves, and full rewrites among them.
    pub(crate) static SAVE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static FULL_WRITE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Byte counter used to size a streamed file before writing it.
struct CountingSink(u64);

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct DelLine {
    del: u64,
}

pub struct CodeSearchIndex {
    chunks: HashMap<u64, ChunkRecord>,
    ann: Index,
    next_id: u64,
    indexed_files: usize,
    embedder_id: String,
    /// path → chunk ids, so per-file work stays O(file) on a large store.
    by_path: HashMap<String, Vec<u64>>,
    /// WP-143: paths whose chunks were dropped by the load-time repair.
    repaired_paths: Vec<String>,
    /// WP-143: the load repair could not name every affected path and
    /// dropped all stamps; the runtime must forget its in-memory ones too.
    stamps_reset: bool,
    /// WP-143: mutations not yet committed + the last committed checkpoint.
    journal: std::sync::Mutex<Journal>,
    /// Identity of this index generation (meta.created_at). Stable across
    /// per-batch saves so `stamps.v1` can be appended (WP-143); a fresh
    /// index gets a new one.
    created_at: String,
}

impl CodeSearchIndex {
    pub fn new_empty() -> Result<Self> {
        let ann = new_ann_index()?;
        Ok(Self {
            chunks: HashMap::new(),
            ann,
            next_id: 1,
            indexed_files: 0,
            embedder_id: production_embedder_id().into(),
            by_path: HashMap::new(),
            repaired_paths: Vec::new(),
            stamps_reset: false,
            journal: std::sync::Mutex::new(Journal::default()),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Load snapshot + append logs (truncated to the checkpoint) + meta.
    ///
    /// WP-143: a crash can leave uncommitted tails (cut back here) or, during
    /// a compaction, files from two generations. Replay is tolerant (ids are
    /// never reused), then the R1 backstop keeps only chunks that have a
    /// vector and vectors that have a chunk; paths that lost chunks get their
    /// stamps invalidated and are re-read. Never a whole-index rebuild.
    pub fn load(workspace_root: &Path) -> Result<Self> {
        super::atomic::sweep_atomic_tmp(&index_dir(workspace_root));
        let ann = new_ann_index()?;
        restore_usearch(&ann, &vectors_path(workspace_root))?;
        let snapshot_len = std::fs::metadata(vectors_path(workspace_root))
            .map(|m| m.len())
            .unwrap_or(0);

        let meta_on_disk = meta::read_meta(workspace_root)?;
        let embedder_id = meta_on_disk
            .as_ref()
            .map(|m| m.embedder_id.clone())
            .unwrap_or_else(|| production_embedder_id().into());

        let ckpt: Option<Checkpoint> = std::fs::read(checkpoint_path(workspace_root))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let mut suspect = false;
        let (log_len, chunks_len) = match ckpt {
            // The checkpoint describes another snapshot generation: a crash
            // inside a compaction's renames. The renamed files are complete,
            // so never cut them to the old lengths (that drops live chunks).
            Some(c) if c.snapshot_len != snapshot_len => {
                suspect = true;
                (None, None)
            }
            Some(c) => {
                let log_ok = super::atomic::truncate_to_committed(
                    &vectors_log_path(workspace_root),
                    c.vectors_log_len,
                )
                .is_ok();
                let chunks_ok = super::atomic::truncate_to_committed(
                    &chunks_path(workspace_root),
                    c.chunks_len,
                )
                .is_ok();
                suspect |= !log_ok || !chunks_ok;
                (
                    if log_ok {
                        Some(c.vectors_log_len)
                    } else {
                        None
                    },
                    if chunks_ok { Some(c.chunks_len) } else { None },
                )
            }
            None => (None, None),
        };

        let mut index = Self {
            chunks: HashMap::new(),
            ann,
            next_id: ckpt.map_or(1, |c| c.next_id.max(1)),
            indexed_files: 0,
            embedder_id,
            by_path: HashMap::new(),
            repaired_paths: Vec::new(),
            stamps_reset: false,
            journal: std::sync::Mutex::new(Journal::default()),
            created_at: meta_on_disk
                .as_ref()
                .map(|m| m.created_at.clone())
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        };

        // Replay vectors.log (tolerant: skip present adds, ignore absent dels).
        if let Ok(buf) = std::fs::read(vectors_log_path(workspace_root)) {
            let end = log_len.map_or(buf.len(), |l| (l as usize).min(buf.len()));
            let rec = 9 + 4 * super::EMBED_DIM;
            let mut at = 0usize;
            while at + 9 <= end {
                let id = u64::from_le_bytes(buf[at..at + 8].try_into().expect("8 bytes"));
                let kind = buf[at + 8];
                if kind == LOG_DEL {
                    let _ = index.ann.remove(id);
                    at += 9;
                    continue;
                }
                if at + rec > end {
                    suspect = true;
                    break;
                }
                let v: Vec<f32> = buf[at + 9..at + rec]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().expect("4 bytes")))
                    .collect();
                if !index.ann.contains(id) {
                    index.ann_add(id, &v)?;
                }
                index.next_id = index.next_id.max(id + 1);
                at += rec;
            }
        }

        // Replay chunks.jsonl (records + `{"del":id}` tombstones).
        let file = File::open(chunks_path(workspace_root))
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut reader = BufReader::new(file);
        let mut read = 0u64;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| LitecodeError::Config(e.to_string()))?;
            if n == 0 || chunks_len.is_some_and(|l| read >= l) {
                break;
            }
            read += n as u64;
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<ChunkRecord>(t) {
                index.next_id = index.next_id.max(chunk.id + 1);
                if let Some(old) = index.chunks.remove(&chunk.id) {
                    index.unlink_path(&old.path, old.id);
                }
                index.insert_chunk(chunk);
            } else if let Ok(d) = serde_json::from_str::<DelLine>(t) {
                if let Some(old) = index.chunks.remove(&d.del) {
                    index.unlink_path(&old.path, old.id);
                }
            } else {
                // Torn line without a checkpoint to cut it: stop here.
                suspect = true;
                break;
            }
        }

        // R1 backstop: chunks without a vector drop their whole file.
        let lost: std::collections::BTreeSet<String> = index
            .chunks
            .values()
            .filter(|c| !index.ann.contains(c.id))
            .map(|c| c.path.clone())
            .collect();
        for path in &lost {
            index.remove_file(path);
        }
        // Vectors without a chunk cannot be listed by key: rebuild the ANN
        // from the kept chunks (repair only).
        let orphans = index.ann.size() != index.chunks.len();
        if orphans {
            index.rebuild_ann()?;
        }
        index.indexed_files = index.by_path.len();
        let meta_stale = meta_on_disk
            .as_ref()
            .is_some_and(|m| m.indexed_chunks != index.chunks.len());
        if let Ok(mut j) = index.journal.lock() {
            j.ops.clear();
            j.committed = ckpt.map(|c| (workspace_root.to_path_buf(), c));
        }
        if !lost.is_empty() || orphans || suspect || meta_stale {
            tracing::warn!(
                lost_files = lost.len(),
                orphans,
                suspect,
                meta_stale,
                "code_search load: store out of step after a crash; repaired"
            );
            index.repaired_paths = lost.into_iter().collect();
            // Orphan vectors or a mixed generation: chunks may be gone whose
            // paths cannot be named. Drop every stamp, so the next reconcile
            // re-reads all files (the content check keeps intact files at
            // zero embeds) and the missing ones are re-indexed.
            index.stamps_reset = orphans || suspect;
            let stamps = if index.stamps_reset {
                super::stamps::discard(workspace_root);
                None
            } else {
                // Stamps are tied to meta.created_at: read them before the
                // repair rewrite, re-save after with the lost paths unstamped.
                super::stamps::load(workspace_root)
            };
            index.save_full(workspace_root)?;
            if let Some(mut entries) = stamps {
                for path in &index.repaired_paths {
                    entries.insert(path.clone(), None);
                }
                super::stamps::save(workspace_root, &entries);
            }
        }
        Ok(index)
    }

    fn rebuild_ann(&mut self) -> Result<()> {
        let fresh = new_ann_index()?;
        fresh
            .reserve(self.chunks.len().max(64))
            .map_err(|e| LitecodeError::Config(format!("ann reserve: {e}")))?;
        let mut ids: Vec<u64> = self.chunks.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if let Some(v) = ann_get(&self.ann, id) {
                fresh
                    .add(id, &v)
                    .map_err(|e| LitecodeError::Config(format!("ann add: {e}")))?;
            }
        }
        self.ann = fresh;
        Ok(())
    }

    /// WP-143: true once if the load repair discarded every stamp.
    pub(crate) fn take_stamps_reset(&mut self) -> bool {
        std::mem::take(&mut self.stamps_reset)
    }

    /// WP-143: paths dropped by the load-time repair (stamps already invalidated).
    pub(crate) fn take_repaired_paths(&mut self) -> Vec<String> {
        std::mem::take(&mut self.repaired_paths)
    }

    /// Commit pending mutations. WP-143: appends this batch's records to
    /// `vectors.log` / `chunks.jsonl` (fsynced), then atomically replaces
    /// `checkpoint.json` (the commit point), then meta. Bytes written ≈ the
    /// batch, not the store. Falls back to a full atomic rewrite (compaction)
    /// when there is no checkpoint yet or the log outgrew the snapshot.
    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let committed = self.journal.lock().ok().and_then(|j| {
            j.committed
                .as_ref()
                .filter(|(dir, _)| dir == workspace_root)
                .map(|(_, c)| *c)
        });
        let Some(c) = committed else {
            return self.save_full(workspace_root);
        };
        let ops: Vec<JournalOp> = self
            .journal
            .lock()
            .map(|j| j.ops.clone())
            .unwrap_or_default();
        let mut vec_bytes: Vec<u8> = Vec::new();
        let mut chunk_bytes: Vec<u8> = Vec::new();
        for op in &ops {
            match *op {
                JournalOp::Add(id) => {
                    let (Some(chunk), Some(v)) = (self.chunks.get(&id), ann_get(&self.ann, id))
                    else {
                        continue; // added then removed before this commit
                    };
                    vec_bytes.extend_from_slice(&id.to_le_bytes());
                    vec_bytes.push(LOG_ADD);
                    for x in v {
                        vec_bytes.extend_from_slice(&x.to_le_bytes());
                    }
                    let line = serde_json::to_string(chunk)
                        .map_err(|e| LitecodeError::Config(e.to_string()))?;
                    chunk_bytes.extend_from_slice(line.as_bytes());
                    chunk_bytes.push(b'\n');
                }
                JournalOp::Del(id) => {
                    vec_bytes.extend_from_slice(&id.to_le_bytes());
                    vec_bytes.push(LOG_DEL);
                    chunk_bytes.extend_from_slice(format!("{{\"del\":{id}}}\n").as_bytes());
                }
            }
        }
        if c.vectors_log_len + vec_bytes.len() as u64 > c.snapshot_len.max(COMPACT_FLOOR_BYTES) {
            return self.save_full(workspace_root);
        }
        let log_len = super::atomic::append_durable(
            &vectors_log_path(workspace_root),
            c.vectors_log_len,
            &vec_bytes,
        )?;
        let chunks_len = super::atomic::append_durable(
            &chunks_path(workspace_root),
            c.chunks_len,
            &chunk_bytes,
        )?;
        let next = Checkpoint {
            vectors_log_len: log_len,
            chunks_len,
            snapshot_len: c.snapshot_len,
            next_id: self.next_id,
        };
        self.commit_checkpoint(workspace_root, next)?;
        if let Ok(mut j) = self.journal.lock() {
            j.ops.drain(..ops.len());
            j.committed = Some((workspace_root.to_path_buf(), next));
        }
        #[cfg(test)]
        SAVE_COUNT.with(|n| n.set(n.get() + 1));
        Ok(())
    }

    fn commit_checkpoint(&self, workspace_root: &Path, c: Checkpoint) -> Result<()> {
        let body = serde_json::to_vec(&c).map_err(|e| LitecodeError::Config(e.to_string()))?;
        super::atomic::atomic_write_bytes(&checkpoint_path(workspace_root), &body)?;
        let meta_body = serde_json::to_vec_pretty(&self.meta_snapshot())
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        super::atomic::atomic_write_bytes(&meta::meta_path(workspace_root), &meta_body)
    }

    /// Compaction: atomic rewrite of snapshot + chunks + empty log, then the
    /// checkpoint, then meta (commit markers last).
    pub(crate) fn save_full(&self, workspace_root: &Path) -> Result<()> {
        std::fs::create_dir_all(index_dir(workspace_root))
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut ann_buf = vec![0u8; self.ann.serialized_length()];
        self.ann
            .save_to_buffer(&mut ann_buf)
            .map_err(|e| LitecodeError::Config(format!("save usearch: {e}")))?;
        // Chunk lines are streamed (twice: once to measure, once to write),
        // never buffered whole: on a large store they dwarf the vectors.
        let mut ids: Vec<u64> = self.chunks.keys().copied().collect();
        ids.sort_unstable();
        let write_lines = |out: &mut dyn Write| -> Result<()> {
            for id in &ids {
                serde_json::to_writer(&mut *out, &self.chunks[id])
                    .map_err(|e| LitecodeError::Config(e.to_string()))?;
                out.write_all(b"\n")
                    .map_err(|e| LitecodeError::Config(e.to_string()))?;
            }
            Ok(())
        };
        let mut sink = CountingSink(0);
        write_lines(&mut sink)?;
        let ckpt = Checkpoint {
            vectors_log_len: 0,
            chunks_len: sink.0,
            snapshot_len: ann_buf.len() as u64,
            next_id: self.next_id,
        };
        let ckpt_body =
            serde_json::to_vec(&ckpt).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let meta_body = serde_json::to_vec_pretty(&self.meta_snapshot())
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        let w = |b: &[u8], w: &mut dyn Write| -> Result<()> {
            w.write_all(b)
                .map_err(|e| LitecodeError::Config(format!("save index: {e}")))
        };
        let (wv, wc, wl, wk, wm) = (
            |o: &mut dyn Write| w(&ann_buf, o),
            |o: &mut dyn Write| write_lines(o),
            |o: &mut dyn Write| w(&[], o),
            |o: &mut dyn Write| w(&ckpt_body, o),
            |o: &mut dyn Write| w(&meta_body, o),
        );
        let (vp, cp, lp, kp, mp) = (
            vectors_path(workspace_root),
            chunks_path(workspace_root),
            vectors_log_path(workspace_root),
            checkpoint_path(workspace_root),
            meta::meta_path(workspace_root),
        );
        use super::atomic::AtomicPart as P;
        super::atomic::atomic_write_set(&[
            P {
                target: &vp,
                write: &wv,
            },
            P {
                target: &cp,
                write: &wc,
            },
            P {
                target: &lp,
                write: &wl,
            },
            P {
                target: &kp,
                write: &wk,
            },
            P {
                target: &mp,
                write: &wm,
            },
        ])?;
        if let Ok(mut j) = self.journal.lock() {
            j.ops.clear();
            j.committed = Some((workspace_root.to_path_buf(), ckpt));
        }
        #[cfg(test)]
        {
            SAVE_COUNT.with(|n| n.set(n.get() + 1));
            FULL_WRITE_COUNT.with(|n| n.set(n.get() + 1));
        }
        Ok(())
    }

    pub fn meta_snapshot(&self) -> IndexMeta {
        IndexMeta {
            model_id: super::MODEL_ID.into(),
            embedder_id: self.embedder_id.clone(),
            pipeline_version: super::PIPELINE_VERSION,
            embed_dim: super::EMBED_DIM,
            chunk_lines: super::CHUNK_LINES,
            chunk_overlap: super::CHUNK_OVERLAP,
            created_at: self.created_at.clone(),
            indexed_files: self.indexed_files,
            indexed_chunks: self.chunks.len(),
        }
    }

    pub fn set_embedder_id(&mut self, id: impl Into<String>) {
        self.embedder_id = id.into();
    }

    pub fn chunks(&self) -> &HashMap<u64, ChunkRecord> {
        &self.chunks
    }

    /// Dense embedding for MMR / diagnostics (stored only in usearch).
    pub fn get_vector(&self, key: u64) -> Option<Vec<f32>> {
        ann_get(&self.ann, key)
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    pub fn set_next_id(&mut self, id: u64) {
        self.next_id = id;
    }

    pub fn remove_file(&mut self, rel_path: &str) {
        let Some(ids) = self.by_path.remove(rel_path) else {
            return;
        };
        for id in &ids {
            self.chunks.remove(id);
            let _ = self.ann.remove(*id);
        }
        self.journal_push(ids.into_iter().map(JournalOp::Del));
        self.indexed_files = self.by_path.len();
    }

    fn journal_push(&self, ops: impl Iterator<Item = JournalOp>) {
        if let Ok(mut j) = self.journal.lock() {
            j.ops.extend(ops);
        }
    }

    fn insert_chunk(&mut self, chunk: ChunkRecord) {
        self.by_path
            .entry(chunk.path.clone())
            .or_default()
            .push(chunk.id);
        self.chunks.insert(chunk.id, chunk);
    }

    /// Chunk count of one indexed path (O(1)).
    pub(crate) fn file_chunk_count(&self, rel_path: &str) -> usize {
        self.by_path.get(rel_path).map_or(0, Vec::len)
    }

    /// Vectors held by the ANN (consistency checks).
    pub(crate) fn vector_count(&self) -> usize {
        self.ann.size()
    }

    /// Test hook: drop one vector behind the store's back.
    #[cfg(test)]
    pub(crate) fn remove_vector_for_test(&self, id: u64) {
        let _ = self.ann.remove(id);
    }

    pub(crate) fn has_path(&self, rel_path: &str) -> bool {
        self.by_path.contains_key(rel_path)
    }

    fn unlink_path(&mut self, path: &str, id: u64) {
        if let Some(ids) = self.by_path.get_mut(path) {
            ids.retain(|x| *x != id);
            if ids.is_empty() {
                self.by_path.remove(path);
            }
        }
    }

    /// Distinct relative paths currently present in the chunk store.
    pub fn indexed_paths(&self) -> std::collections::HashSet<String> {
        self.by_path.keys().cloned().collect()
    }

    /// Whether on-disk file content still matches indexed chunks for `rel_path`.
    pub fn file_content_matches(&self, rel_path: &str, content: &str) -> bool {
        let mut old: Vec<&ChunkRecord> = self
            .by_path
            .get(rel_path)
            .map(|ids| ids.iter().filter_map(|id| self.chunks.get(id)).collect())
            .unwrap_or_default();
        old.sort_by_key(|c| (c.start_line, c.end_line, c.id));
        let (new_chunks, _) = super::chunk::chunk_file(rel_path, content, 1);
        if old.len() != new_chunks.len() {
            return false;
        }
        old.iter().zip(new_chunks.iter()).all(|(o, n)| {
            o.start_line == n.start_line && o.end_line == n.end_line && o.text == n.text
        })
    }

    /// WP-141: of `candidates` (path, freshly cut chunks), the paths whose
    /// indexed chunks are identical (same lines and text). One pass over the
    /// store, so a full-read reconcile stays linear.
    pub fn paths_with_identical_chunks<'a>(
        &self,
        candidates: impl Iterator<Item = (&'a str, &'a [ChunkRecord])>,
    ) -> std::collections::HashSet<String> {
        let mut same = std::collections::HashSet::new();
        for (path, new_chunks) in candidates {
            let Some(ids) = self.by_path.get(path) else {
                continue;
            };
            let mut prev: Vec<&ChunkRecord> =
                ids.iter().filter_map(|id| self.chunks.get(id)).collect();
            prev.sort_by_key(|c| (c.start_line, c.end_line, c.id));
            if prev.len() == new_chunks.len()
                && prev.iter().zip(new_chunks.iter()).all(|(o, n)| {
                    o.start_line == n.start_line && o.end_line == n.end_line && o.text == n.text
                })
            {
                same.insert(path.to_string());
            }
        }
        same
    }

    pub fn add_chunks_with_vectors(
        &mut self,
        chunks: Vec<ChunkRecord>,
        vectors: Vec<Vec<f32>>,
    ) -> Result<()> {
        if chunks.len() != vectors.len() {
            return Err(LitecodeError::Config(
                "chunks/vectors length mismatch".into(),
            ));
        }
        for (chunk, vec) in chunks.into_iter().zip(vectors) {
            self.ann_add(chunk.id, &vec)?;
            self.journal_push(std::iter::once(JournalOp::Add(chunk.id)));
            self.insert_chunk(chunk);
        }
        self.indexed_files = self.by_path.len();
        Ok(())
    }

    pub fn embed_and_add(
        &mut self,
        chunks: Vec<ChunkRecord>,
        embedder: &mut dyn Embedder,
    ) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let vectors = embedder.embed_batch(&texts)?;
        self.add_chunks_with_vectors(chunks, vectors)
    }

    pub fn ann_search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        if self.chunks.is_empty() {
            return Ok(Vec::new());
        }
        let results = self
            .ann
            .search(query, k.min(self.chunks.len()))
            .map_err(|e| LitecodeError::Config(format!("ann search: {e}")))?;
        Ok(results
            .keys
            .iter()
            .zip(results.distances.iter())
            .map(|(&key, &dist)| (key, dist))
            .collect())
    }

    fn ann_add(&mut self, key: u64, vector: &[f32]) -> Result<()> {
        let needed = self.ann.size() + 1;
        if self.ann.capacity() < needed {
            // Geometric growth: one reserve per doubling, not per chunk.
            self.ann
                .reserve(needed.max(self.ann.capacity() * 2).max(64))
                .map_err(|e| LitecodeError::Config(format!("ann reserve: {e}")))?;
        }
        self.ann
            .add(key, vector)
            .map_err(|e| LitecodeError::Config(format!("ann add: {e}")))?;
        Ok(())
    }
}

fn ann_get(ann: &Index, key: u64) -> Option<Vec<f32>> {
    let mut vec = vec![0f32; super::EMBED_DIM];
    ann.get(key, &mut vec).ok().map(|_| vec)
}

fn new_ann_index() -> Result<Index> {
    let mut options = IndexOptions::default();
    options.dimensions = super::EMBED_DIM;
    options.metric = MetricKind::Cos;
    options.quantization = ScalarKind::BF16;
    Index::new(&options).map_err(|e| LitecodeError::Config(format!("usearch new: {e}")))
}

/// Write ANN bytes with `std::fs` instead of usearch's `Index::save(path)`.
///
/// The C++ binding opens the path with libc `fopen` / `CreateFileA` (ANSI on
/// Windows) and stores a temporary `c_str()`. New-workspace warmup then fails
/// with `No such file or directory` for Unicode roots or a dangling path.
pub(crate) fn persist_usearch(ann: &Index, path: &Path) -> Result<()> {
    let mut buf = vec![0u8; ann.serialized_length()];
    ann.save_to_buffer(&mut buf)
        .map_err(|e| LitecodeError::Config(format!("save usearch: {e}")))?;
    std::fs::write(path, buf).map_err(|e| LitecodeError::Config(format!("save usearch: {e}")))
}

/// Read ANN bytes with `std::fs` instead of usearch's `Index::load(path)`.
pub(crate) fn restore_usearch(ann: &Index, path: &Path) -> Result<()> {
    let buf =
        std::fs::read(path).map_err(|e| LitecodeError::Config(format!("load usearch: {e}")))?;
    ann.load_from_buffer(&buf)
        .map_err(|e| LitecodeError::Config(format!("load usearch: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::chunk::chunk_file;
    use crate::engines::code_search::embed::HashEmbedder;
    use tempfile::TempDir;

    #[test]
    fn round_trip_save_load() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut index = CodeSearchIndex::new_empty().unwrap();
        let (chunks, _) = chunk_file("main.rs", "fn main() {}\n", 1);
        let mut emb = HashEmbedder;
        index.embed_and_add(chunks, &mut emb).unwrap();
        index.save(root).unwrap();

        assert!(vectors_path(root).is_file());
        let loaded = CodeSearchIndex::load(root).unwrap();
        assert_eq!(loaded.chunks().len(), 1);
    }

    #[test]
    fn save_empty_index_creates_vectors_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        CodeSearchIndex::new_empty().unwrap().save(root).unwrap();
        assert!(vectors_path(root).is_file());
        let loaded = CodeSearchIndex::load(root).unwrap();
        assert!(loaded.chunks().is_empty());
    }

    #[test]
    fn save_load_unicode_workspace_path() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("新工作区");
        std::fs::create_dir_all(&root).unwrap();
        let mut index = CodeSearchIndex::new_empty().unwrap();
        let (chunks, _) = chunk_file("main.rs", "fn main() {}\n", 1);
        let mut emb = HashEmbedder;
        index.embed_and_add(chunks, &mut emb).unwrap();
        index.save(&root).unwrap();
        let loaded = CodeSearchIndex::load(&root).unwrap();
        assert_eq!(loaded.chunks().len(), 1);
    }
}
