//! Configure `ignore::WalkBuilder` from a [`FilterPreset`].

use std::path::Path;
use std::sync::Arc;

use ignore::WalkBuilder;

use super::binary::looks_binary;
use super::dirs::is_product_internal_dir_name;
use super::exclude::ExcludeMatcher;
use super::path::{RelPathCtx, cheap_rel_under};
use super::path_glob::{PathGlobMatcher, path_matches_include};
use super::preset::FilterPreset;

/// Optional walk filters beyond the preset (file-level include only).
#[derive(Clone, Default)]
pub struct WalkOptions {
    /// When non-empty, files that do not match are skipped; directories always kept.
    pub file_include: Arc<Vec<PathGlobMatcher>>,
}

impl WalkOptions {
    pub fn with_file_include(matchers: Vec<PathGlobMatcher>) -> Self {
        Self {
            file_include: Arc::new(matchers),
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
        .hidden(layers.hide_hidden)
        .git_ignore(layers.git_ignore)
        .git_global(layers.git_global)
        .git_exclude(layers.git_exclude);

    let matcher = Arc::new(ExcludeMatcher::for_preset(preset));
    let prune_product = preset.prune_product_internal_dirs();
    let skip_binary = layers.skip_binary;
    let root = walk_root.to_path_buf();
    let ctx =
        Arc::new(RelPathCtx::new(walk_root).unwrap_or_else(|_| RelPathCtx::new_lossy(walk_root)));
    let include = options.file_include;
    let need_filter = !matcher.is_empty() || prune_product || skip_binary || !include.is_empty();
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
            )
        });
    }
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
        return false;
    }

    let need_rel = !matcher.is_empty() || !include.is_empty();
    let rel = if need_rel {
        Some(match walk_rel(walk_root, path, ctx) {
            Some(r) => r,
            None => return false,
        })
    } else {
        None
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn text_search_walk_skips_nul_binary() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.rs"), "fn ok() {}\n").unwrap();
        std::fs::write(root.join("bad.bin"), b"hello\x00world").unwrap();

        let files: Vec<String> = walk_builder(root, FilterPreset::TextSearch)
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(|e| {
                crate::workspace::filter::path::rel_path_under(root, e.path())
                    .unwrap_or_else(|| panic!("walk entry must be under root: {:?}", e.path()))
            })
            .collect();
        assert!(files.iter().any(|f| f == "ok.rs"));
        assert!(
            !files.iter().any(|f| f == "bad.bin"),
            "skip_binary layer must drop NUL files; got {files:?}"
        );
    }

    #[test]
    fn file_include_skips_non_matching_files_keeps_dirs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("src/b.txt"), "x\n").unwrap();

        let include = vec![super::super::path_glob::compile_include_pattern("**/*.rs").unwrap()];
        let files: Vec<String> = walk_builder_with(
            root,
            FilterPreset::Unfiltered,
            WalkOptions::with_file_include(include),
        )
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| cheap_rel_under(root, e.path()))
        .collect();
        assert!(files.iter().any(|f| f == "src/a.rs"), "{files:?}");
        assert!(!files.iter().any(|f| f == "src/b.txt"), "{files:?}");
    }

    #[test]
    fn index_walk_excludes_discovery_and_product_not_target() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join(".litecode/index")).unwrap();
        std::fs::write(root.join("src.rs"), "fn s() {}\n").unwrap();
        std::fs::write(root.join("target/foo.rs"), "fn t() {}\n").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x\n").unwrap();
        std::fs::write(root.join(".litecode/index/x.rs"), "fn l() {}\n").unwrap();
        std::fs::create_dir_all(root.join(".data")).unwrap();
        std::fs::write(root.join(".data/foo.rs"), "fn d() {}\n").unwrap();

        let files: Vec<String> = walk_builder(root, FilterPreset::Index)
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .filter_map(|e| cheap_rel_under(root, e.path()))
            .collect();
        assert!(files.iter().any(|f| f == "src.rs"), "{files:?}");
        assert!(
            files.iter().any(|f| f == "target/foo.rs"),
            "target is not a discovery exclude; got {files:?}"
        );
        assert!(
            !files.iter().any(|f| f.contains("node_modules")),
            "{files:?}"
        );
        assert!(!files.iter().any(|f| f.contains(".litecode")), "{files:?}");
        assert!(
            files.iter().any(|f| f == ".data/foo.rs"),
            ".data is not product-internal; got {files:?}"
        );
    }

    fn collect_files(root: &Path, preset: FilterPreset) -> Vec<String> {
        walk_builder(root, preset)
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .filter_map(|e| cheap_rel_under(root, e.path()))
            .collect()
    }

    #[test]
    fn agent_text_skips_nested_litecode_not_walk_root() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".litecode/index")).unwrap();
        std::fs::write(root.join(".litecode/index/x.rs"), "fn l() {}\n").unwrap();
        std::fs::write(root.join("src.rs"), "fn s() {}\n").unwrap();

        let agent = collect_files(root, FilterPreset::AgentText);
        assert!(agent.iter().any(|f| f == "src.rs"), "{agent:?}");
        assert!(
            !agent.iter().any(|f| f.contains(".litecode")),
            "AgentText must hard-skip nested .litecode; got {agent:?}"
        );

        let glob = collect_files(root, FilterPreset::FileGlob);
        assert!(!glob.iter().any(|f| f.contains(".litecode")), "{glob:?}");

        let explorer = collect_files(root, FilterPreset::Explorer);
        assert!(
            explorer.iter().any(|f| f.contains(".litecode")),
            "explorer must still show .litecode; got {explorer:?}"
        );

        let raw = collect_files(root, FilterPreset::Unfiltered);
        assert!(
            raw.iter().any(|f| f.contains(".litecode")),
            "Unfiltered must not prune .litecode; got {raw:?}"
        );

        let nested = collect_files(&root.join(".litecode"), FilterPreset::AgentText);
        assert!(
            nested
                .iter()
                .any(|f| f == "index/x.rs" || f.ends_with("x.rs")),
            "path=.litecode is the walk root and must list; got {nested:?}"
        );
    }

    #[test]
    fn explorer_and_index_split_gitignore_in_walk() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // The ignore crate only honors .gitignore with a git repo marker present.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        std::fs::write(root.join("ignored.rs"), "fn ig() {}\n").unwrap();
        std::fs::write(root.join("visible.rs"), "fn vs() {}\n").unwrap();

        let collect = |preset: FilterPreset| -> Vec<String> {
            walk_builder(root, preset)
                .build()
                .flatten()
                .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
                .filter_map(|e| cheap_rel_under(root, e.path()))
                .collect()
        };

        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        // Defaults: search/index honor gitignore, explorer does not (browse split).
        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            let explorer = collect(FilterPreset::Explorer);
            assert!(
                explorer.iter().any(|f| f == "ignored.rs"),
                "explorer must show gitignored file when explorer_git_ignore=false: {explorer:?}"
            );
            assert!(explorer.iter().any(|f| f == "visible.rs"));

            let index = collect(FilterPreset::Index);
            assert!(
                !index.iter().any(|f| f == "ignored.rs"),
                "index walk must honor gitignore: {index:?}"
            );
            assert!(index.iter().any(|f| f == "visible.rs"));
        });

        // Inverse: explorer honors gitignore, index ignores it.
        with_excludes_cache_for_test(
            WorkspaceExcludesFile {
                git_ignore: false,
                explorer_git_ignore: true,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                let explorer2 = collect(FilterPreset::Explorer);
                assert!(
                    !explorer2.iter().any(|f| f == "ignored.rs"),
                    "explorer must honor gitignore when explorer_git_ignore=true: {explorer2:?}"
                );
                let index2 = collect(FilterPreset::Index);
                assert!(
                    index2.iter().any(|f| f == "ignored.rs"),
                    "index walk must include gitignored file when git_ignore=false: {index2:?}"
                );
            },
        );
    }
}
