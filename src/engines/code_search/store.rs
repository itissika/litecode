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

pub struct CodeSearchIndex {
    chunks: HashMap<u64, ChunkRecord>,
    ann: Index,
    next_id: u64,
    indexed_files: usize,
    embedder_id: String,
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
        })
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let ann_path = vectors_path(workspace_root);
        let chunks_path = chunks_path(workspace_root);
        let ann = new_ann_index()?;
        ann.load(ann_path.to_str().unwrap_or("vectors.usearch"))
            .map_err(|e| LitecodeError::Config(format!("load usearch: {e}")))?;

        let meta_on_disk = meta::read_meta(workspace_root)?;
        let embedder_id = meta_on_disk
            .as_ref()
            .map(|m| m.embedder_id.clone())
            .unwrap_or_else(|| production_embedder_id().into());

        let mut index = Self {
            chunks: HashMap::new(),
            ann,
            next_id: 1,
            indexed_files: 0,
            embedder_id,
        };

        let file = File::open(&chunks_path).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.map_err(|e| LitecodeError::Config(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let chunk: ChunkRecord = serde_json::from_str(&line)
                .map_err(|e| LitecodeError::Config(format!("parse chunk: {e}")))?;
            let id = chunk.id;
            index.next_id = index.next_id.max(id + 1);
            index.chunks.insert(id, chunk);
        }
        index.indexed_files = index
            .chunks
            .values()
            .map(|c| c.path.clone())
            .collect::<std::collections::HashSet<_>>()
            .len();
        Ok(index)
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let dir = index_dir(workspace_root);
        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;

        let ann_path = vectors_path(workspace_root);
        self.ann
            .save(ann_path.to_str().unwrap_or("vectors.usearch"))
            .map_err(|e| LitecodeError::Config(format!("save usearch: {e}")))?;

        let chunks_path = chunks_path(workspace_root);
        let mut file =
            File::create(&chunks_path).map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut ids: Vec<u64> = self.chunks.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let chunk = &self.chunks[&id];
            let line =
                serde_json::to_string(chunk).map_err(|e| LitecodeError::Config(e.to_string()))?;
            writeln!(file, "{line}").map_err(|e| LitecodeError::Config(e.to_string()))?;
        }

        meta::write_meta(workspace_root, &self.meta_snapshot())?;
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
            created_at: chrono::Utc::now().to_rfc3339(),
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
        let ids: Vec<u64> = self
            .chunks
            .values()
            .filter(|c| c.path == rel_path)
            .map(|c| c.id)
            .collect();
        for id in ids {
            self.chunks.remove(&id);
            let _ = self.ann.remove(id);
        }
        self.recompute_file_count();
    }

    /// Distinct relative paths currently present in the chunk store.
    pub fn indexed_paths(&self) -> std::collections::HashSet<String> {
        self.chunks.values().map(|c| c.path.clone()).collect()
    }

    /// Whether on-disk file content still matches indexed chunks for `rel_path`.
    pub fn file_content_matches(&self, rel_path: &str, content: &str) -> bool {
        let mut old: Vec<&ChunkRecord> = self
            .chunks
            .values()
            .filter(|c| c.path == rel_path)
            .collect();
        old.sort_by_key(|c| (c.start_line, c.end_line, c.id));
        let (new_chunks, _) = super::chunk::chunk_file(rel_path, content, 1);
        if old.len() != new_chunks.len() {
            return false;
        }
        old.iter().zip(new_chunks.iter()).all(|(o, n)| {
            o.start_line == n.start_line && o.end_line == n.end_line && o.text == n.text
        })
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
            self.chunks.insert(chunk.id, chunk);
        }
        self.recompute_file_count();
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
        let needed = self.chunks.len() + 1;
        if self.ann.capacity() < needed {
            self.ann
                .reserve(needed.max(64))
                .map_err(|e| LitecodeError::Config(format!("ann reserve: {e}")))?;
        }
        self.ann
            .add(key, vector)
            .map_err(|e| LitecodeError::Config(format!("ann add: {e}")))?;
        Ok(())
    }

    fn recompute_file_count(&mut self) {
        self.indexed_files = self
            .chunks
            .values()
            .map(|c| c.path.clone())
            .collect::<std::collections::HashSet<_>>()
            .len();
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
}
