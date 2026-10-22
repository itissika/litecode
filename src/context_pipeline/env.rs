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
    use super::*;
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::GlobalSettings;

    #[test]
    fn build_context_copies_process_snapshot_and_ignores_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# disk claude\n").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# disk agents\n").unwrap();
        let mut workspace = WorkspaceState::new(dir.path());
        workspace.contract = "# process claude\n".into();
        workspace.agents_md = "# process agents\n".into();
        let resolved = resolve_without_catalog(GlobalSettings::default(), workspace);
        let paths = WorkspacePaths::for_legacy_root(dir.path());

        let ctx = build_context(&resolved, dir.path(), &paths);

        assert_eq!(ctx.claude_md.as_deref(), Some("# process claude\n"));
        assert_eq!(ctx.agents_md.as_deref(), Some("# process agents\n"));
    }
}
