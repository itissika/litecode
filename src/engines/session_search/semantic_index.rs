//! Session corpus ANN index under `.litecode/session-index/` (ANN-only).
//!
//! No BM25/CC/RRF here — lexical FTS lives in `sessions.db` on the always-on path.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::engines::code_search::{
    EMBED_DIM, Embedder, MODEL_ID, PIPELINE_VERSION, production_embedder_id,
};
use crate::types::{LitecodeError, Result};

use super::{SEMANTIC_WINDOW, SessionHitLane, SessionTextHit};

use crate::session::SessionDataReader;

const EMBED_BATCH: usize = 32;
const SNIPPET_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionChunk {
    id: u64,
    session_id: String,
    seq: i64,
    item_type: String,
    text: String,
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

fn needs_rebuild(meta: &SessionIndexMeta) -> bool {
    meta.pipeline_version != PIPELINE_VERSION
        || meta.model_id != MODEL_ID
        || meta.embedder_id != production_embedder_id()
        || meta.embed_dim != EMBED_DIM
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
    /// `(session_id, seq)` → chunk id for reconcile.
    by_key: HashMap<(String, i64), u64>,
    ann: Index,
    next_id: u64,
    embedder_id: String,
    last_change_id: i64,
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
        })
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let ann_path = vectors_path(workspace_root);
        let chunks_file = chunks_path(workspace_root);
        let ann = new_ann_index()?;
        ann.load(ann_path.to_str().unwrap_or("vectors.usearch"))
            .map_err(|e| LitecodeError::Config(format!("load session usearch: {e}")))?;

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
            index
                .by_key
                .insert((chunk.session_id.clone(), chunk.seq), id);
            index.chunks.insert(id, chunk);
        }
        Ok(index)
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let dir = session_index_dir(workspace_root);
        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;

        let ann_path = vectors_path(workspace_root);
        self.ann
            .save(ann_path.to_str().unwrap_or("vectors.usearch"))
            .map_err(|e| LitecodeError::Config(format!("save session usearch: {e}")))?;

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

        write_meta(
            workspace_root,
            &SessionIndexMeta {
                last_change_id: self.last_change_id,
                ..SessionIndexMeta::shell(&self.embedder_id, self.chunks.len())
            },
        )?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn last_change_id(&self) -> i64 {
        self.last_change_id
    }

    fn remove_id(&mut self, id: u64) {
        if let Some(chunk) = self.chunks.remove(&id) {
            self.by_key.remove(&(chunk.session_id, chunk.seq));
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
            self.by_key
                .insert((chunk.session_id.clone(), chunk.seq), chunk.id);
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
                char_start: 0,
                char_end: 0,
                lane: SessionHitLane::Semantic,
            });
            if hits.len() >= top_k {
                break;
            }
        }
        Ok(hits)
    }

    /// Reconcile against live sessions.db: add missing / changed texts, drop stale keys.
    pub fn reconcile(
        &mut self,
        reader: &SessionDataReader,
        workspace_root: &Path,
        embedder: &mut dyn Embedder,
    ) -> Result<bool> {
        let latest = reader.latest_change_id_blocking().unwrap_or(0);
        if latest == self.last_change_id && latest > 0 {
            return Ok(false);
        }
        if latest < self.last_change_id {
            *self = Self::new_empty()?;
        }
        let rows = reader.searchable_rows_blocking(None)?;
        let live =
            crate::session::transcript_file::iter_searchable_texts(&rows, reader.data_root())?;
        let live_keys: HashSet<(String, i64)> = live
            .iter()
            .map(|(sid, seq, _, _)| (sid.clone(), *seq))
            .collect();

        let mut dirty = false;
        let stale: Vec<u64> = self
            .chunks
            .iter()
            .filter(|(_, c)| !live_keys.contains(&(c.session_id.clone(), c.seq)))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.remove_id(id);
            dirty = true;
        }

        let mut to_add = Vec::new();
        for (session_id, seq, item_type, text) in live {
            let key = (session_id.clone(), seq);
            if let Some(&id) = self.by_key.get(&key) {
                if self.chunks.get(&id).is_some_and(|c| c.text == text) {
                    continue;
                }
                self.remove_id(id);
                dirty = true;
            }
            let id = self.next_id;
            self.next_id += 1;
            to_add.push(SessionChunk {
                id,
                session_id,
                seq,
                item_type,
                text,
            });
        }

        for batch in to_add.chunks(EMBED_BATCH) {
            self.embed_and_add(batch.to_vec(), embedder)?;
            dirty = true;
        }

        if dirty || self.last_change_id != latest {
            self.embedder_id = embedder.embedder_id().into();
            self.last_change_id = latest;
            self.save(workspace_root)?;
            write_session_pending_hint(workspace_root, 0);
            dirty = true;
        }
        Ok(dirty)
    }
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

pub fn session_work_from_disk(
    workspace_root: &Path,
) -> crate::engines::code_search::IndexWork {
    use crate::engines::code_search::{IndexRebuildReason, IndexWork};
    if session_should_rebuild(workspace_root) {
        let has_vectors = index_files_exist(workspace_root);
        let has_db = workspace_root.join(".litecode").join("sessions.db").is_file();
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
        let dir = session_index_dir(workspace_root);
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(
            reloaded.len(),
            1,
            "load must not digest new session rows"
        );
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
}
