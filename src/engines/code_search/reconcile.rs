//! Disk ↔ index reconcile: only emits dirty signals into `pending_updates`.
//!
//! Digestion is consume-only (`refresh_index_incremental` /
//! [`super::flush_pending_updates`] → [`super::update_files`]).
//!
//! WP-141: the periodic scan is the source of truth (the watcher only makes
//! changes show up sooner). It only stats: a path whose mtime/len match its
//! stamp is skipped without being read; any mismatch is queued and
//! `update_files` compares content (identical ⇒ stamp refresh, no embed).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::build::scan_stat_candidates;
use super::{CodeSearchRuntime, FileStamp, stamps};

/// What the scan compares the disk against.
struct Known {
    /// Every path the index has accounted for (indexed or stamped).
    paths: HashSet<String>,
    stamps: HashMap<String, FileStamp>,
}

fn known_set(runtime: &CodeSearchRuntime) -> Option<Known> {
    let mem: HashMap<String, FileStamp> = runtime.file_stamps.lock().ok()?.clone();
    if runtime.index_is_loaded() {
        let indexed = runtime
            .with_index(|index| Ok(index.indexed_paths()))
            .map_err(|e| tracing::warn!(error = %e, "code_search reconcile: indexed_paths failed"))
            .ok()?;
        let mut paths = indexed;
        paths.extend(mem.keys().cloned());
        return Some(Known { paths, stamps: mem });
    }
    // IndexCold: never pull the index back into RAM for a scan. The persisted
    // stamp keys are the index's path set; without them the scan cannot tell
    // deletions, so it defers to a full reconcile on the next load.
    let Some(disk) = stamps::load(&runtime.workspace_root) else {
        runtime.request_full_reconcile_on_load();
        return None;
    };
    let mut paths: HashSet<String> = disk.keys().cloned().collect();
    paths.extend(mem.keys().cloned());
    let mut merged: HashMap<String, FileStamp> = disk
        .into_iter()
        .filter_map(|(p, s)| s.map(|s| (p, s)))
        .collect();
    merged.extend(mem);
    Some(Known {
        paths,
        stamps: merged,
    })
}

/// Pure diff (stat results in, dirty signals out). `disk` = scannable paths
/// with their current stamp (`None` = stat failed ⇒ queued, update decides).
pub(crate) fn diff_against_stamps(
    disk: &[(String, Option<FileStamp>)],
    known_paths: &HashSet<String>,
    stamps: &HashMap<String, FileStamp>,
) -> Vec<(String, bool)> {
    let disk_set: HashSet<&str> = disk.iter().map(|(p, _)| p.as_str()).collect();
    let mut dirty: Vec<(String, bool)> = known_paths
        .iter()
        .filter(|p| !disk_set.contains(p.as_str()))
        .map(|p| (p.clone(), true))
        .collect();
    for (path, stamp) in disk {
        match (stamp, stamps.get(path)) {
            (Some(now), Some(was)) if now == was => {}
            _ => dirty.push((path.clone(), false)),
        }
    }
    dirty.sort();
    dirty
}

/// Discover paths that diverge from the index and queue them as dirty signals.
///
/// - Known to the index, gone from disk (or no longer indexable) → `(path, true)`
/// - New, or mtime/len differ from the stamp → `(path, false)`
///
/// Runs whether or not the index is loaded (WP-141): a cooled index scans
/// against `stamps.v1` and its changes wait in the queue for the next load.
pub fn queue_reconcile_dirty(runtime: &CodeSearchRuntime) {
    let disk = match scan_stat_candidates(&runtime.workspace_root) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "code_search reconcile: scan failed");
            return;
        }
    };
    let Some(known) = known_set(runtime) else {
        return;
    };
    let dirty = diff_against_stamps(&disk, &known.paths, &known.stamps);
    if dirty.is_empty() {
        return;
    }

    let queued_del = dirty.iter().filter(|(_, d)| *d).count();
    let queued_mod = dirty.len() - queued_del;
    if let Ok(mut pending) = runtime.pending_updates.lock() {
        for item in dirty {
            pending.insert(item);
        }
        let pending_len = pending.len();
        tracing::info!(
            deleted = queued_del,
            modified = queued_mod,
            pending = pending_len,
            "code_search reconcile queued dirty paths"
        );
        runtime.persist_queue(&pending); // WP-131 exact signal
    }
}

/// Serve-process scan when no worker is alive (WP-141 NotWarm / WorkerDown).
/// Pure-stat against `stamps.v1`; writes `queue.json` only. Does not load the
/// index or embed. Missing/invalid stamps ⇒ no-op (full reconcile waits for
/// the next load). An empty dirty set clears a stale count-only queue.
pub fn queue_reconcile_from_stamps(workspace_root: &Path) {
    let Some(entries) = stamps::load(workspace_root) else {
        return;
    };
    let disk = match scan_stat_candidates(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "code_search serve reconcile: scan failed");
            return;
        }
    };
    let known_paths: HashSet<String> = entries.keys().cloned().collect();
    let stamp_map: HashMap<String, FileStamp> = entries
        .into_iter()
        .filter_map(|(p, s)| s.map(|s| (p, s)))
        .collect();
    let dirty = diff_against_stamps(&disk, &known_paths, &stamp_map);
    let pending: HashSet<(String, bool)> = dirty.into_iter().collect();
    if pending.is_empty() {
        // Tree matches stamps: drop a stale count-only queue left by a dead worker.
        super::write_code_queue(workspace_root, &HashSet::new());
        return;
    }
    let queued_del = pending.iter().filter(|(_, d)| *d).count();
    tracing::info!(
        deleted = queued_del,
        modified = pending.len() - queued_del,
        pending = pending.len(),
        "code_search serve reconcile queued dirty paths"
    );
    super::write_code_queue(workspace_root, &pending);
}

/// Reconcile dirty signals then digest via the shared flush/update path.
pub fn sync_index_with_disk(runtime: &CodeSearchRuntime) {
    queue_reconcile_dirty(runtime);
    super::flush_pending_updates(runtime);
}

pub fn read_stamp(abs: &std::path::Path) -> Option<FileStamp> {
    stamp_from_meta(&fs::metadata(abs).ok()?)
}

pub(crate) fn stamp_from_meta(meta: &fs::Metadata) -> Option<FileStamp> {
    let modified = meta.modified().ok()?;
    let mtime_ns = modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos() as u64)
        .or_else(|| {
            SystemTime::now()
                .duration_since(modified)
                .ok()
                .map(|d| u64::MAX.saturating_sub(d.as_nanos() as u64))
        })?;
    Some(FileStamp {
        mtime_ns,
        len: meta.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::index_workspace_blocking;
    use crate::engines::code_search::{CodeSearchRuntime, flush_pending_updates};
    use tempfile::TempDir;

    #[test]
    fn reconcile_queues_new_modified_and_deleted() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();

        let emb = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
        let runtime = CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        );
        // Seed stamps as if we had just indexed.
        sync_index_with_disk(&runtime);
        assert!(runtime.pending_updates.lock().unwrap().is_empty());

        std::fs::write(root.join("a.rs"), "fn a_changed() {}\n").unwrap();
        std::fs::write(root.join("c.rs"), "fn c() {}\n").unwrap();
        std::fs::remove_file(root.join("b.rs")).unwrap();

        queue_reconcile_dirty(&runtime);
        let pending: HashSet<_> = runtime.pending_updates.lock().unwrap().clone();
        assert!(pending.contains(&("a.rs".into(), false)));
        assert!(pending.contains(&("c.rs".into(), false)));
        assert!(pending.contains(&("b.rs".into(), true)));

        flush_pending_updates(&runtime);
        assert!(runtime.pending_updates.lock().unwrap().is_empty());

        let paths = runtime.with_index(|idx| Ok(idx.indexed_paths())).unwrap();
        assert!(paths.contains("a.rs"));
        assert!(paths.contains("c.rs"));
        assert!(!paths.contains("b.rs"));
        let a_ok = runtime
            .with_index(|idx| {
                let content = std::fs::read_to_string(root.join("a.rs")).unwrap();
                Ok(idx.file_content_matches("a.rs", &content))
            })
            .unwrap();
        assert!(a_ok);
    }

    #[test]
    fn stamp_fast_path_skips_unchanged() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let emb = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
        let runtime = CodeSearchRuntime::new(root.to_path_buf(), index, None, None);
        sync_index_with_disk(&runtime);

        queue_reconcile_dirty(&runtime);
        assert!(
            runtime.pending_updates.lock().unwrap().is_empty(),
            "unchanged files must not be re-queued when stamps match"
        );
    }

    #[test]
    fn allowlist_change_reconciles_without_rebuild() {
        use crate::workspace::filter::{
            WorkspaceExcludesFile, activate_workspace_excludes, with_excludes_cache_for_test,
        };

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("notes.md"), "# n\n").unwrap();
        std::fs::write(root.join("extra.foo"), "hello\n").unwrap();

        let mut wide = WorkspaceExcludesFile::builtin_defaults();
        wide.code_index_extensions.push("md".into());
        with_excludes_cache_for_test(wide, || {
            let index = index_workspace_blocking(root, Box::new(HashEmbedder)).unwrap();
            let runtime = CodeSearchRuntime::new(
                root.to_path_buf(),
                index,
                Some(Box::new(HashEmbedder)),
                None,
            );
            let paths = runtime.with_index(|idx| Ok(idx.indexed_paths())).unwrap();
            assert!(paths.contains("a.rs"), "{paths:?}");
            assert!(paths.contains("notes.md"), "{paths:?}");
            assert!(!paths.contains("extra.foo"), "{paths:?}");

            activate_workspace_excludes(WorkspaceExcludesFile::builtin_defaults());
            queue_reconcile_dirty(&runtime);
            let dropped = pending(&runtime);
            assert!(dropped.contains(&("notes.md".into(), true)), "{dropped:?}");
            assert!(
                !dropped.iter().any(|(path, _)| path == "a.rs"),
                "{dropped:?}"
            );
            assert!(
                !dropped.iter().any(|(path, _)| path == "extra.foo"),
                "{dropped:?}"
            );

            let mut with_foo = WorkspaceExcludesFile::builtin_defaults();
            with_foo.code_index_extensions.push("foo".into());
            activate_workspace_excludes(with_foo);
            runtime.pending_updates.lock().unwrap().clear();
            queue_reconcile_dirty(&runtime);
            let added = pending(&runtime);
            assert!(added.contains(&("extra.foo".into(), false)), "{added:?}");
            assert!(added.contains(&("notes.md".into(), true)), "{added:?}");
            assert!(!added.iter().any(|(path, _)| path == "a.rs"), "{added:?}");
        });
    }

    fn saved_runtime(
        root: &std::path::Path,
        emb: Box<dyn crate::engines::code_search::Embedder>,
    ) -> CodeSearchRuntime {
        let e = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(e)).unwrap();
        index.save(root).unwrap();
        CodeSearchRuntime::new(root.to_path_buf(), index, Some(emb), None)
    }

    fn reopen(
        root: &std::path::Path,
        emb: Box<dyn crate::engines::code_search::Embedder>,
    ) -> CodeSearchRuntime {
        let index = crate::engines::code_search::CodeSearchIndex::load(root).unwrap();
        CodeSearchRuntime::new(root.to_path_buf(), index, Some(emb), None)
    }

    fn pending(rt: &CodeSearchRuntime) -> HashSet<(String, bool)> {
        rt.pending_updates.lock().unwrap().clone()
    }

    /// Counts embed calls (proves "touch ⇒ no re-embed").
    struct CountingEmbedder(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl crate::engines::code_search::Embedder for CountingEmbedder {
        fn embedder_id(&self) -> &'static str {
            HashEmbedder.embedder_id()
        }
        fn inference_device(&self) -> &'static str {
            "hash"
        }
        fn embed_batch(&mut self, texts: &[String]) -> crate::types::Result<Vec<Vec<f32>>> {
            self.0
                .fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
            HashEmbedder.embed_batch(texts)
        }
    }

    // WP-141 (a): serve process, no runtime — stamps.v1 scan writes queue.json.
    #[test]
    fn serve_scan_from_stamps_queues_without_runtime() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        drop(saved_runtime(root, Box::new(HashEmbedder)));
        assert!(stamps::stamps_path(root).exists());

        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        std::fs::remove_file(root.join("a.rs")).unwrap();
        queue_reconcile_from_stamps(root);
        let q = crate::engines::code_search::read_code_queue(root);
        assert_eq!((q.queued, q.embed_dirty), (2, 1), "{q:?}");

        // No stamps ⇒ leave the existing queue alone (cannot re-derive paths).
        let before = crate::engines::code_search::read_code_queue(root);
        stamps::discard(root);
        queue_reconcile_from_stamps(root);
        assert_eq!(crate::engines::code_search::read_code_queue(root), before);
    }

    // WP-141 / GM 3: a cooled index keeps scanning; changes are queued
    // (exact queue.json) and the index stays unloaded until consumed.
    #[test]
    fn index_cold_scan_queues_without_reload() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("gone.rs"), "fn g() {}\n").unwrap();
        let runtime = saved_runtime(root, Box::new(HashEmbedder));
        runtime.drop_index_for_cool();
        assert!(!runtime.index_is_loaded());

        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        std::fs::remove_file(root.join("gone.rs")).unwrap();
        queue_reconcile_dirty(&runtime);
        assert!(
            !runtime.index_is_loaded(),
            "scan must not reload the RAM index"
        );
        let expected: HashSet<(String, bool)> =
            [("b.rs".into(), false), ("gone.rs".into(), true)].into();
        assert_eq!(pending(&runtime), expected);
        let q = crate::engines::code_search::read_code_queue(root);
        assert_eq!((q.queued, q.embed_dirty), (2, 1), "exact queue on disk");
        let p = crate::engines::code_search::code_corpus_progress(
            root,
            crate::engines::code_search::CodeEngineFacts {
                desired: true,
                state: Some(crate::engines::EngineState::Warm),
                model_ready: true,
            },
        );
        assert_eq!(p.files_pending, 2, "files_pending = queued paths: {p:?}");
        assert_eq!(p.chunks_pending, 0, "not cut yet: no chunk claim: {p:?}");
        assert!(p.has_pending(), "{p:?}");

        // Consume loads and digests.
        flush_pending_updates(&runtime);
        let paths = runtime.with_index(|i| Ok(i.indexed_paths())).unwrap();
        assert!(paths.contains("b.rs") && !paths.contains("gone.rs"));
        assert_eq!(crate::engines::code_search::read_code_queue(root).queued, 0);
    }

    // GM 3 + 中层裁定 3: cooled + no valid stamps ⇒ nothing guessed now, a
    // full reconcile runs when the index is loaded again.
    #[test]
    fn index_cold_without_stamps_reconciles_on_load() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let runtime = saved_runtime(root, Box::new(HashEmbedder));
        runtime.drop_index_for_cool();
        stamps::discard(root);
        runtime.file_stamps.lock().unwrap().clear();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();

        queue_reconcile_dirty(&runtime);
        assert!(!runtime.index_is_loaded());
        assert!(pending(&runtime).is_empty(), "no path set to diff against");

        runtime.ensure_index().unwrap();
        let p = pending(&runtime);
        assert!(p.contains(&("b.rs".into(), false)), "{p:?}");
        assert!(
            p.contains(&("a.rs".into(), false)),
            "unstamped ⇒ content check"
        );
        flush_pending_updates(&runtime);
        queue_reconcile_dirty(&runtime);
        assert!(
            pending(&runtime).is_empty(),
            "stamped again after the check"
        );
    }

    // Acceptance: changes made while stopped are caught after restart, and an
    // unchanged tree is not read at all (the scan only stats: nothing queued
    // ⇒ update_files never runs). Mode `0o000` is a Unix unreadable file.
    #[cfg(unix)]
    #[test]
    fn restart_uses_persisted_stamps_and_catches_offline_changes() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for f in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(root.join(f), format!("fn {}() {{}}\n", &f[..1])).unwrap();
        }
        drop(saved_runtime(root, Box::new(HashEmbedder)));
        assert!(stamps::stamps_path(root).is_file());

        // Stat-only proof: an unchanged file that cannot even be opened is
        // skipped (a scan that read content would fail on it or queue it).
        use std::os::unix::fs::PermissionsExt;
        let c = root.join("c.rs");
        let mode = std::fs::metadata(&c).unwrap().permissions();
        let stamp_before = read_stamp(&c).unwrap();
        std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(std::fs::read(&c).is_err(), "content really unreadable");
        assert_eq!(read_stamp(&c), Some(stamp_before), "chmod keeps mtime/len");
        let rt = reopen(root, Box::new(HashEmbedder));
        queue_reconcile_dirty(&rt);
        assert!(
            pending(&rt).is_empty(),
            "unchanged tree: stat only, nothing to read"
        );
        std::fs::set_permissions(&c, mode).unwrap();
        drop(rt);

        std::fs::write(root.join("a.rs"), "fn a_changed_offline() {}\n").unwrap();
        std::fs::remove_file(root.join("b.rs")).unwrap();
        std::fs::write(root.join("d.rs"), "fn d() {}\n").unwrap();
        let rt = reopen(root, Box::new(HashEmbedder));
        queue_reconcile_dirty(&rt);
        let expected: HashSet<(String, bool)> = [
            ("a.rs".into(), false),
            ("b.rs".into(), true),
            ("d.rs".into(), false),
        ]
        .into();
        assert_eq!(pending(&rt), expected);
        flush_pending_updates(&rt);
        drop(rt);
        let rt = reopen(root, Box::new(HashEmbedder));
        queue_reconcile_dirty(&rt);
        assert!(pending(&rt).is_empty(), "stamps persisted after the update");
    }

    // 中层裁定 2: mtime moves, content does not ⇒ stamp refresh, zero embeds.
    #[test]
    fn touch_without_content_change_does_not_reembed() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let rt = saved_runtime(root, Box::new(CountingEmbedder(calls.clone())));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(root.join("a.rs"))
            .unwrap()
            .set_modified(later)
            .unwrap();

        queue_reconcile_dirty(&rt);
        assert!(pending(&rt).contains(&("a.rs".into(), false)));
        flush_pending_updates(&rt);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no re-embed"
        );
        let stamp = read_stamp(&root.join("a.rs")).unwrap();
        assert_eq!(rt.file_stamps.lock().unwrap().get("a.rs"), Some(&stamp));
        queue_reconcile_dirty(&rt);
        assert!(pending(&rt).is_empty());
    }

    #[test]
    fn files_without_chunks_are_accounted_not_requeued() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let rt = saved_runtime(root, Box::new(HashEmbedder));
        std::fs::write(root.join("empty.rs"), "").unwrap();
        std::fs::write(root.join("blob.rs"), b"\0\x01binary").unwrap();
        queue_reconcile_dirty(&rt);
        flush_pending_updates(&rt);
        queue_reconcile_dirty(&rt);
        assert!(
            pending(&rt).is_empty(),
            "empty file must not churn every pass"
        );
    }

    #[test]
    fn diff_is_stat_only_and_generic() {
        let s = |m, l| {
            Some(FileStamp {
                mtime_ns: m,
                len: l,
            })
        };
        let known: HashSet<String> = ["x.md", "y.weird", "z"]
            .iter()
            .map(|p| p.to_string())
            .collect();
        let mut stamps = HashMap::new();
        stamps.insert(
            "x.md".to_string(),
            FileStamp {
                mtime_ns: 1,
                len: 1,
            },
        );
        stamps.insert(
            "y.weird".to_string(),
            FileStamp {
                mtime_ns: 1,
                len: 1,
            },
        );
        let disk = vec![
            ("x.md".to_string(), s(1, 1)),    // same
            ("y.weird".to_string(), s(1, 2)), // size changed
            ("new.txt".to_string(), s(1, 1)), // new
            ("z".to_string(), None),          // stat failed ⇒ let update decide
        ];
        assert_eq!(
            diff_against_stamps(&disk, &known, &stamps),
            vec![
                ("new.txt".to_string(), false),
                ("y.weird".to_string(), false),
                ("z".to_string(), false),
            ]
        );
        let disk2 = vec![("x.md".to_string(), s(2, 1))];
        assert_eq!(
            diff_against_stamps(&disk2, &known, &stamps),
            vec![
                ("x.md".to_string(), false),
                ("y.weird".to_string(), true),
                ("z".to_string(), true),
            ]
        );
    }

    // Crash between index commit and stamp write: meta.json moved on, stamps
    // did not, so stamps are discarded and every path is re-checked by
    // content; only the really changed file is embedded (more reads, never a
    // missed change).
    #[test]
    fn crash_between_index_commit_and_stamps_rereads_never_misses() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for f in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(root.join(f), format!("fn {}() {{}}\n", &f[..1])).unwrap();
        }
        drop(saved_runtime(root, Box::new(HashEmbedder)));
        std::fs::write(root.join("b.rs"), "fn b_after_crash() {}\n").unwrap();
        let mut meta = crate::engines::code_search::read_meta(root)
            .unwrap()
            .unwrap();
        meta.created_at = "2099-01-01T00:00:00Z".into();
        crate::engines::code_search::write_meta(root, &meta).unwrap();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let rt = reopen(root, Box::new(CountingEmbedder(calls.clone())));
        assert!(
            !stamps::stamps_path(root).exists(),
            "mismatched stamps discarded"
        );
        queue_reconcile_dirty(&rt);
        assert_eq!(pending(&rt).len(), 3, "no stamps: every path re-checked");
        flush_pending_updates(&rt);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "only b.rs (1 chunk) re-embedded"
        );
        assert!(
            stamps::stamps_path(root).is_file(),
            "stamps rewritten after commit"
        );
        queue_reconcile_dirty(&rt);
        assert!(pending(&rt).is_empty());
    }

    /// WP-141 measurement (manual, `--ignored`): the periodic scan's pure stat
    /// work on `LITECODE_WP141_BENCH_ROOT` (walk + filter + stat + diff, no
    /// index, no model, no reads). Run 0 builds the stamp map (untimed for
    /// the diff baseline but reported); runs 1..=N are timed.
    #[test]
    #[ignore]
    fn wp141_bench_stat_scan() {
        let Ok(root) = std::env::var("LITECODE_WP141_BENCH_ROOT") else {
            return;
        };
        let runs: usize = std::env::var("LITECODE_WP141_BENCH_RUNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let root = std::path::PathBuf::from(root);
        let t0 = std::time::Instant::now();
        let first = scan_stat_candidates(&root).unwrap();
        let first_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let stamps: HashMap<String, FileStamp> = first
            .iter()
            .filter_map(|(p, s)| s.map(|s| (p.clone(), s)))
            .collect();
        let known: HashSet<String> = stamps.keys().cloned().collect();
        println!("WP141 files={} first_scan_ms={first_ms:.1}", first.len());
        for i in 1..=runs {
            let t = std::time::Instant::now();
            let disk = scan_stat_candidates(&root).unwrap();
            let dirty = diff_against_stamps(&disk, &known, &stamps);
            println!(
                "WP141 run={i} files={} dirty={} ms={:.1}",
                disk.len(),
                dirty.len(),
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        let hwm = std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find(|l| l.starts_with("VmHWM"))
            .unwrap_or("")
            .to_string();
        println!("WP141 {hwm}");
    }
}
