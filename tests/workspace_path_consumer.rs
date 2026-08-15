//! LAP consumer gaps: walk relativize, active_paths fallback, tool roots.

use std::sync::Mutex;

use litecode::config::WorkspacePaths;
use litecode::config::path::{canon_abs, strip_verbatim};
use litecode::config::workspace::{
    active_paths, clear_runtime_paths, set_runtime_paths, workspace_root_lap,
};
use litecode::workspace::rel_path_under;

static LOCK: Mutex<()> = Mutex::new(());

#[test]
fn rel_path_under_accepts_raw_canonicalize_form() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("src").join("a.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "fn x() {}\n").unwrap();

    let root = canon_abs(dir.path()).unwrap();
    let raw_file = file.canonicalize().unwrap();
    assert_eq!(
        rel_path_under(&root, &raw_file).as_deref(),
        Some("src/a.rs"),
        "raw canonicalize path must relativize under LAP root"
    );
}

#[cfg(windows)]
#[test]
fn windows_verbatim_walk_path_relativizes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("b.txt");
    std::fs::write(&file, "x\n").unwrap();
    let root = canon_abs(dir.path()).unwrap();
    let raw = file.canonicalize().unwrap();
    assert!(
        raw.to_string_lossy().starts_with(r"\\?\"),
        "expected verbatim canonicalize, got {}",
        raw.display()
    );
    assert_eq!(rel_path_under(&root, &raw).as_deref(), Some("b.txt"));
    assert_eq!(
        rel_path_under(&root, &strip_verbatim(&raw)).as_deref(),
        Some("b.txt")
    );
}

#[test]
fn active_paths_fallback_is_lap() {
    let _guard = LOCK.lock().unwrap();
    clear_runtime_paths();
    let prev = std::env::current_dir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let paths = active_paths();
    let root = workspace_root_lap();
    let expected = canon_abs(dir.path()).unwrap();
    assert_eq!(root, expected);
    assert!(
        !root.to_string_lossy().starts_with(r"\\?\"),
        "workspace_root_lap must be LAP"
    );
    assert_eq!(
        paths.sessions_db.parent().and_then(|p| p.parent()),
        Some(expected.as_path())
    );

    // Restore so other tests see a clean thread-local.
    set_runtime_paths(WorkspacePaths::for_legacy_root(&expected));
    let _ = std::env::set_current_dir(prev);
}

#[test]
fn scannable_files_lists_under_lap_root() {
    use litecode::engines::code_search::scannable_files;

    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    std::fs::write(root.join("lib.rs"), "fn x() {}\n").unwrap();
    let files = scannable_files(&root).expect("scannable");
    assert!(files.iter().any(|f| f == "lib.rs"), "got {files:?}");
}

#[test]
fn project_root_fallback_accepts_verbatim_under_lap_ws() {
    use litecode::lsp::project_root::project_root_for_file;

    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    // No Cargo.toml — force workspace fallback for rust-analyzer.
    let file = root.join("orphan.rs");
    std::fs::write(&file, "fn x() {}\n").unwrap();
    let raw = file.canonicalize().unwrap();
    let resolved = project_root_for_file(&raw, "rust-analyzer", Some(&root)).expect("fallback");
    assert_eq!(resolved, root);
}
