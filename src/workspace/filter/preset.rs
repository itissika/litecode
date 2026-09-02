//! Named presets: fixed compositions of [`super::layers::FilterLayers`].
//!
//! Three user-facing faces (VS Code): Explorer, Search, Watcher. Plus
//! [`FilterPreset::Unfiltered`] for explicit `no_ignore`.

use super::layers::FilterLayers;
use super::workspace_excludes::active_workspace_excludes;

/// Consumer-facing filter presets. Semantics follow VS Code — not ad-hoc denylists.
///
/// Path trust (ALL/SAFE) is orthogonal: presets only shape discovery corpora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPreset {
    /// File tree: `files.exclude`. Gitignore only when `explorer_git_ignore`.
    Explorer,
    /// Zero walk filters — escape hatch for explicit `no_ignore` discovery.
    /// Not the Agent default; known-path read/write never uses walk presets.
    Unfiltered,
    /// Search line (human text, Agent grep/glob, text + vector index):
    /// `files.exclude ∪ search.exclude` + `git_ignore`. Hidden files are not
    /// skipped here (VS Code ripgrep `--hidden`); gitignore decides.
    Search,
    /// OS watcher event gate: `files.watcherExclude` only. Hard cut; consumers
    /// do not get events this preset drops.
    Watcher,
}

impl FilterPreset {
    pub fn layers(self) -> FilterLayers {
        let mut layers = match self {
            Self::Explorer => FilterLayers {
                files_exclude: true,
                search_exclude: false,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                skip_binary: false,
            },
            Self::Unfiltered => FilterLayers::NONE,
            Self::Search => FilterLayers {
                files_exclude: true,
                search_exclude: true,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                skip_binary: true,
            },
            Self::Watcher => FilterLayers {
                files_exclude: false,
                search_exclude: false,
                watcher_exclude: true,
                git_ignore: false,
                git_global: false,
                git_exclude: false,
                skip_binary: false,
            },
        };
        let cfg = active_workspace_excludes();
        // Browse-only split: explorer honors `.gitignore` independently from
        // the search corpora switch (`git_ignore`). Watcher / Unfiltered
        // layers already bake `git_ignore: false`; the override only ever forces
        // layers off, never on.
        let honor_git_ignore = if self == Self::Explorer {
            cfg.explorer_git_ignore
        } else {
            cfg.git_ignore
        };
        if !honor_git_ignore {
            layers.git_ignore = false;
            layers.git_global = false;
            layers.git_exclude = false;
        }
        layers
    }

    /// Hard-skip nested `.litecode` (product runtime). Explorer stays visible;
    /// `Unfiltered` (`no_ignore`) does not prune. Never prune the walk root so
    /// `path=.litecode` still lists.
    pub fn prune_product_internal_dirs(self) -> bool {
        !matches!(self, Self::Explorer | Self::Unfiltered)
    }
}

/// VS Code `getExcludes(includeSearchExcludes=true)`: files ∪ search.
pub fn search_and_files_exclude_globs() -> Vec<String> {
    let cfg = active_workspace_excludes();
    let mut out = Vec::with_capacity(cfg.files_exclude.len() + cfg.search_exclude.len());
    out.extend(cfg.files_exclude);
    out.extend(cfg.search_exclude);
    out
}

/// Glob patterns active for a preset's exclude expression (path matching).
pub fn exclude_globs(preset: FilterPreset) -> Vec<String> {
    let cfg = active_workspace_excludes();
    let layers = preset.layers();
    let mut out = Vec::new();
    if layers.watcher_exclude {
        out.extend(cfg.watcher_exclude);
    }
    if layers.search_exclude {
        out.extend(search_and_files_exclude_globs());
    } else if layers.files_exclude {
        out.extend(cfg.files_exclude);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

    fn with_config(cfg: WorkspaceExcludesFile, f: impl FnOnce()) {
        with_excludes_cache_for_test(cfg, f);
    }

    fn git_layers(preset: FilterPreset) -> (bool, bool, bool) {
        let l = preset.layers();
        (l.git_ignore, l.git_global, l.git_exclude)
    }

    #[test]
    fn browse_search_split_explorer_reads_own_switch() {
        with_config(
            WorkspaceExcludesFile {
                git_ignore: true,
                explorer_git_ignore: false,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                assert_eq!(git_layers(FilterPreset::Explorer), (false, false, false));
                assert_eq!(git_layers(FilterPreset::Search), (true, true, true));
            },
        );
    }

    #[test]
    fn browse_search_split_inverse() {
        with_config(
            WorkspaceExcludesFile {
                git_ignore: false,
                explorer_git_ignore: true,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                assert_eq!(git_layers(FilterPreset::Explorer), (true, true, true));
                assert_eq!(git_layers(FilterPreset::Search), (false, false, false));
            },
        );
    }

    #[test]
    fn watcher_and_unfiltered_never_honor_gitignore() {
        with_config(
            WorkspaceExcludesFile {
                git_ignore: true,
                explorer_git_ignore: true,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                assert_eq!(git_layers(FilterPreset::Watcher), (false, false, false));
                assert_eq!(git_layers(FilterPreset::Unfiltered), (false, false, false));
            },
        );
    }

    #[test]
    fn prune_product_internal_presets() {
        assert!(!FilterPreset::Explorer.prune_product_internal_dirs());
        assert!(!FilterPreset::Unfiltered.prune_product_internal_dirs());
        assert!(FilterPreset::Search.prune_product_internal_dirs());
        assert!(FilterPreset::Watcher.prune_product_internal_dirs());
    }
}
