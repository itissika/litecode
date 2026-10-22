//! Configure `ignore::WalkBuilder` from a [`FilterPreset`].

use std::path::Path;
use std::sync::Arc;

use ignore::WalkBuilder;

use super::binary::looks_binary;
use super::dirs::{is_agent_knowledge_rel, is_product_internal_dir_name};
use super::exclude::ExcludeMatcher;
use super::path::{RelPathCtx, cheap_rel_under};
use super::path_glob::{PathGlobMatcher, path_matches_include};
use super::preset::FilterPreset;

/// Optional walk filters beyond the preset (file-level include only).
#[derive(Clone, Default)]
pub struct WalkOptions {
    /// When non-empty, files that do not match are skipped; directories always kept.
    pub file_include: Arc<Vec<PathGlobMatcher>>,
    /// Override [`super::layers::FilterLayers::skip_binary`]. `None` uses the preset.
    pub skip_binary: Option<bool>,
    /// Agent `grep` / `glob` may read `knowledge/` under `.litecode`.
    /// The code index and the watcher leave this false.
    pub allow_knowledge_root: bool,
}

impl WalkOptions {
    pub fn with_file_include(matchers: Vec<PathGlobMatcher>) -> Self {
        Self {
            file_include: Arc::new(matchers),
            skip_binary: None,
            allow_knowledge_root: false,
        }
    }
}

/// Apply preset layers to an existing [`WalkBuilder`] rooted at `walk_root`.
pub fn configure_walk(builder: &mut WalkBuilder, walk_root: &Path, preset: FilterPreset) {
    configure_walk_with(builder, walk_root, preset, WalkOptions::default());
}

/// Like [`configure_walk`], with optional file-include prefilter.
pub fn configure_walk_with(
    builder: &mut WalkBuilder,
    walk_root: &Path,
    preset: FilterPreset,
    options: WalkOptions,
) {
    let layers = preset.layers();
    builder
        // VS Code search passes ripgrep `--hidden`; gitignore is the ignore axis.
        .hidden(false)
        .git_ignore(layers.git_ignore)
        .git_global(layers.git_global)
        .git_exclude(layers.git_exclude);

    let matcher = Arc::new(ExcludeMatcher::for_preset(preset));
    let prune_product = preset.prune_product_internal_dirs();
    let skip_binary = options.skip_binary.unwrap_or(layers.skip_binary);
    let allow_knowledge_root = options.allow_knowledge_root;
    let root = walk_root.to_path_buf();
    let ctx =
        Arc::new(RelPathCtx::new(walk_root).unwrap_or_else(|_| RelPathCtx::new_lossy(walk_root)));
    let include = options.file_include;
    let need_filter = !matcher.is_empty()
        || prune_product
        || skip_binary
        || !include.is_empty()
        || allow_knowledge_root;
    if need_filter {
        builder.filter_entry(move |entry| {
            keep_entry(
                entry,
                &root,
                &ctx,
                &matcher,
                &include,
                prune_product,
                skip_binary,
                allow_knowledge_root,
            )
        });
    }
}

/// Files under `<workspace>/.litecode/knowledge`.
///
/// `.gitignore` hides `.litecode/`, and a whitelist override would hide every
/// other file, so this walk turns gitignore off and stays inside that folder.
pub fn private_knowledge_walker(
    workspace: &Path,
    preset: FilterPreset,
    skip_binary: Option<bool>,
) -> Option<WalkBuilder> {
    let dir = workspace.join(".litecode").join("knowledge");
    if !dir.is_dir() {
        return None;
    }
    let options = WalkOptions {
        skip_binary,
        allow_knowledge_root: true,
        ..WalkOptions::default()
    };
    let mut builder = walk_builder_with(&dir, preset, options);
    builder
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);
    Some(builder)
}

/// Build a walker rooted at `root` for `preset`.
pub fn walk_builder(root: &Path, preset: FilterPreset) -> WalkBuilder {
    walk_builder_with(root, preset, WalkOptions::default())
}

/// Build a walker with optional file-include prefilter.
pub fn walk_builder_with(root: &Path, preset: FilterPreset, options: WalkOptions) -> WalkBuilder {
    // Prefer LAP root so DirEntry paths strip_prefix cleanly (esp. Windows).
    let ctx = RelPathCtx::new(root).unwrap_or_else(|_| RelPathCtx::new_lossy(root));
    let lap = ctx.root_lap();
    let mut builder = WalkBuilder::new(lap);
    configure_walk_with(&mut builder, lap, preset, options);
    builder
}

fn keep_entry(
    entry: &ignore::DirEntry,
    walk_root: &Path,
    ctx: &RelPathCtx,
    matcher: &ExcludeMatcher,
    include: &[PathGlobMatcher],
    prune_product: bool,
    skip_binary: bool,
    allow_knowledge_root: bool,
) -> bool {
    let path = entry.path();
    let file_type = entry.file_type();
    let is_dir = file_type.is_some_and(|t| t.is_dir());
    let is_file = file_type.is_some_and(|t| t.is_file());

    if prune_product
        && is_dir
        && let Some(name) = path.file_name().and_then(|n| n.to_str())
        && is_product_internal_dir_name(name)
        && !is_walk_root(walk_root, path)
    {
        let rel = walk_rel(walk_root, path, ctx).unwrap_or_default();
        if !(allow_knowledge_root && rel == ".litecode") {
            return false;
        }
    }

    let need_rel = !matcher.is_empty() || !include.is_empty() || allow_knowledge_root;
    let rel = if need_rel {
        Some(match walk_rel(walk_root, path, ctx) {
            Some(r) => r,
            None => return false,
        })
    } else {
        None
    };

    if allow_knowledge_root {
        let rel = rel.as_deref().unwrap_or("");
        if hides_litecode_entry(walk_root, rel) {
            return false;
        }
    }

    if !matcher.is_empty() {
        let rel = rel.as_deref().unwrap_or("");
        if matcher.matches(rel) {
            return false;
        }
    }

    // File include: directories always kept so children can be walked.
    if !include.is_empty() && is_file {
        let rel = rel.as_deref().unwrap_or("");
        if !path_matches_include(rel, include) {
            return false;
        }
    }

    // Ripgrep-style binary skip: directories always kept so children can be walked.
    if skip_binary && is_file && looks_binary(path) {
        return false;
    }

    true
}

fn walk_rel(walk_root: &Path, path: &Path, ctx: &RelPathCtx) -> Option<String> {
    cheap_rel_under(walk_root, path).or_else(|| ctx.rel(path))
}

/// Drop `.litecode` except the knowledge root.
/// A walk that starts at `.litecode` keeps only its `knowledge` child.
fn hides_litecode_entry(walk_root: &Path, rel: &str) -> bool {
    let rel = rel.trim_matches('/');
    if rel == ".litecode" || rel.starts_with(".litecode/") {
        return !is_agent_knowledge_rel(rel);
    }
    if walk_root.file_name().and_then(|name| name.to_str()) == Some(".litecode") {
        return !(rel.is_empty() || rel == "knowledge" || rel.starts_with("knowledge/"));
    }
    false
}

fn is_walk_root(walk_root: &Path, path: &Path) -> bool {
    path == walk_root
        || cheap_rel_under(walk_root, path)
            .as_deref()
            .is_some_and(str::is_empty)
}

/// When walking a subdirectory (e.g. tree listing), match excludes against
/// paths relative to `workspace_root`.
pub fn configure_walk_under(
    builder: &mut WalkBuilder,
    workspace_root: &Path,
    _dir: &Path,
    preset: FilterPreset,
) {
    configure_walk(builder, workspace_root, preset);
}
