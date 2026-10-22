//! Runtime context for LLM / tool / hook assembly.
//!
//! Holds workspace paths and instruction-file content used when building the
//! system prompt. This is not session durable state — it lives with the context
//! pipeline.

use std::path::{Path, PathBuf};

use crate::config::{ResolvedConfig, WorkspacePaths};

#[derive(Debug, Clone)]
pub struct Context {
    pub cwd: PathBuf,
    pub workspace_paths: WorkspacePaths,
    pub agents_md: Option<String>,
    pub claude_md: Option<String>,
}

pub fn build_context(
    resolved: &ResolvedConfig,
    cwd: &Path,
    workspace_paths: &WorkspacePaths,
) -> Context {
    // Both files are the process snapshot on `ResolvedConfig`. This function
    // does not touch the disk: a mid-process edit must not move the prompt prefix.
    Context {
        cwd: cwd.to_path_buf(),
        workspace_paths: workspace_paths.clone(),
        agents_md: non_empty(resolved.agents_md()),
        claude_md: non_empty(resolved.contract()),
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    
    
    

}
