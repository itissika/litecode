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
    let contract = resolved.contract();
    let claude_md = if contract.is_empty() {
        None
    } else {
        Some(contract.to_string())
    };
    let agents_md = read_agents_md(cwd);

    Context {
        cwd: cwd.to_path_buf(),
        workspace_paths: workspace_paths.clone(),
        agents_md,
        claude_md,
    }
}

/// Legacy compatibility: read `AGENTS.md` from workspace root when present.
fn read_agents_md(workspace_root: &Path) -> Option<String> {
    let path = workspace_root.join("AGENTS.md");
    std::fs::read_to_string(&path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::GlobalSettings;

    #[test]
    fn build_context_maps_resolved_contract_to_claude_md() {
        let mut workspace = WorkspaceState::new("/tmp/project");
        workspace.contract = "# workspace contract\n\nFollow these rules.".into();
        let resolved = resolve(GlobalSettings::default(), workspace);
        let paths = WorkspacePaths::for_legacy_root(&std::path::PathBuf::from("/tmp/project"));

        let ctx = build_context(&resolved, std::path::Path::new("/tmp/project"), &paths);

        assert_eq!(
            ctx.claude_md.as_deref(),
            Some("# workspace contract\n\nFollow these rules.")
        );
    }

    #[test]
    fn build_context_omits_claude_md_when_contract_empty() {
        let resolved = resolve(GlobalSettings::default(), WorkspaceState::new("/tmp/empty"));
        let paths = WorkspacePaths::for_legacy_root(&std::path::PathBuf::from("/tmp/empty"));

        let ctx = build_context(&resolved, std::path::Path::new("/tmp/empty"), &paths);

        assert!(ctx.claude_md.is_none());
    }
}
