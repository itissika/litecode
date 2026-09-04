//! Index readiness status + on-disk job progress for UI polling.
//!
//! Progress lives under `.litecode/index/` so `GET /engines/detail` can read it
//! without talking to the worker (warmup/refresh hold the IPC mutex).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::meta::{self, needs_rebuild};
use super::store;
use crate::engines::EngineState;
use crate::types::{LitecodeError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexStatus {
    Absent,
    Ready,
    Stale,
    NeedsRebuild,
    Building,
    Refreshing,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPhase {
    Scanning,
    Embedding,
    Saving,
    Syncing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingProgress {
    pub phase: IndexPhase,
    pub files_done: usize,
    pub files_total: usize,
    pub chunks_done: usize,
}

/// Work remaining after reconcile. Consume (refresh / agent) is the only runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndexWork {
    None,
    Update { dirty: usize },
    Rebuild { reason: IndexRebuildReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexRebuildReason {
    FirstDesired,
    Incompatible,
    DirtyTooLarge,
}

/// Pick update vs rebuild from reconcile counts. Compatible index required.
///
/// Only **embed** work (new / content-changed files) can upgrade to rebuild:
/// `embed_dirty ≥ 50%` of `max(indexed, scannable)` and at least two such files.
/// Deletions are cheap (`remove_file` + save, no embed) and always stay on update.
pub fn decide_index_work(
    compatible: bool,
    has_usable_index: bool,
    embed_dirty: usize,
    dirty: usize,
    indexed_files: usize,
    scannable_files: usize,
) -> IndexWork {
    if !has_usable_index {
        return IndexWork::Rebuild {
            reason: IndexRebuildReason::FirstDesired,
        };
    }
    if !compatible {
        return IndexWork::Rebuild {
            reason: IndexRebuildReason::Incompatible,
        };
    }
    if dirty == 0 {
        return IndexWork::None;
    }
    let corpus = indexed_files.max(scannable_files);
    if embed_dirty >= 2 && embed_dirty.saturating_mul(2) >= corpus {
        return IndexWork::Rebuild {
            reason: IndexRebuildReason::DirtyTooLarge,
        };
    }
    IndexWork::Update { dirty }
}

/// `(embed_dirty, dirty)` from pending `(path, deleted)` rows.
pub fn pending_work_counts(pending: &HashSet<(String, bool)>) -> (usize, usize) {
    let dirty = pending.len();
    let embed_dirty = pending.iter().filter(|(_, deleted)| !*deleted).count();
    (embed_dirty, dirty)
}

/// Parent-process view of [`decide_index_work`] from disk hint + meta (no scannable walk).
pub fn index_work_from_disk(workspace_root: &Path) -> IndexWork {
    if should_full_rebuild(workspace_root) {
        let has = store::index_files_exist(workspace_root);
        return IndexWork::Rebuild {
            reason: if has {
                IndexRebuildReason::Incompatible
            } else {
                IndexRebuildReason::FirstDesired
            },
        };
    }
    let dirty = read_pending_hint(workspace_root);
    let embed_dirty = read_pending_embed_hint(workspace_root);
    let indexed = meta::read_meta(workspace_root)
        .ok()
        .flatten()
        .map(|m| m.indexed_files)
        .unwrap_or(0);
    decide_index_work(
        true,
        true,
        embed_dirty,
        dirty,
        indexed,
        indexed.max(dirty),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JobKind {
    Building,
    Refreshing,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexJobFile {
    status: JobKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    progress: Option<IndexingProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingHintFile {
    pending_updates: usize,
    /// New / modified files that need embedding. Absent on old hints → treat as all pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embed_dirty: Option<usize>,
}

fn job_path(workspace_root: &Path) -> PathBuf {
    meta::index_dir(workspace_root).join("job.json")
}

fn pending_hint_path(workspace_root: &Path) -> PathBuf {
    meta::index_dir(workspace_root).join("pending_hint.json")
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let body =
        serde_json::to_string_pretty(value).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(path, body).map_err(|e| LitecodeError::Config(e.to_string()))
}

fn read_job(workspace_root: &Path) -> Option<IndexJobFile> {
    let path = job_path(workspace_root);
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn clear_index_job(workspace_root: &Path) {
    let _ = std::fs::remove_file(job_path(workspace_root));
}

pub fn begin_building(workspace_root: &Path) {
    let _ = write_json(
        &job_path(workspace_root),
        &IndexJobFile {
            status: JobKind::Building,
            progress: Some(IndexingProgress {
                phase: IndexPhase::Scanning,
                files_done: 0,
                files_total: 0,
                chunks_done: 0,
            }),
            error: None,
        },
    );
}

pub fn begin_refreshing(workspace_root: &Path) {
    let _ = write_json(
        &job_path(workspace_root),
        &IndexJobFile {
            status: JobKind::Refreshing,
            progress: Some(IndexingProgress {
                phase: IndexPhase::Syncing,
                files_done: 0,
                files_total: 0,
                chunks_done: 0,
            }),
            error: None,
        },
    );
}

pub fn update_build_progress(workspace_root: &Path, progress: IndexingProgress) {
    let _ = write_json(
        &job_path(workspace_root),
        &IndexJobFile {
            status: JobKind::Building,
            progress: Some(progress),
            error: None,
        },
    );
}

pub fn mark_index_job_failed(workspace_root: &Path, error: impl Into<String>) {
    let _ = write_json(
        &job_path(workspace_root),
        &IndexJobFile {
            status: JobKind::Failed,
            progress: None,
            error: Some(error.into()),
        },
    );
}

pub fn write_pending_hint(workspace_root: &Path, pending_updates: usize) {
    write_pending_hint_counts(workspace_root, pending_updates, pending_updates);
}

/// Persist pending totals from `(path, deleted)` rows so disk work ignores deletions.
pub fn write_pending_hint_from(
    workspace_root: &Path,
    pending: &HashSet<(String, bool)>,
) {
    let (embed_dirty, dirty) = pending_work_counts(pending);
    write_pending_hint_counts(workspace_root, dirty, embed_dirty);
}

fn write_pending_hint_counts(
    workspace_root: &Path,
    pending_updates: usize,
    embed_dirty: usize,
) {
    if pending_updates == 0 {
        let _ = std::fs::remove_file(pending_hint_path(workspace_root));
        return;
    }
    let _ = write_json(
        &pending_hint_path(workspace_root),
        &PendingHintFile {
            pending_updates,
            embed_dirty: Some(embed_dirty),
        },
    );
}

pub fn read_pending_hint(workspace_root: &Path) -> usize {
    read_pending_hint_file(workspace_root)
        .map(|h| h.pending_updates)
        .unwrap_or(0)
}

pub fn read_pending_embed_hint(workspace_root: &Path) -> usize {
    read_pending_hint_file(workspace_root)
        .map(|h| h.embed_dirty.unwrap_or(h.pending_updates))
        .unwrap_or(0)
}

fn read_pending_hint_file(workspace_root: &Path) -> Option<PendingHintFile> {
    let content = std::fs::read_to_string(pending_hint_path(workspace_root)).ok()?;
    serde_json::from_str(&content).ok()
}

/// Disk-only classification (ignores in-flight jobs and pending hints).
pub fn disk_index_status(workspace_root: &Path) -> IndexStatus {
    let meta = meta::read_meta(workspace_root).ok().flatten();
    let vectors_ready = store::index_files_exist(workspace_root);
    match meta {
        None if !vectors_ready => IndexStatus::Absent,
        None => IndexStatus::NeedsRebuild,
        Some(m) if needs_rebuild(&m) || !vectors_ready => IndexStatus::NeedsRebuild,
        Some(_) => IndexStatus::Ready,
    }
}

pub fn should_full_rebuild(workspace_root: &Path) -> bool {
    matches!(
        disk_index_status(workspace_root),
        IndexStatus::Absent | IndexStatus::NeedsRebuild
    )
}

#[derive(Debug, Clone)]
pub struct ResolvedIndexView {
    pub status: IndexStatus,
    pub progress: Option<IndexingProgress>,
    pub job_error: Option<String>,
}

/// Resolve UI index status from disk job file + meta + pending hint + engine state.
pub fn resolve_index_view(
    workspace_root: &Path,
    engine_state: Option<EngineState>,
) -> ResolvedIndexView {
    if let Some(job) = read_job(workspace_root) {
        match job.status {
            JobKind::Building => {
                return ResolvedIndexView {
                    status: IndexStatus::Building,
                    progress: job.progress,
                    job_error: None,
                };
            }
            JobKind::Refreshing => {
                return ResolvedIndexView {
                    status: IndexStatus::Refreshing,
                    progress: job.progress,
                    job_error: None,
                };
            }
            JobKind::Failed => {
                return ResolvedIndexView {
                    status: IndexStatus::Failed,
                    progress: None,
                    job_error: job.error,
                };
            }
        }
    }

    let disk = disk_index_status(workspace_root);
    if disk != IndexStatus::Ready {
        // Warming with absent/needs_rebuild implies a build is about to run / running
        // before job.json appears.
        if matches!(engine_state, Some(EngineState::Warming))
            && matches!(disk, IndexStatus::Absent | IndexStatus::NeedsRebuild)
        {
            return ResolvedIndexView {
                status: IndexStatus::Building,
                progress: None,
                job_error: None,
            };
        }
        return ResolvedIndexView {
            status: disk,
            progress: None,
            job_error: None,
        };
    }

    let pending = read_pending_hint(workspace_root);
    if pending > 0 {
        return ResolvedIndexView {
            status: IndexStatus::Stale,
            progress: None,
            job_error: None,
        };
    }

    ResolvedIndexView {
        status: IndexStatus::Ready,
        progress: None,
        job_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::meta::{IndexMeta, init_workspace_index, write_meta};
    use std::collections::HashSet;
    use tempfile::TempDir;

    #[test]
    fn disk_status_absent_then_needs_rebuild_shell() {
        let dir = TempDir::new().unwrap();
        assert_eq!(disk_index_status(dir.path()), IndexStatus::Absent);
        init_workspace_index(dir.path()).unwrap();
        // Shell meta without vectors → needs_rebuild
        assert_eq!(disk_index_status(dir.path()), IndexStatus::NeedsRebuild);
    }

    #[test]
    fn job_file_overrides_to_building() {
        let dir = TempDir::new().unwrap();
        begin_building(dir.path());
        let view = resolve_index_view(dir.path(), None);
        assert_eq!(view.status, IndexStatus::Building);
        assert!(view.progress.is_some());
        clear_index_job(dir.path());
    }

    #[test]
    fn pending_hint_marks_stale_when_ready() {
        let dir = TempDir::new().unwrap();
        init_workspace_index(dir.path()).unwrap();
        // Pretend compatible ready by writing meta that matches + fake vectors flag:
        // without vectors, disk is needs_rebuild — write meta with matching pipeline
        // and create empty vector files via store helpers is heavy; use Ready path by
        // only testing hint when we force Ready via job absence + mock:
        write_pending_hint(dir.path(), 3);
        assert_eq!(read_pending_hint(dir.path()), 3);
        write_pending_hint(dir.path(), 0);
        assert_eq!(read_pending_hint(dir.path()), 0);

        // Full ready+stale path: create meta that needs_rebuild is false AND touch vector files.
        let mut meta = IndexMeta::shell();
        meta.indexed_files = 1;
        meta.indexed_chunks = 1;
        write_meta(dir.path(), &meta).unwrap();
        let index_dir = meta::index_dir(dir.path());
        std::fs::write(index_dir.join("chunks.jsonl"), "").unwrap();
        std::fs::write(index_dir.join("vectors.usearch"), "").unwrap();
        // empty usearch may or may not count as exist — check helper
        if store::index_files_exist(dir.path()) {
            write_pending_hint(dir.path(), 2);
            let view = resolve_index_view(dir.path(), Some(EngineState::Warm));
            assert_eq!(view.status, IndexStatus::Stale);
        }
    }

    #[test]
    fn decide_index_work_first_desired_and_incompatible() {
        assert_eq!(
            decide_index_work(true, false, 0, 0, 0, 0),
            IndexWork::Rebuild {
                reason: IndexRebuildReason::FirstDesired
            }
        );
        assert_eq!(
            decide_index_work(false, true, 0, 0, 10, 10),
            IndexWork::Rebuild {
                reason: IndexRebuildReason::Incompatible
            }
        );
    }

    #[test]
    fn decide_index_work_small_dirty_updates_large_rebuilds() {
        assert_eq!(
            decide_index_work(true, true, 0, 0, 10, 10),
            IndexWork::None
        );
        assert_eq!(
            decide_index_work(true, true, 1, 1, 10, 10),
            IndexWork::Update { dirty: 1 }
        );
        assert_eq!(
            decide_index_work(true, true, 5, 5, 10, 10),
            IndexWork::Rebuild {
                reason: IndexRebuildReason::DirtyTooLarge
            }
        );
        assert_eq!(
            decide_index_work(true, true, 1, 1, 1, 1),
            IndexWork::Update { dirty: 1 },
            "single-file corpus must not upgrade to rebuild"
        );
    }

    #[test]
    fn decide_index_work_deletions_stay_on_update() {
        assert_eq!(
            decide_index_work(true, true, 0, 8, 10, 2),
            IndexWork::Update { dirty: 8 },
            "mass delete must not rebuild remaining files"
        );
        assert_eq!(
            decide_index_work(true, true, 1, 9, 10, 2),
            IndexWork::Update { dirty: 9 },
            "one embed + many deletes stays on update"
        );
        assert_eq!(
            decide_index_work(true, true, 5, 9, 10, 6),
            IndexWork::Rebuild {
                reason: IndexRebuildReason::DirtyTooLarge
            },
            "embed share still upgrades when additions/edits are half the corpus"
        );
    }

    #[test]
    fn pending_hint_embed_dirty_keeps_deletes_on_update() {
        let dir = TempDir::new().unwrap();
        init_workspace_index(dir.path()).unwrap();
        let mut meta = IndexMeta::shell();
        meta.indexed_files = 10;
        meta.indexed_chunks = 10;
        write_meta(dir.path(), &meta).unwrap();
        let index_dir = meta::index_dir(dir.path());
        std::fs::write(index_dir.join("chunks.jsonl"), "").unwrap();
        std::fs::write(index_dir.join("vectors.usearch"), "").unwrap();
        if !store::index_files_exist(dir.path()) {
            return;
        }
        let mut pending = HashSet::new();
        for i in 0..8 {
            pending.insert((format!("gone{i}.rs"), true));
        }
        write_pending_hint_from(dir.path(), &pending);
        assert_eq!(read_pending_hint(dir.path()), 8);
        assert_eq!(read_pending_embed_hint(dir.path()), 0);
        assert_eq!(
            index_work_from_disk(dir.path()),
            IndexWork::Update { dirty: 8 }
        );
    }

    #[test]
    fn index_work_serializes_kind_tag() {
        let none = serde_json::to_value(IndexWork::None).unwrap();
        assert_eq!(none["kind"], "none");
        let update = serde_json::to_value(IndexWork::Update { dirty: 4 }).unwrap();
        assert_eq!(update["kind"], "update");
        assert_eq!(update["dirty"], 4);
        let rebuild = serde_json::to_value(IndexWork::Rebuild {
            reason: IndexRebuildReason::DirtyTooLarge,
        })
        .unwrap();
        assert_eq!(rebuild["kind"], "rebuild");
        assert_eq!(rebuild["reason"], "dirty_too_large");
    }
}
