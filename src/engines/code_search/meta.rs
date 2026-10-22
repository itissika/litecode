//! `.litecode/index/meta.json` — engine-layer index metadata (§2.2.1).

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::types::{LitecodeError, Result};

use super::embed::production_embedder_id;
use super::{CHUNK_LINES, CHUNK_OVERLAP, EMBED_DIM, EMBEDDER_ID_PASS, MODEL_ID, PIPELINE_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMeta {
    pub model_id: String,
    #[serde(default = "default_embedder_id")]
    pub embedder_id: String,
    pub pipeline_version: u32,
    pub embed_dim: usize,
    pub chunk_lines: usize,
    pub chunk_overlap: usize,
    pub created_at: String,
    pub indexed_files: usize,
    pub indexed_chunks: usize,
}

fn default_embedder_id() -> String {
    EMBEDDER_ID_PASS.into()
}

impl IndexMeta {
    pub fn shell() -> Self {
        Self {
            model_id: MODEL_ID.into(),
            embedder_id: production_embedder_id().into(),
            pipeline_version: PIPELINE_VERSION,
            embed_dim: EMBED_DIM,
            chunk_lines: CHUNK_LINES,
            chunk_overlap: CHUNK_OVERLAP,
            created_at: Utc::now().to_rfc3339(),
            indexed_files: 0,
            indexed_chunks: 0,
        }
    }
}

pub fn index_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("index")
}

pub fn meta_path(workspace_root: &Path) -> PathBuf {
    index_dir(workspace_root).join("meta.json")
}

pub fn needs_rebuild(meta: &IndexMeta) -> bool {
    meta.pipeline_version != PIPELINE_VERSION
        || meta.model_id != MODEL_ID
        || meta.embedder_id != production_embedder_id()
        || meta.embed_dim != EMBED_DIM
        || meta.chunk_lines != CHUNK_LINES
        || meta.chunk_overlap != CHUNK_OVERLAP
}

pub fn read_meta(workspace_root: &Path) -> Result<Option<IndexMeta>> {
    let path = meta_path(workspace_root);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let meta: IndexMeta = serde_json::from_str(&content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))?;
    Ok(Some(meta))
}

pub fn write_meta(workspace_root: &Path, meta: &IndexMeta) -> Result<()> {
    let path = meta_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let body =
        serde_json::to_string_pretty(meta).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(&path, body).map_err(|e| LitecodeError::Config(e.to_string()))
}

/// Init(workspace): create index directory + meta shell; does not load model or build vectors.
pub fn init_workspace_index(workspace_root: &Path) -> Result<()> {
    let dir = index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    if read_meta(workspace_root)?.is_none() {
        write_meta(workspace_root, &IndexMeta::shell())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::{EMBEDDER_ID_HASH, EMBEDDER_ID_PASS};
    use tempfile::TempDir;

    #[test]
    fn init_creates_meta_without_vectors() {
        let dir = TempDir::new().unwrap();
        init_workspace_index(dir.path()).unwrap();
        let meta = read_meta(dir.path()).unwrap().expect("meta");
        assert_eq!(meta.pipeline_version, PIPELINE_VERSION);
        assert_eq!(meta.indexed_chunks, 0);
        assert!(!dir.path().join(".litecode/index/vectors.usearch").exists());
    }

    #[test]
    fn pipeline_version_change_requires_rebuild() {
        let mut meta = IndexMeta::shell();
        assert!(!needs_rebuild(&meta));
        meta.pipeline_version = 0;
        assert!(needs_rebuild(&meta));
    }

    #[test]
    fn embedder_id_mismatch_requires_rebuild() {
        let mut meta = IndexMeta::shell();
        assert!(!needs_rebuild(&meta));
        meta.embedder_id = if production_embedder_id() == EMBEDDER_ID_HASH {
            EMBEDDER_ID_PASS.into()
        } else {
            EMBEDDER_ID_HASH.into()
        };
        assert!(needs_rebuild(&meta));
    }
}
