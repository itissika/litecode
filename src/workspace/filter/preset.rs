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
    /// policy (product-internal dir prune + caller content gates); hidden files
    /// not blanket-skipped.
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
        if !active_workspace_excludes().git_ignore {
            layers.git_ignore = false;
            layers.git_global = false;
            layers.git_exclude = false;
        }
        layers
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
