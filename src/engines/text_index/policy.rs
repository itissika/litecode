//! Thresholds and path gates for the adaptive text index.

use crate::workspace::filter::{FilterPreset, path_excluded, path_has_product_internal_dir};

/// Build when discovery corpus file count reaches this (auto mode).
pub const BUILD_FILE_THRESHOLD: u64 = 1_000;
/// Drop on-disk index when count falls below this (hysteresis).
pub const DROP_FILE_THRESHOLD: u64 = 750;
/// Refuse to build above this (protect against $HOME-sized roots).
pub const HARD_FILE_CAP: u64 = 200_000;
/// Skip indexing individual files larger than this.
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

/// Queue gate for text-index incremental updates (AgentText discovery face).
pub fn should_queue_text_path(rel: &str, deleted: bool) -> bool {
    if path_has_product_internal_dir(rel) || path_excluded(rel, FilterPreset::AgentText) {
        return false;
    }
    if deleted {
        return true;
    }
    // Index any non-excluded path; binary sniff happens at apply time.
    true
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
    }

    #[test]
    fn queue_gate_matches_agent_text_exclude_matcher() {
        use crate::workspace::filter::path_excluded;
        // Parity: anything AgentText excludes must not queue (plus product-internal).
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
                "{rel} must not enter text-index queue"
            );
        }
    }
}
