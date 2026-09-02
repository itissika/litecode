//! On-disk meta for `.litecode/text-index/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::types::{LitecodeError, Result};

pub const INDEX_FORMAT: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextIndexMeta {
    pub format: u32,
    pub workspace_root: String,
    pub file_count: u64,
    pub built_unix_ms: u64,
    #[serde(default)]
    pub corpus_fingerprint: String,
    /// AgentText files too large for Tantivy; still verified on every grep.
    #[serde(default)]
    pub oversized: Vec<String>,
}

pub fn text_index_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("text-index")
}

pub fn meta_path(workspace_root: &Path) -> PathBuf {
    text_index_dir(workspace_root).join("meta.json")
}

pub fn tantivy_dir(workspace_root: &Path) -> PathBuf {
    text_index_dir(workspace_root).join("tantivy")
}

pub fn load_meta(workspace_root: &Path) -> Result<Option<TextIndexMeta>> {
    let path = meta_path(workspace_root);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let meta: TextIndexMeta =
        serde_json::from_str(&raw).map_err(|e| LitecodeError::Config(e.to_string()))?;
    Ok(Some(meta))
}

pub fn save_meta(workspace_root: &Path, meta: &TextIndexMeta) -> Result<()> {
    let dir = text_index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let path = meta_path(workspace_root);
    let raw =
        serde_json::to_string_pretty(meta).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(path, raw).map_err(|e| LitecodeError::Config(e.to_string()))?;
    Ok(())
}
