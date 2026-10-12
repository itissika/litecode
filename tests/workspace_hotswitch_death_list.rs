//! Death checklist: forbidden hot-switch symbols must not reappear in product source.
//!
//! Ban list = symbols removed when workspace-root hot-switch was deleted (exact /
//! identifier-boundary match). Generic substrings that false-positive on legitimate
//! APIs (`reload_workspace`, `post_open`, `api/workspace/open`) were dropped in
//! WP-051 / ISSUE-007: excludes reload, config ApplyPlan.reload_workspace, and
//! open_with_default_app are not root hot-switch. HTTP open-for-root-switch is
//! guarded by `workspace_open_http_endpoint_is_gone` in workspace_process_contract.
//!
//! Excludes docs/phase1-removed-tests (historical archive).

use std::path::{Path, PathBuf};

/// Deleted hot-switch symbols (handlers, protocol frames, client resets).
/// Path-like entries contain `/` and match as substrings; others use identifier boundaries.
const FORBIDDEN: &[&str] = &[
    "commit_workspace_lock",
    "WorkspaceOpenEvent",
    "workspace/root_changed",
    "on_workspace_open",
    "workspace_root_changed",
    "rebind_db_path",
    "reload_root",
    "workspace_open_mutex",
    "resetForWorkspaceRoot",
    "onRootChanged",
    "WorkspaceRootChanged",
];

const SCAN_ROOTS: &[&str] = &["src", "web/src", "desktop/src", "tests", "scripts"];

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// True if `needle` appears as a whole identifier (or path substring when it contains `/`).
fn contains_forbidden(text: &str, needle: &str) -> bool {
    if needle.contains('/') {
        return text.contains(needle);
    }
    let bytes = text.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || bytes.len() < n.len() {
        return false;
    }
    let mut i = 0;
    while i + n.len() <= bytes.len() {
        if &bytes[i..i + n.len()] == n {
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let after = i + n.len();
            let after_ok = after == bytes.len() || !is_ident_char(bytes[after]);
            if before_ok && after_ok {
                return true;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    false
}

fn should_scan(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    if s.contains("/scripts/_tmp_") {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("rs" | "ts" | "tsx" | "js" | "ps1" | "sh")
    )
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "node_modules" | "target" | "dist" | ".git") {
                continue;
            }
            walk(&path, out);
        } else if should_scan(&path) {
            out.push(path);
        }
    }
}

#[test]
fn death_list_forbidden_hot_switch_symbols() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for rel in SCAN_ROOTS {
        walk(&root.join(rel), &mut files);
    }

    let mut hits: Vec<String> = Vec::new();
    for path in files {
        if path.ends_with("workspace_hotswitch_death_list.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for needle in FORBIDDEN {
            if contains_forbidden(&text, needle) {
                hits.push(format!("{}: contains `{needle}`", path.display()));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "hot-switch death list violations:\n{}",
        hits.join("\n")
    );
}

#[test]
fn forbidden_match_uses_identifier_boundaries() {
    // Longer identifiers must not trip a shorter deleted symbol (substring trap).
    assert!(!contains_forbidden(
        "fn reload_workspace_excludes_from_disk()",
        "reload_workspace"
    ));
    assert!(!contains_forbidden(
        "fn reload_root_helper()",
        "reload_root"
    ));
    // Exact deleted symbol / protocol path still hit.
    assert!(contains_forbidden(
        "fn commit_workspace_lock() {}",
        "commit_workspace_lock"
    ));
    assert!(contains_forbidden(
        r#""workspace/root_changed""#,
        "workspace/root_changed"
    ));
    assert!(contains_forbidden(
        "emit(WorkspaceRootChanged)",
        "WorkspaceRootChanged"
    ));
}
