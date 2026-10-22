//! Named presets: fixed compositions of [`super::layers::FilterLayers`].
//!
//! Three user-facing faces (VS Code): Explorer, Search, Watcher. Plus
//! [`FilterPreset::Unfiltered`] (tests / raw walks) and [`FilterPreset::NoIgnore`]
//! (agent grep/glob `-u`).

use super::layers::FilterLayers;
use super::workspace_excludes::active_workspace_excludes;

/// Consumer-facing filter presets. Semantics follow VS Code — not ad-hoc denylists.
///
/// Path trust (ALL/SAFE) is orthogonal: presets only shape discovery corpora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterPreset {
    /// File tree: `files.exclude`. Gitignore only when `explorer_git_ignore`.
    Explorer,
    /// Zero walk filters. Tests and raw discovery; not the Agent `-u` hatch.
    /// Known-path read/write never uses walk presets.
    Unfiltered,
    /// Agent grep/glob `-u`: off gitignore / files.exclude / search.exclude,
    /// still skip binary and prune nested `.litecode`.
    NoIgnore,
    /// Search line (human text, Agent grep/glob, text index):
    /// `files.exclude ∪ search.exclude` + `git_ignore`. Hidden files are not
    /// skipped here (VS Code ripgrep `--hidden`); gitignore decides.
    /// The code semantic index uses [`FilterPreset::CodeIndex`].
    Search,
    /// Code semantic index: `files.exclude` + `git_ignore`, not
    /// `search.exclude`. `node_modules` and `bower_components` are always
    /// skipped. Suffixes are a separate allowlist applied by the scan.
    CodeIndex,
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
            Self::NoIgnore => FilterLayers {
                files_exclude: false,
                search_exclude: false,
                watcher_exclude: false,
                git_ignore: false,
                git_global: false,
                git_exclude: false,
                skip_binary: true,
            },
            Self::Search => FilterLayers {
                files_exclude: true,
                search_exclude: true,
                watcher_exclude: false,
                git_ignore: true,
                git_global: true,
                git_exclude: true,
                skip_binary: true,
            },
            Self::CodeIndex => FilterLayers {
                files_exclude: true,
                search_exclude: false,
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
        // the search corpora switch (`git_ignore`). Watcher / Unfiltered /
        // NoIgnore layers already bake `git_ignore: false`; the override only
        // ever forces layers off, never on.
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
    /// `Unfiltered` does not prune. `NoIgnore` (agent `-u`) still prunes nested
    /// `.litecode`. Never prune the walk root so `path=.litecode` still lists.
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
    if preset == FilterPreset::CodeIndex {
        for dir in super::defaults::CODE_INDEX_SKIP_DIRS {
            out.push(format!("**/{dir}"));
        }
    }
    out
}
