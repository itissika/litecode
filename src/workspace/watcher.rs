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
        let rescan_tx = workspace.rescan_sender();

        // `None` = the backend reported an error (WP-141: never dropped).
        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel::<Option<Event>>();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                let _ = notify_tx.send(notify_signal(res));
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
                        // WP-141: a watcher error, a Rescan flag, or a queue
                        // overflow means events were lost; the consumers run a
                        // full disk reconcile instead of trusting the stream.
                        let Some(event) = event.filter(|e| !event_requests_rescan(e)) else {
                            // Events were lost, so the media snapshot is not
                            // trusted either: the next listing walks again.
                            workspace.invalidate_media();
                            let _ = rescan_tx.send(());
                            continue;
                        };
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
                        let excludes_changed = changes_include_workspace_excludes(&changes);
                        if excludes_changed {
                            reload_workspace_excludes_from_disk(&rel_base);
                        }
                        // The tree moved outside the service. Patch the media
                        // snapshot for the paths we can name; a change to the
                        // ignore rules themselves alters what is visible at all,
                        // so that still drops the snapshot for a fresh walk.
                        if excludes_changed
                            || changes.iter().any(|change| {
                                change.paths.iter().any(|rel| rel_changes_ignore_rules(rel))
                            })
                        {
                            workspace.invalidate_media();
                        } else {
                            for change in &changes {
                                workspace.apply_media_paths(&change.paths);
                            }
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

/// Watcher callback result → loop signal. `None` = backend error: events may
/// be lost, so the loop requests a full rescan instead of dropping it.
fn notify_signal(res: Result<Event, notify::Error>) -> Option<Event> {
    match res {
        Ok(event) => Some(event),
        Err(error) => {
            tracing::warn!(error = %error, "workspace watcher error; requesting full rescan");
            None
        }
    }
}

/// Backend says its event stream is incomplete (inotify `Q_OVERFLOW`,
/// FSEvents `MustScanSubDirs`, Windows buffer overflow all map to `Rescan`).
fn event_requests_rescan(event: &Event) -> bool {
    event.need_rescan()
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

/// Keep editable `.litecode/*.json`, plan markdown, and the private knowledge
/// corpus; drop other product-internal trees (index writes); otherwise
/// `watcher_exclude` is a hard cut — no Search-line rescue.
fn event_rel_is_broadcast(rel: &str) -> bool {
    if is_editable_litecode_json(rel) || is_plan_file_rel(rel) || is_knowledge_corpus_rel(rel) {
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
    if is_editable_litecode_json(rel) || is_plan_file_rel(rel) || is_knowledge_corpus_rel(rel) {
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

/// A plan markdown file: exactly one level under `.litecode/plan/`, ending .md.
/// The atomic-create temp name (`.<slug>.md.tmp`) and nested paths stay out.
fn is_plan_file_rel(rel: &str) -> bool {
    let Some(name) = rel.strip_prefix(".litecode/plan/") else {
        return false;
    };
    !name.contains('/') && name.len() > 3 && name.ends_with(".md")
}

/// Private knowledge tree. Same path as `knowledge::root::PRIVATE_ROOT` and the
/// web `KNOWLEDGE_ROOT` (`.litecode/knowledge`). Checked before user excludes,
/// like plan files and editable `.litecode` json. Atomic-save temps stay out.
fn is_knowledge_corpus_rel(rel: &str) -> bool {
    let root = crate::knowledge::root::PRIVATE_ROOT;
    if rel != root && !rel.starts_with(&format!("{root}/")) {
        return false;
    }
    !rel.split('/').any(|seg| seg.contains(".litecode-tmp-"))
}

/// Edits that change which paths are visible to the Explorer face.
fn rel_changes_ignore_rules(rel: &str) -> bool {
    rel.ends_with(".gitignore") || rel.ends_with(".git/info/exclude")
}

fn changes_include_workspace_excludes(changes: &[WorkspaceChange]) -> bool {
    changes
        .iter()
        .any(|c| c.paths.iter().any(|p| is_workspace_excludes_rel(p)))
}

/// `HEAD` moved: commit, checkout, reset, or rebase.
///
/// Explorer excludes `.git`, so these paths never arrive on `workspace/changed`.
/// A symbol warning compares the worktree body with HEAD and needs this signal
/// to clear when the file itself did not change.
pub fn change_moves_head(change: &WorkspaceChange) -> bool {
    change.paths.iter().any(|path| moves_git_head(path))
}

fn moves_git_head(path: &str) -> bool {
    matches!(
        classify_root_git_meta(path),
        Some(GitMetaClass::Head | GitMetaClass::BranchRef)
    )
}

/// Source Control refresh for repository metadata that never reaches
/// `workspace/changed`. Worktree edits use that bus and only need `git status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitMetaRefresh {
    /// Index, upstream refs, or the `.git` directory itself. Status only.
    Status,
    /// A branch tip moved. Status and the commit log.
    History,
}

pub fn git_meta_refresh(change: &WorkspaceChange) -> Option<GitMetaRefresh> {
    let mut status = false;
    for path in &change.paths {
        match classify_root_git_meta(path) {
            Some(GitMetaClass::Head | GitMetaClass::BranchRef) => {
                return Some(GitMetaRefresh::History);
            }
            Some(GitMetaClass::Status) => status = true,
            None => {}
        }
    }
    status.then_some(GitMetaRefresh::Status)
}

#[derive(Clone, Copy)]
enum GitMetaClass {
    /// `HEAD` or its reflog. Knowledge drift cares about this specifically.
    Head,
    /// Loose branch tip (`refs/heads`, or the reflog of one).
    BranchRef,
    /// Staged state, upstream tips, or a bare `.git` directory event.
    Status,
}

/// Classify a path inside the workspace-root `.git`. Nested repos and lock
/// files are ignored: a lock is git's own temporary, and `git status` must
/// not retrigger itself by refreshing the index (`GIT_OPTIONAL_LOCKS=0`).
fn classify_root_git_meta(path: &str) -> Option<GitMetaClass> {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    let rest = path.strip_prefix(".git")?;
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    let rel = rest.trim_start_matches('/');
    if rel.is_empty() {
        return Some(GitMetaClass::Status);
    }
    if rel.split('/').any(|seg| seg.ends_with(".lock")) {
        return None;
    }
    if rel.starts_with("objects/")
        || rel.starts_with("lfs/")
        || rel.starts_with("hooks/")
        || rel.starts_with("subtree-cache/")
    {
        return None;
    }
    if rel == "HEAD" || rel == "logs/HEAD" {
        return Some(GitMetaClass::Head);
    }
    if rel.starts_with("refs/heads/") || rel.starts_with("logs/refs/heads/") {
        return Some(GitMetaClass::BranchRef);
    }
    if rel == "index"
        || rel == "packed-refs"
        || rel.starts_with("refs/remotes/")
        || rel.starts_with("logs/refs/remotes/")
    {
        return Some(GitMetaClass::Status);
    }
    None
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
    // WP-141: lost-event signals must request a full reconcile, plain events
    // must not.
    #[test]
    fn rescan_flag_and_overflow_request_full_rescan() {
        use notify::event::{CreateKind, Flag};
        let overflow = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(event_requests_rescan(&overflow));
        let rescan_on_create =
            Event::new(EventKind::Create(CreateKind::File)).set_flag(Flag::Rescan);
        assert!(event_requests_rescan(&rescan_on_create));
        let plain = Event::new(EventKind::Create(CreateKind::File));
        assert!(!event_requests_rescan(&plain));
        // Backend error is not dropped: it becomes the rescan signal.
        assert!(notify_signal(Err(notify::Error::generic("boom"))).is_none());
        assert!(notify_signal(Ok(plain)).is_some());
    }

    use super::*;
    use notify::event::{EventAttributes, RemoveKind};
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

    // --- classify_event -----------------------------------------------------

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

    // --- coalesce_pending (the atomic-save fix) -----------------------------

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

}
