//! WP-040 regression (ISSUE-037): create_file / write_file_bytes(overwrite=false)
//! must not let concurrent creators both succeed (no silent clobber).
use litecode::workspace::WorkspaceService;
use std::sync::{Arc, Barrier};

fn race<F>(label: &str, f: F)
where
    F: Fn(&WorkspaceService, &str, usize) -> bool + Send + Sync + 'static,
{
    let dir = tempfile::tempdir().unwrap();
    let svc = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
    let f = Arc::new(f);
    let mut double = 0;
    let rounds = 300;
    let threads = 8;
    for r in 0..rounds {
        let path = format!("race_{r}.txt");
        let barrier = Arc::new(Barrier::new(threads));
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                let (svc, f, b, p) = (svc.clone(), f.clone(), barrier.clone(), path.clone());
                std::thread::spawn(move || {
                    b.wait();
                    f(&svc, &p, t)
                })
            })
            .collect();
        let ok = hs
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|x| *x)
            .count();
        if ok > 1 {
            double += 1;
        }
    }
    println!("{label}: rounds={rounds} threads={threads} rounds_with_multiple_successes={double}");
    assert_eq!(
        double, 0,
        "{label}: concurrent creates both reported success (later write clobbers earlier)"
    );
}

#[test]
fn probe_s2_create_file_race() {
    race("create_file", |s, p, t| {
        s.create_file(p, &format!("writer {t}")).is_ok()
    });
}

#[test]
fn probe_s2_write_file_bytes_no_overwrite_race() {
    race("write_file_bytes(overwrite=false)", |s, p, t| {
        s.write_file_bytes(p, format!("writer {t}").as_bytes(), false)
            .is_ok()
    });
}
