//! Named presets: fixed compositions of [`super::layers::FilterLayers`].

use super::layers::FilterLayers;
use super::workspace_excludes::active_workspace_excludes;

/// Consumer-facing filter presets. Semantics follow VS Code / ripgrep / product
/// index policy — not ad-hoc denylists.
///
/// Path trust (ALL/SAFE) is orthogonal: presets only shape discovery corpora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPreset {
    /// File tree: `files.exclude` + gitignore (hidden shown).
    Explorer,
    /// Zero walk filters — escape hatch for explicit `no_ignore` discovery.
    /// Not the Agent default; known-path read/write never uses walk presets.
    Unfiltered,
    /// Human workspace text search: files∪search exclude + gitignore + hide
    /// hidden + binary skip (ripgrep + VS Code Search defaults).
    TextSearch,
    /// File name discovery (Agent `glob` default / VS Code `findFiles`):
    /// files∪search exclude + gitignore; hidden files shown.
    FileGlob,
    /// Agent content discovery (`grep` default): [`FileGlob`] layers plus
    /// binary skip; does **not** hide_hidden (so un-ignored `.env` remains).
    AgentText,
    /// OS watcher event gate: `files.watcherExclude` only.
    Watcher,
    /// Semantic index scan: search-style excludes + gitignore + index content
    /// gates at callers; `.litecode` pruned via [`Self::prune_product_internal_dirs`].
    Index,
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
                hide_hidden: false,
                skip_binary: false,
                index_content: false,
            },
            Self::Unfiltered => FilterLayers::NONE,
            Self::TextSearch => FilterLayers {
                files_exclude: true,
                search_exclude: true,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                hide_hidden: true,
                skip_binary: true,
                index_content: false,
            },
            Self::FileGlob => FilterLayers {
                files_exclude: true,
                search_exclude: true,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                hide_hidden: false,
                skip_binary: false,
                index_content: false,
            },
            Self::AgentText => FilterLayers {
                files_exclude: true,
                search_exclude: true,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                hide_hidden: false,
                skip_binary: true,
                index_content: false,
            },
            Self::Watcher => FilterLayers {
                files_exclude: false,
                search_exclude: false,
                watcher_exclude: true,
                git_ignore: false,
                git_global: false,
                git_exclude: false,
                hide_hidden: false,
                skip_binary: false,
                index_content: false,
            },
            Self::Index => FilterLayers {
                files_exclude: true,
                search_exclude: true,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                // Match prior `scannable_files` WalkBuilder (hidden=false).
                hide_hidden: false,
                skip_binary: true,
                index_content: true,
            },
        };
        let cfg = active_workspace_excludes();
        // Browse-only split: the explorer honors `.gitignore` independently from
        // the search / index corpora switch (`git_ignore`). Watcher / Unfiltered
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
        // Search/index side honors gitignore, explorer does not: the split the
        // user asked for (browse independently).
        with_config(
            WorkspaceExcludesFile {
                git_ignore: true,
                explorer_git_ignore: false,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                assert_eq!(git_layers(FilterPreset::Explorer), (false, false, false));
                assert_eq!(git_layers(FilterPreset::Index), (true, true, true));
                assert_eq!(git_layers(FilterPreset::TextSearch), (true, true, true));
                assert_eq!(git_layers(FilterPreset::FileGlob), (true, true, true));
                assert_eq!(git_layers(FilterPreset::AgentText), (true, true, true));
            },
        );
    }

    #[test]
    fn browse_search_split_inverse() {
        // Explorer honors gitignore while search/index ignores it: switches are
        // fully independent in both directions.
        with_config(
            WorkspaceExcludesFile {
                git_ignore: false,
                explorer_git_ignore: true,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                assert_eq!(git_layers(FilterPreset::Explorer), (true, true, true));
                assert_eq!(git_layers(FilterPreset::Index), (false, false, false));
                assert_eq!(git_layers(FilterPreset::TextSearch), (false, false, false));
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
        assert!(FilterPreset::AgentText.prune_product_internal_dirs());
        assert!(FilterPreset::FileGlob.prune_product_internal_dirs());
        assert!(FilterPreset::TextSearch.prune_product_internal_dirs());
        assert!(FilterPreset::Index.prune_product_internal_dirs());
        assert!(FilterPreset::Watcher.prune_product_internal_dirs());
    }
}
