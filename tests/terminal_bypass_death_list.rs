//! Death list: no second PTY stack outside `src/terminal/`.
//!
//! - desktop/web must not pull `node-pty` (Electron must not own ConPTY)
//! - `src/` must not mention ConPTY / winpty / portable-pty outside `terminal/` + Cargo.toml

use std::path::{Path, PathBuf};

const FORBIDDEN_PKG: &[&str] = &[
    "node-pty",
    "@lydell/node-pty",
    "node-pty-prebuilt",
    "node-pty-prebuilt-multiarch",
];

const FORBIDDEN_SRC_SYMBOLS: &[&str] = &[
    "portable-pty",
    "portable_pty",
    "ConPTY",
    "conpty",
    "winpty",
    "CreatePseudoConsole",
];

fn should_scan_src(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("rs"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, filter: &dyn Fn(&Path) -> bool) {
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
            walk(&path, out, filter);
        } else if filter(&path) {
            out.push(path);
        }
    }
}

fn is_under_terminal(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    s.contains("/src/terminal/") || s.ends_with("/src/terminal/mod.rs")
}

#[test]
fn death_list_no_node_pty_in_desktop_or_web_package_json() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    for rel in [
        "desktop/package.json",
        "web/package.json",
        "desktop/package-lock.json",
        "web/package-lock.json",
    ] {
        let path = root.join(rel);
        if !path.exists() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for pkg in FORBIDDEN_PKG {
            if text.contains(pkg) {
                hits.push(format!("{rel}: forbidden package `{pkg}`"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "PTY must live in Rust TerminalHub, not Node:\n{}",
        hits.join("\n")
    );
}

#[test]
fn death_list_src_pty_symbols_only_in_terminal_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files, &should_scan_src);

    let mut hits = Vec::new();
    for path in files {
        if is_under_terminal(&path) {
            continue;
        }
        // Historical comment in tools/bash.rs is rewritten in Phase 3; still scan.
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for sym in FORBIDDEN_SRC_SYMBOLS {
            if text.contains(sym) {
                hits.push(format!("{}: forbidden `{sym}`", path.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "PTY / ConPTY symbols must stay in src/terminal/:\n{}",
        hits.join("\n")
    );
}

#[test]
fn death_list_no_node_pty_require_in_desktop_web_src() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let filter = |p: &Path| {
        matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("ts" | "tsx" | "js" | "mjs" | "cjs")
        )
    };
    walk(&root.join("desktop/src"), &mut files, &filter);
    walk(&root.join("web/src"), &mut files, &filter);

    let mut hits = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for pkg in FORBIDDEN_PKG {
            if text.contains(pkg) {
                hits.push(format!("{}: `{pkg}`", path.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "desktop/web must not import node-pty:\n{}",
        hits.join("\n")
    );
}
