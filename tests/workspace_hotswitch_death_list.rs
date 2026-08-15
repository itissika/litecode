//! Death checklist: forbidden hot-switch symbols must not reappear in product source.
//!
//! Excludes docs/phase1-removed-tests (historical archive).

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "post_open",
    "commit_workspace_lock",
    "WorkspaceOpenEvent",
    "workspace/root_changed",
    "on_workspace_open",
    "workspace_root_changed",
    "rebind_db_path",
    "reload_root",
    "workspace_open_mutex",
    "api/workspace/open",
    "resetForWorkspaceRoot",
    "onRootChanged",
    "WorkspaceRootChanged",
    "reload_workspace",
];

const SCAN_ROOTS: &[&str] = &["src", "web/src", "desktop/src", "tests", "scripts"];

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
        // Desktop/preload expose openWorkspace as IPC relaunch — allowed name, not HTTP.
        // Death list targets the deleted POST client and protocol frames.
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for needle in FORBIDDEN {
            if text.contains(needle) {
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
