//! Cross-module LAP sameness + wire / filter / snapshot acceptance.
//!
//! All checks use a real tempdir (and real `canonicalize` on Windows).

use std::sync::Mutex;

use litecode::config::load_workspace_state;
use litecode::config::path::{canon_abs, os_probe_abs, strip_verbatim};
use litecode::lsp::{LspHub, file_to_uri};
use litecode::session::snapshot_paths::workspace_snapshot_id;
use litecode::workspace::{RelPathCtx, Sandbox, rel_path_under};

static LOCK: Mutex<()> = Mutex::new(());

#[test]
fn boot_sandbox_lsp_os_probe_roots_are_identical_lap() {
    let _guard = LOCK.lock().unwrap();
    let prev = std::env::current_dir().expect("prev cwd");
    let dir = tempfile::tempdir().unwrap();
    let state = load_workspace_state(Some(dir.path())).expect("load workspace");
    let boot = state.workspace_root.clone();

    let s = boot.to_string_lossy();
    assert!(
        !s.starts_with(r"\\?\"),
        "boot root must be LAP (no verbatim): {s}"
    );

    let sandbox = Sandbox::new(boot.clone()).expect("sandbox");
    assert_eq!(sandbox.root(), &boot);

    let raw_canon = dir.path().canonicalize().expect("canonicalize tempdir");
    let from_raw = strip_verbatim(&raw_canon);
    assert_eq!(boot, from_raw);
    assert_eq!(boot, canon_abs(dir.path()).expect("canon_abs"));
    assert_eq!(
        boot,
        os_probe_abs(dir.path()).expect("os_probe_abs"),
        "probe export must match product LAP"
    );

    let hub = LspHub::new();
    hub.set_workspace(boot.clone());
    assert_eq!(hub.workspace_root().as_ref(), Some(&boot));

    // Wire shape used by server/hello `project` (SessionController copies workspace_root).
    let project = boot.to_string_lossy().to_string();
    assert!(
        !project.contains(r"\\?\"),
        "hello.project must be LAP: {project}"
    );
    assert_eq!(project, canon_abs(dir.path()).unwrap().to_string_lossy());

    let file = boot.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let resolved = sandbox.resolve("a.txt").expect("resolve");
    assert!(resolved.starts_with(&boot));

    let uri = file_to_uri(&resolved);
    assert!(!uri.contains("?/"), "uri must not retain verbatim: {uri}");

    let _ = std::env::set_current_dir(prev);
}

#[cfg(windows)]
#[test]
fn windows_raw_canonicalize_is_verbatim_then_strips_to_lap() {
    let _guard = LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().canonicalize().expect("canonicalize");
    let raw_s = raw.to_string_lossy();
    assert!(
        raw_s.starts_with(r"\\?\"),
        "Windows canonicalize should emit verbatim prefix on this host (got {raw_s}); \
         CI windows-latest must satisfy this so strip→LAP is proven against real OS output"
    );
    let lap = strip_verbatim(&raw);
    assert!(!lap.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(lap, canon_abs(dir.path()).unwrap());
    assert_eq!(lap, os_probe_abs(dir.path()).unwrap());
}

#[test]
fn strip_verbatim_unc_keeps_unc_prefix() {
    let stripped = strip_verbatim(std::path::Path::new(r"\\?\UNC\host\share\proj"));
    assert_eq!(
        stripped,
        std::path::PathBuf::from(r"\\host\share\proj"),
        "UNC strip must produce \\\\host\\share form, not UNC\\host"
    );
}

#[test]
fn rel_path_under_uses_lap_compare() {
    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    let file = root.join("src").join("a.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "fn x() {}\n").unwrap();

    assert_eq!(rel_path_under(&root, &file).as_deref(), Some("src/a.rs"));

    // Pass a path that may still be pre-LAP (tempdir join); helper must LAP both sides.
    assert_eq!(
        rel_path_under(dir.path(), &file).as_deref(),
        Some("src/a.rs")
    );

    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("x.rs");
    std::fs::write(&other, "x").unwrap();
    assert_eq!(rel_path_under(&root, &other), None);
}

#[test]
fn rel_path_ctx_matches_rel_path_under_and_rejects_outside() {
    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    let file = root.join("src").join("a.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "fn x() {}\n").unwrap();

    let ctx = RelPathCtx::new(&root).unwrap();
    assert_eq!(ctx.rel(&file).as_deref(), Some("src/a.rs"));
    assert_eq!(ctx.rel(&file), rel_path_under(&root, &file));

    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("x.rs");
    std::fs::write(&other, "x").unwrap();
    assert_eq!(ctx.rel(&other), None);
}

#[test]
fn snapshot_id_stable_across_verbatim_and_lap_input() {
    let dir = tempfile::tempdir().unwrap();
    let lap = canon_abs(dir.path()).unwrap();
    let raw = dir.path().canonicalize().unwrap();
    let id_lap = workspace_snapshot_id(&lap);
    let id_raw = workspace_snapshot_id(&raw);
    let id_stripped = workspace_snapshot_id(&strip_verbatim(&raw));
    assert_eq!(id_lap, id_stripped);
    assert_eq!(
        id_lap, id_raw,
        "workspace_snapshot_id must LAP-normalize before hash"
    );
}
