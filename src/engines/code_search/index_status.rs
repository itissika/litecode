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

/// File-level build phase kept inside `job.json` for logs (no longer exposed
/// in engines detail since the WP-131 cleanup). Consumers read
/// [`code_corpus_progress`] (chunk-exact).
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
}

/// Pick update vs rebuild from reconcile counts. Compatible index required.
///
/// Rebuild only for first-time / incompatible indexes (WP-057). Dirty volume —
/// including large embed dirty — always stays on incremental [`IndexWork::Update`].
/// `embed_dirty` / corpus counts remain in the signature for callers and tests.
pub fn decide_index_work(
    compatible: bool,
    has_usable_index: bool,
    _embed_dirty: usize,
    dirty: usize,
    _indexed_files: usize,
    _scannable_files: usize,
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
    // WP-131: exact queue record.
    let CodeQueue {
        queued: dirty,
        embed_dirty,
        ..
    } = read_code_queue(workspace_root);
    let indexed = meta::read_meta(workspace_root)
        .ok()
        .flatten()
        .map(|m| m.indexed_files)
        .unwrap_or(0);
    decide_index_work(true, true, embed_dirty, dirty, indexed, indexed.max(dirty))
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
    /// Chunk-exact progress (WP-131). `total == None` ⇔ scanning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chunks: Option<JobChunks>,
}

/// Chunk counters for the in-flight job. Denominator = the whole index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct JobChunks {
    done: u64,
    #[serde(default)]
    total: Option<u64>,
}

fn job_path(workspace_root: &Path) -> PathBuf {
    meta::index_dir(workspace_root).join("job.json")
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let body =
        serde_json::to_string_pretty(value).map_err(|e| LitecodeError::Config(e.to_string()))?;
    // L5: atomic — temp file in the same dir, then rename over the target, so
    // readers never see a torn job.json / queue.json.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_file_name(format!(
        ".{}.{}.{seq}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        std::process::id()
    ));
    std::fs::write(&tmp, body).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        LitecodeError::Config(e.to_string())
    })
}

/// Clear a job left Building/Refreshing by a dead worker. **Worker load path
/// only** (code_search_ipc initialize, before warmup): at that point no task
/// of this workspace's worker can be running, so any running job is stale.
/// The serve process (tick / start / status) never calls this — a worker
/// process may own the job, and in-process flags cannot see it.
/// A stale incremental job is re-queued via `queue.json` so the next tick
/// reconciles; a stale full build needs no entry (warmup rebuilds).
/// Returns true when a stale job was reclaimed.
pub fn reclaim_stale_index_job(workspace_root: &Path) -> bool {
    let Some(job) = read_job(workspace_root) else {
        return false;
    };
    match job.status {
        JobKind::Failed => false,
        JobKind::Building => {
            clear_index_job(workspace_root);
            true
        }
        JobKind::Refreshing => {
            clear_index_job(workspace_root);
            let q = read_code_queue(workspace_root);
            write_code_queue_counts(workspace_root, q.queued.max(1), q.embed_dirty.max(1), None);
            true
        }
    }
}

fn read_job(workspace_root: &Path) -> Option<IndexJobFile> {
    let path = job_path(workspace_root);
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn clear_index_job(workspace_root: &Path) {
    let _ = std::fs::remove_file(job_path(workspace_root));
}


/// Chunks currently in the on-disk index (0 when absent).
pub fn indexed_chunk_count(workspace_root: &Path) -> u64 {
    meta::read_meta(workspace_root)
        .ok()
        .flatten()
        .map_or(0, |m| m.indexed_chunks as u64)
}

pub fn begin_refreshing(workspace_root: &Path) {
    let done = done_floor(workspace_root);
    #[cfg(test)]
    CHUNK_PROGRESS_TRACE.with(|t| t.borrow_mut().push((done, None)));
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
            chunks: Some(JobChunks { done, total: None }),
        },
    );
}

/// `done` to report while no chunk is being embedded: the chunks that survive
/// the queued work when something is queued, else the whole index.
fn done_floor(workspace_root: &Path) -> u64 {
    let q = read_code_queue(workspace_root);
    if q.queued > 0 {
        q.kept_chunks
    } else {
        indexed_chunk_count(workspace_root)
    }
}

/// Report chunk-exact progress for the in-flight job (keeps status + legacy
/// progress). `total == None` = still scanning. No-op without a running job.
#[cfg(test)]
thread_local! {
    /// Test-only trace of every chunk progress report on this thread.
    pub(crate) static CHUNK_PROGRESS_TRACE: std::cell::RefCell<Vec<(u64, Option<u64>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub fn update_chunk_progress(workspace_root: &Path, done: u64, total: Option<u64>) {
    #[cfg(test)]
    CHUNK_PROGRESS_TRACE.with(|t| t.borrow_mut().push((done, total)));
    let Some(mut job) = read_job(workspace_root) else {
        return;
    };
    if job.status == JobKind::Failed {
        return;
    }
    job.chunks = Some(JobChunks { done, total });
    let _ = write_json(&job_path(workspace_root), &job);
}

pub fn mark_index_job_failed(workspace_root: &Path, error: impl Into<String>) {
    let _ = write_json(
        &job_path(workspace_root),
        &IndexJobFile {
            status: JobKind::Failed,
            progress: None,
            error: Some(error.into()),
            chunks: None,
        },
    );
}

/// Exact pending-queue record (WP-131). Written by the worker whenever its
/// `pending_updates` queue changes; read by the parent tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeQueue {
    /// Queued paths (including deletions).
    pub queued: usize,
    /// Queued paths that need embedding (deletions excluded).
    pub embed_dirty: usize,
    /// Index chunks that survive this queue (index minus old chunks of every
    /// queued path), fixed at enqueue time. Idle and refresh views report
    /// `done = kept_chunks` while work is queued, so `done` never drops when
    /// the refresh starts (same rule as Session's `SessionQueue.kept_chunks`).
    #[serde(default)]
    pub kept_chunks: u64,
}

fn queue_path(workspace_root: &Path) -> PathBuf {
    meta::index_dir(workspace_root).join("queue.json")
}

/// Persist the worker queue (exact counts; `kept_chunks` falls back to the
/// previous / on-disk value).
pub fn write_code_queue(workspace_root: &Path, pending: &HashSet<(String, bool)>) {
    let (embed_dirty, queued) = pending_work_counts(pending);
    write_code_queue_counts(workspace_root, queued, embed_dirty, None);
}

/// Like [`write_code_queue`] with the exact surviving chunk count (worker side,
/// computed from the live index).
pub fn write_code_queue_kept(
    workspace_root: &Path,
    pending: &HashSet<(String, bool)>,
    kept_chunks: u64,
) {
    let (embed_dirty, queued) = pending_work_counts(pending);
    write_code_queue_counts(workspace_root, queued, embed_dirty, Some(kept_chunks));
}

/// `kept == None` (writer without index access): keep the previous exact
/// value, else fall back to the on-disk chunk count.
fn write_code_queue_counts(
    workspace_root: &Path,
    queued: usize,
    embed_dirty: usize,
    kept: Option<u64>,
) {
    if queued == 0 {
        let _ = std::fs::remove_file(queue_path(workspace_root));
        return;
    }
    let _ = write_json(
        &queue_path(workspace_root),
        &CodeQueue {
            queued,
            embed_dirty,
            kept_chunks: kept.unwrap_or_else(|| {
                let prev = read_code_queue(workspace_root);
                if prev.queued > 0 {
                    prev.kept_chunks
                } else {
                    indexed_chunk_count(workspace_root)
                }
            }),
        },
    );
}

/// Exact signal for the periodic tick: queued work on disk (no hint needed).
pub fn read_code_queue(workspace_root: &Path) -> CodeQueue {
    std::fs::read_to_string(queue_path(workspace_root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Disk-only classification (ignores in-flight jobs and the queue).
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

/// Resolve UI index status from disk job file + meta + queue.json + engine state.
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

    if read_code_queue(workspace_root).queued > 0 {
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

/// Inputs for [`code_corpus_progress`] that live outside the index directory.
#[derive(Debug, Clone, Copy)]
pub struct CodeEngineFacts {
    /// Persisted workspace intent (engine enabled).
    pub desired: bool,
    pub state: Option<EngineState>,
    pub model_ready: bool,
}

/// Chunk-exact code corpus progress (WP-131 frozen interface).
///
/// Failure order: model missing → engine unavailable → engine stopped (only
/// when desired) → index job failed. Warming is not a state of its own.
pub fn code_corpus_progress(
    workspace_root: &Path,
    facts: CodeEngineFacts,
) -> crate::engines::progress::CorpusProgress {
    use crate::engines::progress::{CorpusError, CorpusProgress, FailReason, ProgressEngine};
    let indexed = indexed_chunk_count(workspace_root);
    if facts.desired {
        if !facts.model_ready {
            return CorpusProgress::failed(
                CorpusError::new(
                    ProgressEngine::Model,
                    FailReason::ModelMissing,
                    "embedding model is missing",
                ),
                indexed,
            );
        }
        match facts.state {
            Some(EngineState::Failed) => {
                return CorpusProgress::failed(
                    CorpusError::new(
                        ProgressEngine::Code,
                        FailReason::EngineUnavailable,
                        "code search engine is unavailable",
                    ),
                    indexed,
                );
            }
            Some(EngineState::Stopped) => {
                return CorpusProgress::failed(
                    CorpusError::new(
                        ProgressEngine::Code,
                        FailReason::EngineStopped,
                        "code search engine is stopped",
                    ),
                    indexed,
                );
            }
            _ => {}
        }
    }
    // files_pending = exact queued files; chunks_pending only when exact.
    let files = read_code_queue(workspace_root).queued as u64;
    if let Some(job) = read_job(workspace_root) {
        match job.status {
            JobKind::Failed => {
                return CorpusProgress::failed(
                    CorpusError::new(
                        ProgressEngine::Code,
                        FailReason::IndexFailed,
                        job.error.unwrap_or_else(|| "index job failed".into()),
                    ),
                    indexed,
                );
            }
            JobKind::Building | JobKind::Refreshing => {
                return match job.chunks {
                    Some(JobChunks {
                        done,
                        total: Some(total),
                    }) => CorpusProgress::embedding(done, total, total.saturating_sub(done))
                        .with_files_pending(files),
                    Some(JobChunks { done, total: None }) => {
                        CorpusProgress::scanning(done, 0).with_files_pending(files)
                    }
                    // Job written by an older binary: no chunk counters yet.
                    None => CorpusProgress::scanning(indexed, 0).with_files_pending(files),
                };
            }
        }
    }
    match disk_index_status(workspace_root) {
        IndexStatus::Absent | IndexStatus::NeedsRebuild => {
            if facts.desired && matches!(facts.state, Some(EngineState::Warming)) {
                // Build about to start; job.json not written yet.
                CorpusProgress::scanning(0, 0).with_files_pending(files)
            } else {
                CorpusProgress::needs_rebuild(indexed, indexed)
            }
        }
        _ => CorpusProgress::ready(done_floor(workspace_root), 0).with_files_pending(files),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    use std::collections::HashSet;
    use tempfile::TempDir;

    #[test]
    fn decide_index_work_dirty_always_updates() {
        assert_eq!(decide_index_work(true, true, 0, 0, 10, 10), IndexWork::None);
        assert_eq!(
            decide_index_work(true, true, 1, 1, 10, 10),
            IndexWork::Update { dirty: 1 }
        );
        assert_eq!(
            decide_index_work(true, true, 5, 5, 10, 10),
            IndexWork::Update { dirty: 5 },
            "WP-057: large embed dirty must not rebuild"
        );
        assert_eq!(
            decide_index_work(true, true, 1, 1, 1, 1),
            IndexWork::Update { dirty: 1 },
            "single-file corpus stays on update"
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
            IndexWork::Update { dirty: 9 },
            "WP-057: embed share never upgrades to DirtyTooLarge rebuild"
        );
    }

    fn facts(state: Option<EngineState>) -> CodeEngineFacts {
        CodeEngineFacts {
            desired: true,
            state,
            model_ready: true,
        }
    }

    #[test]
    fn corpus_job_scanning_then_embedding() {
        use crate::engines::progress::CorpusPhase;
        let dir = tempfile::TempDir::new().unwrap();
        // WP-143: first build is a refresh job; scanning then embedding.
        begin_refreshing(dir.path());
        let p = code_corpus_progress(dir.path(), facts(Some(EngineState::Warming)));
        assert_eq!(p.phase, Some(CorpusPhase::Scanning));
        assert_eq!(p.chunks_total, None);
        update_chunk_progress(dir.path(), 3, Some(10));
        let p = code_corpus_progress(dir.path(), facts(Some(EngineState::Warming)));
        assert_eq!(p.phase, Some(CorpusPhase::Embedding));
        assert_eq!((p.chunks_done, p.chunks_total), (3, Some(10)));
    }

    // Stuck-Running (worker load): a stale Refreshing job is cleared and
    // re-queued; a stale Building is just cleared; Failed is kept.

    // L5: state files are written via temp + rename; no temp left behind and
    // a reader never sees partial JSON.
    #[test]
    fn job_and_queue_writes_are_atomic() {
        let dir = TempDir::new().unwrap();
        let mut q = HashSet::new();
        q.insert(("a.rs".to_string(), false));
        write_code_queue(dir.path(), &q);
        begin_refreshing(dir.path());
        update_chunk_progress(dir.path(), 1, Some(2));
        let idx = meta::index_dir(dir.path());
        let names: Vec<String> = std::fs::read_dir(&idx)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        assert!(
            serde_json::from_str::<IndexJobFile>(
                &std::fs::read_to_string(idx.join("job.json")).unwrap()
            )
            .is_ok()
        );
        assert_eq!(read_code_queue(dir.path()).queued, 1);
        // Concurrent writer/reader: every read parses.
        let root = dir.path().to_path_buf();
        let w = std::thread::spawn(move || {
            for i in 0..300 {
                update_chunk_progress(&root, i, Some(300));
            }
        });
        for _ in 0..300 {
            if let Ok(s) = std::fs::read_to_string(idx.join("job.json")) {
                assert!(
                    serde_json::from_str::<IndexJobFile>(&s).is_ok(),
                    "torn read: {s}"
                );
            }
        }
        w.join().unwrap();
    }
}
