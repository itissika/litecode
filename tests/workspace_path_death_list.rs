//! Death checklist: bare path canonicalize / legacy strip must not reappear.
//!
//! Allowed canonicalize site: `src/config/path.rs` only (`canon_abs` / `os_probe_abs`).
//! Excludes docs/phase1-removed-tests (historical archive).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FORBIDDEN_SYMBOLS: &[&str] = &[
    "normalize_lsp_abs",
    "strip_windows_verbatim",
    "normalizeWindowsPath",
];

/// Silent-strip patterns that must not return in the web LAP client.
const FORBIDDEN_WEB_STRIP_RES: &[&str] = &[
    r"\/\/\/?\?\/", // old normalizeWindowsPath replace
    r"\/\/\?\/unc",
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

fn is_config_path_rs(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    s.ends_with("/src/config/path.rs") || s.ends_with("src/config/path.rs")
}

fn is_under_src(path: &Path, root: &Path) -> bool {
    path.starts_with(root.join("src"))
}

fn canonicalize_call_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\.canonicalize\s*\(").expect("regex"))
}

#[test]
fn death_list_no_bare_canonicalize_outside_path_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);

    let re = canonicalize_call_re();
    let mut hits: Vec<String> = Vec::new();
    for path in files {
        if is_config_path_rs(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if re.is_match(&text) {
            hits.push(format!(
                "{}: bare `.canonicalize(` — use config::path::{{canon_abs,os_probe_abs}}",
                path.display()
            ));
        }
    }

    assert!(
        hits.is_empty(),
        "LAP zero-bypass death list (canonicalize):\n{}",
        hits.join("\n")
    );
}

#[test]
fn death_list_forbidden_legacy_path_symbols() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for rel in SCAN_ROOTS {
        walk(&root.join(rel), &mut files);
    }

    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        if path.ends_with("workspace_path_death_list.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for needle in FORBIDDEN_SYMBOLS {
            if text.contains(needle) {
                hits.push(format!("{}: contains `{needle}`", path.display()));
            }
        }
    }

    assert!(
        hits.is_empty(),
        "LAP zero-bypass death list (legacy symbols):\n{}",
        hits.join("\n")
    );
}

#[test]
fn death_list_web_no_silent_verbatim_strip() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/src/lib/litecodeLsp.ts");
    let text = std::fs::read_to_string(&path).expect("litecodeLsp.ts");
    let mut hits = Vec::new();
    for pat in FORBIDDEN_WEB_STRIP_RES {
        let re = regex::Regex::new(pat).expect("web strip regex");
        if re.is_match(&text) {
            hits.push(format!("{}: matches silent-strip /{pat}/", path.display()));
        }
    }
    assert!(
        hits.is_empty(),
        "web must reject non-LAP roots, not silently strip:\n{}",
        hits.join("\n")
    );
}

#[test]
fn death_list_src_canonicalize_whitelist_is_only_path_rs() {
    // Sanity: the whitelist file itself must still call canonicalize (otherwise
    // the zero-bypass rule is vacuously wrong).
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config/path.rs");
    let text = std::fs::read_to_string(&path).expect("path.rs");
    assert!(
        canonicalize_call_re().is_match(&text),
        "config/path.rs must be the sole canonicalize site"
    );
    assert!(
        is_under_src(&path, Path::new(env!("CARGO_MANIFEST_DIR"))),
        "path.rs must live under src/"
    );
}
