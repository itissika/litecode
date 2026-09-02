//! Orthogonal filter capability switches (VS Code / ripgrep philosophy).

/// Which ignore / exclude layers to apply. Presets compose these; callers may
/// also build custom layer sets without inventing new glob tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterLayers {
    /// Workspace `files.exclude` list.
    pub files_exclude: bool,
    /// Workspace `search.exclude` list (always composed with `files_exclude`
    /// when both are on — same as VS Code `getExcludes`).
    pub search_exclude: bool,
    /// VS Code `files.watcherExclude` defaults.
    pub watcher_exclude: bool,
    /// Respect `.gitignore` (via `ignore` crate).
    pub git_ignore: bool,
    /// Respect global git excludes file.
    pub git_global: bool,
    /// Respect `$GIT_DIR/info/exclude`.
    pub git_exclude: bool,
    /// Skip files that look binary (NUL in first 8 KiB). Search-line content
    /// gate, not a fourth exclude list.
    pub skip_binary: bool,
}

impl FilterLayers {
    pub const NONE: Self = Self {
        files_exclude: false,
        search_exclude: false,
        watcher_exclude: false,
        git_ignore: false,
        git_global: false,
        git_exclude: false,
        skip_binary: false,
    };
}
