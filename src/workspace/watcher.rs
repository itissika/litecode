use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::change::WorkspaceChange;
use super::filter::{
    FilterPreset, is_workspace_excludes_rel, path_excluded, path_has_product_internal_dir,
    rel_path_under, reload_workspace_excludes_from_disk,
};
use super::service::WorkspaceService;

const DEBOUNCE_MS: u64 = 300;

/// Keeps the OS watcher alive for the lifetime of the server.
///
/// Sole `notify` owner for the workspace (DESIGN §2.9). Watcher exclude is a
/// hard cut at the source; the shared bus never sees those paths (except
/// editable `.litecode` json). Consumers filter again: engines with Search,
/// the UI with Explorer.
pub struct WorkspaceWatcher {
    // The watcher is held purely for ownership (kept alive for the server's
    // lifetime); it is never locked after creation, so a Mutex is not needed.
    _watcher: RecommendedWatcher,
}

impl WorkspaceWatcher {
    pub fn start(workspace: Arc<WorkspaceService>) -> anyhow::Result<Arc<Self>> {
        let root = workspace.sandbox().root().to_path_buf();
        let change_tx = workspace.change_sender();

        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = notify_tx.send(event);
                }
            },
            notify::Config::default(),
        )?;
        watcher.watch(&root, RecursiveMode::Recursive)?;

        let handle = Arc::new(Self { _watcher: watcher });

        let rel_base = root;
        tokio::spawn(async move {
            let mut pending: HashSet<String> = HashSet::new();
            let mut pending_deleted: HashSet<String> = HashSet::new();
            let mut debounce = tokio::time::interval(Duration::from_millis(DEBOUNCE_MS));
            debounce.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    maybe_event = notify_rx.recv() => {
                        let Some(event) = maybe_event else { break };
                        if let Some((paths, deleted)) = classify_event(&event, &rel_base) {
                            if deleted {
                                pending_deleted.extend(paths);
                            } else {
                                pending.extend(paths);
                            }
                        }
                    }
                    _ = debounce.tick() => {
                        // A `Remove` event does not always mean the file is gone: an
                        // atomic save (write a temp file, then rename it over the
                        // original — which `WorkspaceService::atomic_write` does) is
                        // reported as `Remove` for the original path on Windows. If the
                        // path still exists on disk at flush time it was an overwrite,
                        // not a deletion, so reclassify it as `modified`. This mirrors
                        // VS Code's file-watcher behaviour (nodejsWatcherLib.ts): on a
                        // rename/delete it waits, stats the path, and emits a `change`
                        // when the file still exists, a `delete` only when it is truly
                        // gone. It prevents the frontend from closing the editor tab on
                        // every Ctrl+S save.
                        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |rel| {
                            rel_base.join(rel).exists()
                        });
                        if changes_include_workspace_excludes(&changes) {
                            reload_workspace_excludes_from_disk(&rel_base);
                        }
                        for change in changes {
                            let _ = change_tx.send(change);
                        }
                    }
                }
            }
        });

        Ok(handle)
    }
}

fn classify_event(event: &Event, root: &Path) -> Option<(Vec<String>, bool)> {
    let deleted = matches!(event.kind, EventKind::Remove(_));
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_)
    ) {
        return None;
    }

    let paths: Vec<String> = event
        .paths
        .iter()
        .filter_map(|p| rel_path_under(root, p))
        .filter(|p| event_rel_is_broadcast(p))
        .collect();

    if paths.is_empty() {
        return None;
    }

    Some((paths, deleted))
}

/// Keep editable `.litecode/*.json`; drop product-internal trees (index writes);
/// otherwise `watcher_exclude` is a hard cut — no Search-line rescue.
fn event_rel_is_broadcast(rel: &str) -> bool {
    if is_editable_litecode_json(rel) {
        return true;
    }
    if path_has_product_internal_dir(rel) {
        return false;
    }
    !path_excluded(rel, FilterPreset::Watcher)
}

/// Paths the explorer / editor should hear. Same bus as engines; Explorer
/// (`files.exclude`) is the UI face. Source already applied Watcher.
pub fn filter_change_for_ui(mut change: WorkspaceChange) -> Option<WorkspaceChange> {
    change.paths.retain(|rel| ui_rel_is_noteworthy(rel));
    if change.paths.is_empty() {
        None
    } else {
        Some(change)
    }
}

fn ui_rel_is_noteworthy(rel: &str) -> bool {
    if is_editable_litecode_json(rel) {
        return true;
    }
    if path_has_product_internal_dir(rel) {
        return false;
    }
    !path_excluded(rel, FilterPreset::Explorer)
}

/// Coalesce the pending modify/delete sets into the changes to broadcast.
///
/// Mirrors VS Code's file-watcher handling of atomic saves (see
/// `nodejsWatcherLib.ts`): a `Remove` event is not trusted as a deletion on its
/// own, because an atomic save (temp file + rename over the original) surfaces
/// as `Remove` for the original path on Windows. Each pending deletion is
/// therefore re-stated via `path_exists`; if it still exists on disk it is
/// reclassified as `modified` (the file was overwritten, not deleted).
///
/// `path_exists` is injected so the logic is unit-testable without touching the
/// real filesystem; in production it is `|rel| root.join(rel).exists()`.
fn coalesce_pending(
    pending: &mut HashSet<String>,
    pending_deleted: &mut HashSet<String>,
    path_exists: impl Fn(&str) -> bool,
) -> Vec<WorkspaceChange> {
    let mut changes = Vec::new();
    let mut really_deleted: Vec<String> = Vec::new();
    let mut resurrected: Vec<String> = Vec::new();
    for rel in pending_deleted.drain() {
        if path_exists(&rel) {
            resurrected.push(rel);
        } else {
            really_deleted.push(rel);
        }
    }
    if !really_deleted.is_empty() {
        changes.push(WorkspaceChange {
            paths: really_deleted,
            kind: "deleted".into(),
        });
    }
    let mut to_merge = std::mem::take(pending);
    to_merge.extend(resurrected);
    if !to_merge.is_empty() {
        changes.push(WorkspaceChange {
            paths: to_merge.into_iter().collect(),
            kind: "modified".into(),
        });
    }
    changes
}

fn is_editable_litecode_json(rel: &str) -> bool {
    is_workspace_excludes_rel(rel) || crate::config::workspace::is_workspace_tool_defs_rel(rel)
}

fn changes_include_workspace_excludes(changes: &[WorkspaceChange]) -> bool {
    changes
        .iter()
        .any(|c| c.paths.iter().any(|p| is_workspace_excludes_rel(p)))
}

pub fn spawn_watcher(workspace: Arc<WorkspaceService>) -> anyhow::Result<Arc<WorkspaceWatcher>> {
    WorkspaceWatcher::start(workspace)
}

/// Drop the previous watcher (if any) and start watching the current workspace root.
pub async fn restart_watcher(
    watcher_slot: &tokio::sync::Mutex<Option<Arc<WorkspaceWatcher>>>,
    workspace: Arc<WorkspaceService>,
) -> anyhow::Result<()> {
    let new = spawn_watcher(workspace)?;
    let mut guard = watcher_slot.lock().await;
    *guard = Some(new);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, EventAttributes, ModifyKind, RemoveKind};
    use notify::{Event, EventKind};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_event(kind: EventKind, paths: &[&str]) -> Event {
        Event {
            paths: paths.iter().map(std::path::PathBuf::from).collect(),
            kind,
            attrs: EventAttributes::default(),
        }
    }

    /// A fresh, unique, real temp dir (rel_path_under canonicalizes the root,
    /// so it must exist on disk).
    fn temp_root() -> std::path::PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let base =
            std::env::temp_dir().join(format!("litecode_watch_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn changed(paths: &[&str]) -> WorkspaceChange {
        WorkspaceChange {
            paths: paths.iter().map(|s| s.to_string()).collect(),
            kind: "modified".into(),
        }
    }

    fn deleted(paths: &[&str]) -> WorkspaceChange {
        WorkspaceChange {
            paths: paths.iter().map(|s| s.to_string()).collect(),
            kind: "deleted".into(),
        }
    }

    // --- classify_event -----------------------------------------------------

    #[test]
    fn remove_event_is_classified_as_deleted() {
        let root = temp_root();
        let f = root.join("a.txt");
        fs::write(&f, b"x").unwrap();
        let ev = make_event(EventKind::Remove(RemoveKind::Any), &[f.to_str().unwrap()]);
        let (paths, is_deleted) = classify_event(&ev, &root).unwrap();
        assert!(is_deleted);
        assert_eq!(paths, vec!["a.txt".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_and_modify_events_are_not_deleted() {
        let root = temp_root();
        let f = root.join("b.txt");
        fs::write(&f, b"x").unwrap();
        let p = f.to_str().unwrap();
        let (_, d_create) =
            classify_event(&make_event(EventKind::Create(CreateKind::Any), &[p]), &root).unwrap();
        let (_, d_modify) =
            classify_event(&make_event(EventKind::Modify(ModifyKind::Any), &[p]), &root).unwrap();
        assert!(!d_create);
        assert!(!d_modify);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unsupported_event_kinds_are_ignored() {
        let root = temp_root();
        let f = root.join("c.txt");
        fs::write(&f, b"x").unwrap();
        let ev = make_event(EventKind::Access(AccessKind::Any), &[f.to_str().unwrap()]);
        assert!(classify_event(&ev, &root).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn excludes_json_is_kept_even_when_watcher_exclude_matches() {
        let _lock = crate::workspace::filter::lock_excludes_cache_for_test();
        let prev = crate::workspace::filter::active_workspace_excludes();
        let root = temp_root();
        let mut lists = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        lists.watcher_exclude.push("**/.litecode/**".into());
        crate::workspace::filter::activate_workspace_excludes(lists);
        assert!(path_excluded(
            ".litecode/excludes.json",
            FilterPreset::Watcher
        ));
        let path = root.join(".litecode").join("excludes.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{}").unwrap();
        let ev = make_event(
            EventKind::Modify(ModifyKind::Any),
            &[path.to_str().unwrap()],
        );
        let (paths, is_deleted) = classify_event(&ev, &root).unwrap();
        assert!(!is_deleted);
        assert_eq!(paths, vec![".litecode/excludes.json".to_string()]);
        crate::workspace::filter::activate_workspace_excludes(prev);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn product_internal_index_is_not_broadcast() {
        let _lock = crate::workspace::filter::lock_excludes_cache_for_test();
        let prev = crate::workspace::filter::active_workspace_excludes();
        let root = temp_root();
        let mut lists = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        lists.watcher_exclude = vec!["*.litecode-tmp*".into()];
        crate::workspace::filter::activate_workspace_excludes(lists);
        let path = root.join(".litecode").join("index").join("chunks.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"x").unwrap();
        let ev = make_event(
            EventKind::Modify(ModifyKind::Any),
            &[path.to_str().unwrap()],
        );
        assert!(classify_event(&ev, &root).is_none());
        crate::workspace::filter::activate_workspace_excludes(prev);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn flush_detects_excludes_json_among_changes() {
        let modified = WorkspaceChange {
            paths: vec!["src/a.rs".into(), ".litecode/excludes.json".into()],
            kind: "modified".into(),
        };
        assert!(changes_include_workspace_excludes(&[modified]));
        let other = WorkspaceChange {
            paths: vec!["src/a.rs".into()],
            kind: "modified".into(),
        };
        assert!(!changes_include_workspace_excludes(&[other]));
    }

    #[test]
    fn excluded_paths_are_ignored() {
        let root = temp_root();
        let git_obj = root.join(".git/objects/ab/cd");
        fs::create_dir_all(git_obj.parent().unwrap()).unwrap();
        fs::write(&git_obj, b"x").unwrap();
        let ev = make_event(
            EventKind::Remove(RemoveKind::Any),
            &[git_obj.to_str().unwrap()],
        );
        assert!(classify_event(&ev, &root).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn watcher_exclude_is_hard_cut() {
        let _lock = crate::workspace::filter::lock_excludes_cache_for_test();
        let prev = crate::workspace::filter::active_workspace_excludes();
        let mut lists = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        lists.watcher_exclude.push("**/heavy/**".into());
        crate::workspace::filter::activate_workspace_excludes(lists);
        let root = temp_root();
        let path = root.join("heavy").join("eval.rs");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"fn x() {}\n").unwrap();
        assert!(path_excluded("heavy/eval.rs", FilterPreset::Watcher));
        assert!(!path_excluded("heavy/eval.rs", FilterPreset::Search));
        let ev = make_event(
            EventKind::Modify(ModifyKind::Any),
            &[path.to_str().unwrap()],
        );
        assert!(
            classify_event(&ev, &root).is_none(),
            "watcher_exclude must not reach the bus"
        );
        crate::workspace::filter::activate_workspace_excludes(prev);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mcp_and_custom_tools_json_are_broadcast() {
        let _lock = crate::workspace::filter::lock_excludes_cache_for_test();
        let prev = crate::workspace::filter::active_workspace_excludes();
        let root = temp_root();
        crate::workspace::filter::activate_workspace_excludes(
            crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
        );
        for name in ["mcp.json", "custom_tools.json"] {
            let path = root.join(".litecode").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"{}").unwrap();
            let ev = make_event(
                EventKind::Modify(ModifyKind::Any),
                &[path.to_str().unwrap()],
            );
            let (paths, _) = classify_event(&ev, &root).expect(name);
            assert_eq!(paths, vec![format!(".litecode/{name}")]);
        }
        crate::workspace::filter::activate_workspace_excludes(prev);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ui_filter_uses_explorer_keeps_watcher_only_trees() {
        let _lock = crate::workspace::filter::lock_excludes_cache_for_test();
        let prev = crate::workspace::filter::active_workspace_excludes();
        crate::workspace::filter::activate_workspace_excludes(
            crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
        );
        // Explorer is `files.exclude` only — a path that is not on that list stays.
        let watcher_only = filter_change_for_ui(changed(&[".data/eval.rs"])).unwrap();
        assert_eq!(watcher_only.paths, vec![".data/eval.rs".to_string()]);
        let dropped = filter_change_for_ui(changed(&[".git/config"]));
        assert!(dropped.is_none(), "UI Explorer uses files.exclude");
        let mixed = filter_change_for_ui(changed(&[
            ".git/config",
            "src/a.rs",
            ".litecode/excludes.json",
            ".litecode/mcp.json",
            ".litecode/custom_tools.json",
        ]))
        .unwrap();
        let mut paths = mixed.paths;
        paths.sort();
        assert_eq!(
            paths,
            vec![
                ".litecode/custom_tools.json".to_string(),
                ".litecode/excludes.json".to_string(),
                ".litecode/mcp.json".to_string(),
                "src/a.rs".to_string()
            ]
        );
        crate::workspace::filter::activate_workspace_excludes(prev);
    }

    #[test]
    fn paths_outside_root_are_ignored() {
        let root = temp_root();
        let outside = std::env::temp_dir().join(format!("litecode_outside_{}", std::process::id()));
        fs::write(&outside, b"x").unwrap();
        let ev = make_event(
            EventKind::Remove(RemoveKind::Any),
            &[outside.to_str().unwrap()],
        );
        assert!(classify_event(&ev, &root).is_none());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn multiple_paths_in_one_event_all_returned() {
        let root = temp_root();
        let f1 = root.join("d1.txt");
        let f2 = root.join("d2.txt");
        fs::write(&f1, b"x").unwrap();
        fs::write(&f2, b"x").unwrap();
        let ev = make_event(
            EventKind::Modify(ModifyKind::Any),
            &[f1.to_str().unwrap(), f2.to_str().unwrap()],
        );
        let (paths, is_deleted) = classify_event(&ev, &root).unwrap();
        assert!(!is_deleted);
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"d1.txt".to_string()));
        assert!(paths.contains(&"d2.txt".to_string()));
        let _ = fs::remove_dir_all(&root);
    }

    // --- coalesce_pending (the atomic-save fix) -----------------------------

    #[test]
    fn genuine_delete_emits_deleted() {
        let mut pending = HashSet::new();
        let mut pending_deleted = HashSet::from(["a.txt".to_string()]);
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| false);
        assert_eq!(changes, vec![deleted(&["a.txt"])]);
    }

    #[test]
    fn atomic_save_overwrite_is_reclassified_as_modified() {
        // Original path is removed then recreated by rename; at flush it exists.
        let mut pending = HashSet::new();
        let mut pending_deleted = HashSet::from(["a.txt".to_string()]);
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| true);
        assert_eq!(changes, vec![changed(&["a.txt"])]);
        assert!(!changes.iter().any(|c| c.kind == "deleted"));
    }

    #[test]
    fn delete_then_recreate_coalesces_to_modified() {
        // Same path appears in both sets within the debounce window.
        let mut pending = HashSet::from(["a.txt".to_string()]);
        let mut pending_deleted = HashSet::from(["a.txt".to_string()]);
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| true);
        assert_eq!(changes, vec![changed(&["a.txt"])]);
    }

    #[test]
    fn mixed_delete_and_overwrite_in_one_flush() {
        let mut pending = HashSet::new();
        let mut pending_deleted = HashSet::from(["gone.txt".to_string(), "kept.txt".to_string()]);
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |rel| rel == "kept.txt");
        assert!(changes.contains(&deleted(&["gone.txt"])));
        assert!(changes.contains(&changed(&["kept.txt"])));
    }

    #[test]
    fn multiple_genuine_deletes_coalesce_into_one_event() {
        let mut pending = HashSet::new();
        let mut pending_deleted = HashSet::from(["x.txt".to_string(), "y.txt".to_string()]);
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| false);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "deleted");
        let mut paths = changes[0].paths.clone();
        paths.sort();
        assert_eq!(paths, vec!["x.txt".to_string(), "y.txt".to_string()]);
    }

    #[test]
    fn no_pending_events_emit_nothing() {
        let mut pending = HashSet::new();
        let mut pending_deleted = HashSet::new();
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| false);
        assert!(changes.is_empty());
    }

    #[test]
    fn pure_modify_emits_modified() {
        let mut pending = HashSet::from(["m.txt".to_string()]);
        let mut pending_deleted = HashSet::new();
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| false);
        assert_eq!(changes, vec![changed(&["m.txt"])]);
    }

    #[test]
    fn overwritten_path_in_modified_set_is_deduped() {
        // A path that was both modified and (spuriously) removed collapses to one.
        let mut pending = HashSet::from(["z.txt".to_string()]);
        let mut pending_deleted = HashSet::from(["z.txt".to_string()]);
        let changes = coalesce_pending(&mut pending, &mut pending_deleted, |_| true);
        assert_eq!(changes, vec![changed(&["z.txt"])]);
    }
}
