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
    // Product runtime tree (also hard-gated via [`PRODUCT_INTERNAL_DIRS`] on Search walks).
    "**/.litecode/**",
];

/// Product-owned directory basenames (not VS Code, not user seed).
///
/// `.litecode` is the workspace runtime tree: Index / snapshot / Agent discovery
/// walks hard-skip nested copies. Extra heavy folders belong in that repo's
/// `.gitignore` or `excludes.json`, not this product list.
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

/// Suffixes the code semantic index includes when `excludes.json` omits
/// `code_index_extensions`. A missing field falls back to this table. An
/// empty list in the file means the index takes no source files.
///
/// Match is the last filename segment (`Button.tsx` → `tsx`, `types.d.ts` →
/// `ts`). Documents and lockfiles (`json`, `md`, `txt`, `lock`, `csv`) and
/// extensionless names are not in this table.
pub const CODE_INDEX_EXTENSIONS: &[&str] = &[
    "bash", "c", "cc", "cjs", "clj", "cljc", "cljs", "cpp", "cs", "css", "cts", "cxx", "dart",
    "elm", "ex", "exs", "fs", "fsi", "fsx", "go", "gql", "graphql", "h", "hh", "hpp", "hs", "htm",
    "html", "hxx", "java", "js", "jsx", "kt", "kts", "less", "lua", "m", "ml", "mli", "mm", "mjs",
    "mts", "nim", "php", "pl", "pm", "proto", "ps1", "psm1", "py", "pyi", "pyw", "r", "rb", "rs",
    "scala", "scss", "sh", "sql", "svelte", "swift", "ts", "tsx", "v", "vue", "zig", "zsh",
];

/// Dependency directories the code index never walks, even when their
/// suffixes are allowlisted. This is not a user exclude pattern.
pub const CODE_INDEX_SKIP_DIRS: &[&str] = &["node_modules", "bower_components"];
