//! WP-028 probe S1 (ISSUE-038): rename_path across devices with overwrite=true onto an existing
//! file should succeed (fall back to copy); currently fails with CrossesDevices.
//! Needs a workspace root spanning two filesystems; uses "/" with /dev/shm (tmpfs) vs repo .tmp (ext4).
use litecode::workspace::WorkspaceService;

#[test]
#[ignore = "WP-028 probe: ISSUE-038"]
fn probe_s1_cross_device_overwrite_succeeds() {
    let shm = std::path::Path::new("/dev/shm");
    let repo_tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/wp028_s1");
    std::fs::create_dir_all(&repo_tmp).unwrap();
    let src = shm.join(format!("wp028_s1_src_{}.txt", std::process::id()));
    let dst = repo_tmp.join("dst.txt");
    std::fs::write(&src, "NEW").unwrap();
    std::fs::write(&dst, "ORIGINAL").unwrap();
    let svc = match WorkspaceService::new("/".into()) {
        Ok(s) => s,
        Err(e) => {
            println!("S1 probe: cannot open workspace at / ({e}); scenario unreachable");
            return;
        }
    };
    let from = src.strip_prefix("/").unwrap().to_string_lossy().to_string();
    let to = dst.strip_prefix("/").unwrap().to_string_lossy().to_string();
    let res = svc.rename_path(&from, &to, true);
    let dst_now = std::fs::read_to_string(&dst).ok();
    let src_now = std::fs::read_to_string(&src).ok();
    println!("S1 probe: rename_path result={res:?} dst={dst_now:?} src={src_now:?}");
    let _ = std::fs::remove_file(&src);
    assert!(res.is_ok(), "ISSUE-038: cross-device overwrite move failed: {res:?}");
    assert_eq!(dst_now.as_deref(), Some("NEW"), "destination should hold moved content");
    assert!(src_now.is_none(), "source should be gone after move");
}
