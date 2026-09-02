//! Thresholds and path gates for the adaptive text index.

use crate::workspace::filter::{
    FilterPreset, active_workspace_excludes, is_workspace_excludes_rel, path_excluded,
    path_has_product_internal_dir,
};

/// Refuse to build above this (protect against $HOME-sized roots).
pub const HARD_FILE_CAP: u64 = 200_000;
/// Skip putting individual files larger than this into Tantivy; keep the path
/// as an oversized sidecar so grep still verifies them (superset, never a miss).
pub const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextIndexMode {
    Auto,
    On,
    Off,
}

/// `LITECODE_TEXT_INDEX=auto|on|off` (default auto).
pub fn mode_from_env() -> TextIndexMode {
    match std::env::var("LITECODE_TEXT_INDEX")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "on" | "1" | "true" => TextIndexMode::On,
        "off" | "0" | "false" => TextIndexMode::Off,
        _ => TextIndexMode::Auto,
    }
}

/// Auto and On both build whenever the AgentText corpus fits under the cap.
pub fn should_build(mode: TextIndexMode, file_count: u64) -> bool {
    match mode {
        TextIndexMode::Off => false,
        TextIndexMode::On | TextIndexMode::Auto => file_count <= HARD_FILE_CAP,
    }
}

/// Fingerprint of the AgentText corpus definition. A mismatch means rebuild.
pub fn corpus_fingerprint() -> String {
    let cfg = active_workspace_excludes();
    let mut files = cfg.files_exclude.clone();
    files.sort();
    let mut search = cfg.search_exclude.clone();
    search.sort();
    format!(
        "v1\nfiles:{}\nsearch:{}\ngit_ignore:{}",
        files.join("\x1f"),
        search.join("\x1f"),
        cfg.git_ignore
    )
}

/// Ignore-rule / excludes files define the corpus; a change must rebuild, not
/// incrementally reindex the file as content.
pub fn is_corpus_definition_rel(rel: &str) -> bool {
    let rel = rel.trim_start_matches("./").replace('\\', "/");
    if is_workspace_excludes_rel(&rel) {
        return true;
    }
    if rel == ".git/info/exclude" || rel.ends_with("/.git/info/exclude") {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel.as_str());
    matches!(name, ".gitignore" | ".ignore" | ".fdignore" | ".rgignore")
}

/// Queue gate for text-index incremental updates (AgentText discovery face).
pub fn should_queue_text_path(rel: &str, deleted: bool) -> bool {
    if path_has_product_internal_dir(rel) {
        return false;
    }
    if path_excluded(rel, FilterPreset::AgentText) {
        // Still queue deletes so a path that later matches excludes is dropped.
        return deleted;
    }
    true
}

/// Whether this relative path is skipped by `preset` at query time.
pub fn path_skipped_by_preset(rel: &str, preset: FilterPreset, hide_hidden: bool) -> bool {
    if path_has_product_internal_dir(rel) {
        return true;
    }
    if path_excluded(rel, preset) {
        return true;
    }
    hide_hidden && path_has_hidden_component(rel)
}

fn path_has_hidden_component(rel: &str) -> bool {
    rel.split(['/', '\\'])
        .any(|c| c.starts_with('.') && c != "." && c != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_skips_node_modules_and_litecode() {
        assert!(!should_queue_text_path("node_modules/x.js", false));
        assert!(!should_queue_text_path(".litecode/text-index/x", false));
        assert!(!should_queue_text_path(".git/config", false));
        assert!(should_queue_text_path("src/main.rs", false));
        assert!(should_queue_text_path("src/main.rs", true));
        // target/ is NOT discovery-excluded (same as AgentText/grep); only gitignore drops it.
        assert!(should_queue_text_path("target/foo.rs", false));
        assert!(should_queue_text_path(".data/foo.rs", false));
        // Excluded path: delete still queues so the posting can be removed.
        assert!(should_queue_text_path("node_modules/x.js", true));
    }

    #[test]
    fn queue_gate_matches_agent_text_exclude_matcher() {
        use crate::workspace::filter::path_excluded;
        for rel in [
            "node_modules/pkg/index.js",
            "bower_components/x",
            ".git/HEAD",
            ".svn/entries",
            ".DS_Store",
        ] {
            assert!(
                path_excluded(rel, FilterPreset::AgentText),
                "{rel} should be AgentText-excluded"
            );
            assert!(
                !should_queue_text_path(rel, false),
                "{rel} must not enter text-index queue on create"
            );
        }
    }

    #[test]
    fn corpus_definition_paths() {
        assert!(is_corpus_definition_rel(".gitignore"));
        assert!(is_corpus_definition_rel("nested/.gitignore"));
        assert!(is_corpus_definition_rel(".litecode/excludes.json"));
        assert!(!is_corpus_definition_rel("src/main.rs"));
    }

    #[test]
    fn auto_builds_small_workspaces() {
        assert!(should_build(TextIndexMode::Auto, 1));
        assert!(should_build(TextIndexMode::Auto, HARD_FILE_CAP));
        assert!(!should_build(TextIndexMode::Auto, HARD_FILE_CAP + 1));
        assert!(!should_build(TextIndexMode::Off, 10));
    }

    #[test]
    fn hidden_skip_for_text_search() {
        assert!(path_skipped_by_preset(
            ".env",
            FilterPreset::TextSearch,
            true
        ));
        assert!(!path_skipped_by_preset(
            ".env",
            FilterPreset::AgentText,
            false
        ));
        assert!(path_skipped_by_preset(
            "src/.secret/a.rs",
            FilterPreset::TextSearch,
            true
        ));
    }
}
