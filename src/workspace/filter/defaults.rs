//! Built-in exclude globs — seed for `.litecode/excludes.json` on first open.
//! After seed, the workspace file is the source of truth (Settings can edit it).
//!
//! Sources (tracked against upstream):
//! - VS Code `files.exclude` / `files.watcherExclude`:
//!   `src/vs/workbench/contrib/files/browser/files.contribution.ts`
//! - VS Code `search.exclude`:
//!   `src/vs/workbench/contrib/search/browser/search.contribution.ts`
//! - ripgrep automatic filtering: BurntSushi/ripgrep `GUIDE.md` (gitignore / hidden / binary)
//!
//! Product-owned directory lists below are **not** VS Code; they compose with
//! discovery segments via [`super::dirs`].

/// VS Code default `files.exclude` (desktop; omits web-only `**/*.crswap`).
pub const FILES_EXCLUDE: &[&str] = &[
    "**/.git",
    "**/.svn",
    "**/.hg",
    "**/.DS_Store",
    "**/Thumbs.db",
];

/// VS Code default `search.exclude` (does **not** include `files.exclude`;
/// consumers must union via [`super::preset::search_and_files_exclude_globs`]).
pub const SEARCH_EXCLUDE: &[&str] = &["**/node_modules", "**/bower_components", "**/*.code-search"];

/// VS Code default `files.watcherExclude` (matched relative to workspace root).
pub const WATCHER_EXCLUDE: &[&str] = &[
    // Our own atomic-save temp files (see `WorkspaceService::atomic_write`).
    "*.litecode-tmp*",
    ".git/objects/**",
    ".git/subtree-cache/**",
    ".hg/store/**",
    "*/.git/objects/**",
    "*/.git/subtree-cache/**",
    "*/.hg/store/**",
    // Product / eval trees (not VS Code). Seeded into new `excludes.json`.
    // Nested `.litecode` is also hard-gated via [`PRODUCT_INTERNAL_DIRS`].
    "**/.litecode/**",
    "**/.data/**",
    "**/.venv-ort/**",
];

/// Product-owned directory basenames (not VS Code, not user seed).
///
/// `.litecode` is the workspace runtime tree: Index / snapshot / Agent discovery
/// walks hard-skip nested copies. User eval dirs such as `.data` / `.venv-ort`
/// belong in that repo's `.gitignore`, not this product list.
pub const PRODUCT_INTERNAL_DIRS: &[&str] = &[".litecode"];

/// Snapshot-only heavy trees (Unity / IDE / common build outputs).
///
/// Composed on top of discovery segments ∪ [`PRODUCT_INTERNAL_DIRS`] — do not
/// re-list `.git` / `node_modules` / `.litecode` here.
pub const SNAPSHOT_ONLY_DIRS: &[&str] = &[
    "target",
    "dist",
    "Library",
    "library",
    "Temp",
    "temp",
    "Logs",
    "logs",
    "Obj",
    "obj",
    "Build",
    "Builds",
    "UserSettings",
    "MemoryCaptures",
    "Recordings",
    "__pycache__",
    ".venv",
    ".cursor",
    ".vscode",
    ".vs",
    ".opencode",
    ".codebuddy",
    ".trae",
    ".sisyphus",
    ".codeartsdoer",
    ".gradle",
];
