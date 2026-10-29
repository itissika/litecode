//! Per-workspace human trust for custom app definitions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::registry::AppRegistry;
use crate::config::path::strip_verbatim;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    #[serde(default)]
    workspaces: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for TrustFile {
    fn default() -> Self {
        Self {
            version: 1,
            workspaces: BTreeMap::new(),
        }
    }
}

fn workspace_key(workspace_root: &Path) -> String {
    let canonical = std::fs::canonicalize(workspace_root)
        .map(strip_verbatim)
        .unwrap_or_else(|_| strip_verbatim(workspace_root.to_path_buf()));
    canonical.to_string_lossy().to_string()
}

fn read_trust_file(path: &Path) -> TrustFile {
    let Ok(text) = std::fs::read_to_string(path) else {
        return TrustFile::default();
    };
    let mut file: TrustFile = serde_json::from_str(&text).unwrap_or_default();
    if file.version != 1 {
        file = TrustFile::default();
    }
    file
}

pub fn is_trusted(registry: &AppRegistry, app_id: &str, hash: &str) -> bool {
    let file = read_trust_file(&registry.trust_file());
    let workspace = workspace_key(&registry.workspace_root);
    file.workspaces
        .get(&workspace)
        .and_then(|apps| apps.get(app_id))
        .map(|saved| saved.eq_ignore_ascii_case(hash))
        .unwrap_or(false)
}

pub fn trust_app(registry: &AppRegistry, app_id: &str, hash: &str) -> Result<(), String> {
    let path: PathBuf = registry.trust_file();
    let mut file = read_trust_file(&path);
    file.version = 1;
    let workspace = workspace_key(&registry.workspace_root);
    file.workspaces
        .entry(workspace)
        .or_default()
        .insert(app_id.to_string(), hash.to_ascii_lowercase());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create trust directory: {e}"))?;
    }
    let text =
        serde_json::to_string_pretty(&file).map_err(|e| format!("serialize trust file: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("write trust file: {e}"))
}
