//! Session-scoped agent pipe jobs: running list, async wait, and exit mailbox.
//!
//! A live command is owned by [`supervise_pipe`]. That task is the only code
//! that flips `detached`, and it does so in the same step as the oneshot it
//! sends to the tool. Stdout and stderr readers take no cancel token and read
//! until EOF, so a nested or turn cancel cannot stop them after detach.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::tee::BoundedTee;

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn output_file_rel(path: &Path, workspace_root: &Path) -> String {
    super::display_output_path(path, workspace_root)
}

fn pipe_log_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("run")
}

fn pipe_log_paths(workspace_root: &Path, id: &str) -> (PathBuf, PathBuf) {
    let dir = pipe_log_dir(workspace_root);
    (
        dir.join(format!("{id}.stdout")),
        dir.join(format!("{id}.stderr")),
    )
}

fn job_id_is_file_safe(id: &str) -> bool {
    !id.is_empty()
        && !id.contains(':')
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
}

/// `bg_` + 8 hex. Retries when the id or either log file is already taken.
/// The id is never a `call_id` (`:` is illegal in a Windows file name).
pub(crate) fn alloc_pipe_job_id(jobs: &AgentJobRegistry, workspace_root: &Path) -> String {
    for _ in 0..16 {
        let id = format!("bg_{}", super::bash_nonce());
        if jobs.get(&id).is_some() {
            continue;
        }
        let (stdout, stderr) = pipe_log_paths(workspace_root, &id);
        if stdout.exists() || stderr.exists() {
            continue;
        }
        return id;
    }
    format!("bg_{}", super::bash_nonce())
}

/// Kill a process tree by pid. Unix: the child's process group. Windows:
/// `taskkill /T /F`.
pub(crate) fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[derive(Debug, Clone)]
pub struct ExitNotice {
    pub bash_id: String,
    pub session_id: String,
    pub exit_code: Option<u32>,
    pub output_path: PathBuf,
    pub command_preview: String,
    /// True when the human stopped the job from the bash card (not a natural exit).
    pub user_killed: bool,
    /// True when `tools.process.kill` stopped the job (no human reminder mailbox).
    pub tool_killed: bool,
}

#[derive(Debug, Clone)]
pub struct RunningJob {
    pub id: String,
    pub command_preview: String,
    pub output_path: PathBuf,
}

/// Kill scope for one job's whole process tree.
///
/// Windows keeps a per-job Job Object created with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so a descendant that outlived the
/// shell can still be terminated and a runtime crash still reaps the tree.
/// Elsewhere the tree is the process group rooted at `pid`.
struct JobTree {
    pid: u32,
    #[cfg(windows)]
    job: Option<usize>,
}

impl JobTree {
    fn kill_tree(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.filter(|job| *job != 0) {
            use windows_sys::Win32::Foundation::HANDLE;
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            // SAFETY: the handle is owned by this struct and still open.
            unsafe {
                if TerminateJobObject(job as HANDLE, 1) != 0 {
                    return;
                }
            }
        }
        kill_process_tree(self.pid);
    }

    /// Stop killing the tree when this handle closes. Used once a job finished
    /// normally so a daemon it started is not reaped by our own cleanup.
    fn release_on_close(&self) {
        #[cfg(windows)]
        if let Some(job) = self.job.filter(|job| *job != 0) {
            use windows_sys::Win32::Foundation::HANDLE;
            use windows_sys::Win32::System::JobObjects::{
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            };
            // SAFETY: plain Win32 call on a handle this struct owns.
            unsafe {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = 0;
                let _ = SetInformationJobObject(
                    job as HANDLE,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
        }
    }
}

impl Drop for JobTree {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.filter(|job| *job != 0) {
            use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
            // SAFETY: this struct owns the handle and closes it exactly once.
            unsafe {
                CloseHandle(job as HANDLE);
            }
        }
    }
}

pub struct AgentJobRecord {
    pub session_id: String,
    pub call_id: String,
    pub command_preview: String,
    pub output_path: PathBuf,
    pub tee: Arc<Mutex<BoundedTee>>,
    /// The process has not been reaped yet. Flipped only where the supervisor
    /// observes the child exit; it never tracks the pipe readers.
    pub alive: bool,
    /// Both pipe readers hit EOF. A shell that exited while a descendant still
    /// holds the pipe is `!alive && !reader_eof`: draining, still owning its
    /// log, but no longer running.
    pub reader_eof: bool,
    pub exit_code: Option<u32>,
    pub started_at_ms: i64,
    pub user_killed: bool,
    pub tool_killed: bool,
    workspace_root: PathBuf,
    /// Stderr log. `bash/tail` reads stdout; the tool result reads both.
    stderr_path: PathBuf,
    stderr_tee: Arc<Mutex<BoundedTee>>,
    pid: Option<u32>,
    /// Kill scope for the tree. Set by the supervisor right after spawn on
    /// Windows; fixtures and Unix jobs fall back to `pid`.
    tree: Option<Arc<JobTree>>,
    /// Set only by [`supervise_pipe`] when it chooses the background or promoted
    /// oneshot. Fixture inserts may be born already detached; nothing else flips it.
    detached: bool,
}

impl AgentJobRecord {
    fn to_notice(&self, bash_id: &str) -> ExitNotice {
        ExitNotice {
            bash_id: bash_id.to_string(),
            session_id: self.session_id.clone(),
            exit_code: self.exit_code,
            output_path: self.output_path.clone(),
            command_preview: self.command_preview.clone(),
            user_killed: self.user_killed,
            tool_killed: self.tool_killed,
        }
    }
}

struct Waiter {
    session_id: String,
    call_id: String,
    watching_id: Option<String>,
    started_at_ms: i64,
    deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BashJobWire {
    pub id: String,
    #[serde(default)]
    pub call_id: String,
    pub command_preview: String,
    pub output_file: String,
    /// Stderr log. Absent on older snapshots; `bash/tail` still reads stdout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_file: Option<String>,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BashWaitWire {
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watching_id: Option<String>,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BashJobsSnapshot {
    #[serde(default)]
    pub jobs: Vec<BashJobWire>,
    #[serde(default)]
    pub waits: Vec<BashWaitWire>,
}

#[derive(Debug, Clone)]
pub struct BashTailView {
    pub text: String,
    pub truncated_on_disk: bool,
    pub alive: bool,
    /// The process exited but a pipe is still open, so the tail can still grow.
    pub draining: bool,
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct KilledJob {
    pub id: String,
    pub alive: bool,
    /// Readers already hit EOF. `!alive && reader_eof` means the job was
    /// already sealed when the kill arrived.
    pub reader_eof: bool,
    pub exit_code: Option<u32>,
    pub output_path: Option<PathBuf>,
}

struct JobState {
    jobs: HashMap<String, AgentJobRecord>,
    mailbox: HashMap<String, VecDeque<ExitNotice>>,
    waiters: HashMap<String, Waiter>,
    generation: u64,
}

pub struct AgentJobRegistry {
    inner: Mutex<JobState>,
    notify: tokio::sync::Notify,
    exit_handler: Mutex<Option<Arc<dyn Fn(ExitNotice) + Send + Sync>>>,
    jobs_changed: Mutex<Option<Arc<dyn Fn(String) + Send + Sync>>>,
}

impl Default for AgentJobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentJobRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(JobState {
                jobs: HashMap::new(),
                mailbox: HashMap::new(),
                waiters: HashMap::new(),
                generation: 0,
            }),
            notify: tokio::sync::Notify::new(),
            exit_handler: Mutex::new(None),
            jobs_changed: Mutex::new(None),
        }
    }

    pub fn set_exit_handler(&self, handler: Arc<dyn Fn(ExitNotice) + Send + Sync>) {
        *self.exit_handler.lock().expect("exit handler lock") = Some(handler);
    }

    pub fn set_jobs_changed_handler(&self, handler: Arc<dyn Fn(String) + Send + Sync>) {
        *self.jobs_changed.lock().expect("jobs changed lock") = Some(handler);
    }

    fn emit_jobs_changed(&self, session_id: &str) {
        if session_id.is_empty() || session_id == "_" {
            return;
        }
        if let Some(handler) = self.jobs_changed.lock().expect("jobs changed lock").clone() {
            handler(session_id.to_string());
        }
    }

    fn wake(&self) {
        self.notify.notify_waiters();
    }

    pub fn insert(&self, id: String, record: AgentJobRecord) {
        let session_id = record.session_id.clone();
        {
            let mut g = self.inner.lock().expect("jobs lock");
            g.jobs.insert(id, record);
            g.generation += 1;
        }
        self.wake();
        self.emit_jobs_changed(&session_id);
    }

    pub fn snapshot_capture(&self, id: &str) -> Option<super::tee::TeeCapture> {
        let g = self.inner.lock().expect("jobs lock");
        let rec = g.jobs.get(id)?;
        rec.tee.lock().ok().map(|t| t.snapshot_capture())
    }

    /// Stdout and stderr windows plus both log paths. The registry lock is not
    /// held while the tees are snapshotted.
    pub fn pipe_streams(
        &self,
        id: &str,
    ) -> Option<(
        super::tee::TeeCapture,
        super::tee::TeeCapture,
        PathBuf,
        PathBuf,
    )> {
        let (stdout_tee, stderr_tee, stdout_path, stderr_path) = {
            let g = self.inner.lock().expect("jobs lock");
            let rec = g.jobs.get(id)?;
            (
                Arc::clone(&rec.tee),
                Arc::clone(&rec.stderr_tee),
                rec.output_path.clone(),
                rec.stderr_path.clone(),
            )
        };
        let stdout = stdout_tee.lock().ok()?.snapshot_capture();
        let stderr = stderr_tee.lock().ok()?.snapshot_capture();
        Some((stdout, stderr, stdout_path, stderr_path))
    }

    /// Remove a job that never detached, once its readers are done. A finished
    /// call must not leave a row or a mailbox notice. Returns false while the
    /// id is missing, already detached, or still draining.
    pub fn discard_attached(&self, id: &str) -> bool {
        self.discard_inner(id, true)
    }

    /// Supervisor cancel path: the child is reaped and the readers are already
    /// joined, so the row can go even if the EOF flag lagged behind.
    pub fn discard_cancelled(&self, id: &str) -> bool {
        self.discard_inner(id, false)
    }

    fn discard_inner(&self, id: &str, require_sealed: bool) -> bool {
        let session_id = {
            let mut g = self.inner.lock().expect("jobs lock");
            let (detached, reader_eof) = match g.jobs.get(id) {
                Some(rec) => (rec.detached, rec.reader_eof),
                None => return false,
            };
            if detached || (require_sealed && !reader_eof) {
                return false;
            }
            let Some(rec) = g.jobs.remove(id) else {
                return false;
            };
            let session_id = rec.session_id;
            if let Some(queue) = g.mailbox.get_mut(&session_id) {
                queue.retain(|notice| notice.bash_id != id);
            }
            if g.mailbox
                .get(&session_id)
                .is_some_and(|queue| queue.is_empty())
            {
                g.mailbox.remove(&session_id);
            }
            g.generation += 1;
            session_id
        };
        self.wake();
        self.emit_jobs_changed(&session_id);
        true
    }

    pub fn running(&self, session_id: &str) -> Vec<RunningJob> {
        let g = self.inner.lock().expect("jobs lock");
        let mut out: Vec<RunningJob> = g
            .jobs
            .iter()
            .filter(|(_, rec)| rec.alive && rec.session_id == session_id)
            .map(|(id, rec)| RunningJob {
                id: id.clone(),
                command_preview: rec.command_preview.clone(),
                output_path: rec.output_path.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn get(&self, id: &str) -> Option<(bool, Option<u32>, String, PathBuf)> {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(id).map(|rec| {
            (
                rec.alive,
                rec.exit_code,
                rec.session_id.clone(),
                rec.output_path.clone(),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn job_pid(&self, id: &str) -> Option<u32> {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(id).and_then(|rec| rec.pid)
    }

    #[cfg(test)]
    pub(crate) fn is_detached(&self, id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(id).is_some_and(|rec| rec.detached)
    }

    /// Snapshot used by wait/kill/reminder formatting (same fields as an exit notice).
    pub fn notice_snapshot(&self, id: &str) -> Option<ExitNotice> {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(id).map(|rec| rec.to_notice(id))
    }

    pub fn finish(&self, id: &str, exit_code: Option<u32>) {
        self.finish_inner(id, exit_code, false);
    }

    pub fn finish_user_killed(&self, id: &str, exit_code: Option<u32>) {
        self.finish_inner(id, exit_code, true);
    }

    /// Supervisor only: both pipe readers hit EOF. This is not a second exit
    /// event. The notice and the exit handler already ran in [`Self::finish`];
    /// this only records that the log stopped growing and releases the
    /// kill-on-close job of an attached job that ended normally.
    pub fn finish_readers(&self, id: &str) {
        let session_id = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(rec) = g.jobs.get_mut(id) else {
                return;
            };
            if rec.reader_eof {
                return;
            }
            rec.reader_eof = true;
            if !rec.detached
                && let Some(tree) = &rec.tree
            {
                tree.release_on_close();
            }
            let session_id = rec.session_id.clone();
            g.generation += 1;
            session_id
        };
        self.wake();
        self.emit_jobs_changed(&session_id);
    }

    /// True when the job is unknown or has no pipe reader left.
    pub fn readers_eof(&self, id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(id).is_none_or(|rec| rec.reader_eof)
    }

    /// True while at least one pipe reader is still draining.
    pub fn readers_open(&self, id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(id).is_some_and(|rec| !rec.reader_eof)
    }

    /// Wait until both readers hit EOF. `timeout` bounds the wait and the
    /// return value is the state at the end.
    pub async fn wait_readers(&self, id: &str, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            if self.readers_eof(id) {
                return true;
            }
            notified.as_mut().enable();
            if self.readers_eof(id) {
                return true;
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep_until(deadline) => return self.readers_eof(id),
            }
        }
    }

    /// Undo the `tool_killed` latch after a kill that did not land, so a later
    /// natural exit is not reported as killed.
    pub fn clear_tool_kill_if_alive(&self, id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        if let Some(rec) = g.jobs.get_mut(id)
            && rec.alive
        {
            rec.tool_killed = false;
        }
    }

    /// Latch UI Kill before the process is signalled so a racing reaper still
    /// records `user_killed` on the mailbox notice.
    pub fn mark_user_kill(&self, id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        let session_id = {
            let Some(rec) = g.jobs.get_mut(id) else {
                return;
            };
            rec.user_killed = true;
            rec.session_id.clone()
        };
        if let Some(q) = g.mailbox.get_mut(&session_id) {
            for n in q.iter_mut() {
                if n.bash_id == id {
                    n.user_killed = true;
                }
            }
        }
    }

    /// Latch a Code-surface `tools.process.kill` so wait can distinguish it from a
    /// natural exit without setting `user_killed` (no human reminder).
    pub fn mark_tool_kill(&self, id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        let session_id = {
            let Some(rec) = g.jobs.get_mut(id) else {
                return;
            };
            rec.tool_killed = true;
            rec.session_id.clone()
        };
        if let Some(q) = g.mailbox.get_mut(&session_id) {
            for n in q.iter_mut() {
                if n.bash_id == id {
                    n.tool_killed = true;
                }
            }
        }
    }

    /// Supervisor only. Same step as sending `background` or `promoted`.
    fn mark_detached(&self, id: &str) {
        let session_id = {
            let mut g = self.inner.lock().expect("jobs lock");
            let session_id = {
                let Some(rec) = g.jobs.get_mut(id) else {
                    return;
                };
                rec.detached = true;
                rec.session_id.clone()
            };
            g.generation += 1;
            session_id
        };
        self.wake();
        self.emit_jobs_changed(&session_id);
    }

    fn finish_inner(&self, id: &str, exit_code: Option<u32>, user_killed: bool) {
        let settled = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(rec) = g.jobs.get_mut(id) else {
                return;
            };
            if !rec.alive {
                rec.exit_code = rec.exit_code.or(exit_code);
                rec.user_killed = rec.user_killed || user_killed;
                let session_id = rec.session_id.clone();
                let patch = rec.user_killed;
                let code = rec.exit_code;
                if patch && let Some(q) = g.mailbox.get_mut(&session_id) {
                    for n in q.iter_mut() {
                        if n.bash_id == id {
                            n.user_killed = true;
                            n.exit_code = code;
                        }
                    }
                }
                None
            } else {
                rec.alive = false;
                rec.exit_code = exit_code;
                rec.user_killed = rec.user_killed || user_killed;
                let notice = rec.to_notice(id);
                let detached = rec.detached;
                if detached {
                    g.mailbox
                        .entry(notice.session_id.clone())
                        .or_default()
                        .push_back(notice.clone());
                }
                g.generation += 1;
                Some((notice, detached))
            }
        };
        self.wake();
        let Some((notice, detached)) = settled else {
            return;
        };
        self.emit_jobs_changed(&notice.session_id);
        if detached
            && let Some(handler) = self.exit_handler.lock().expect("exit handler lock").clone()
        {
            handler(notice);
        }
    }

    pub fn begin_wait(
        &self,
        session_id: &str,
        call_id: &str,
        watching_id: Option<&str>,
        timeout: Option<Duration>,
    ) {
        if call_id.is_empty() {
            return;
        }
        let now = now_unix_ms();
        let deadline_ms = timeout.map(|d| now.saturating_add(d.as_millis() as i64));
        {
            let mut g = self.inner.lock().expect("jobs lock");
            g.waiters.insert(
                call_id.to_string(),
                Waiter {
                    session_id: session_id.to_string(),
                    call_id: call_id.to_string(),
                    watching_id: watching_id.map(str::to_string),
                    started_at_ms: now,
                    deadline_ms,
                },
            );
            g.generation += 1;
        }
        self.wake();
        self.emit_jobs_changed(session_id);
    }

    pub fn end_wait(&self, call_id: &str) {
        if call_id.is_empty() {
            return;
        }
        let session_id = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(waiter) = g.waiters.remove(call_id) else {
                return;
            };
            g.generation += 1;
            waiter.session_id
        };
        self.wake();
        self.emit_jobs_changed(&session_id);
    }

    pub fn wire_snapshot(&self, session_id: &str) -> BashJobsSnapshot {
        let g = self.inner.lock().expect("jobs lock");
        let mut jobs: Vec<BashJobWire> = g
            .jobs
            .iter()
            .filter(|(_, rec)| rec.alive && rec.session_id == session_id)
            .map(|(id, rec)| {
                let stderr_file = output_file_rel(&rec.stderr_path, &rec.workspace_root);
                BashJobWire {
                    id: id.clone(),
                    call_id: rec.call_id.clone(),
                    command_preview: rec.command_preview.clone(),
                    output_file: output_file_rel(&rec.output_path, &rec.workspace_root),
                    stderr_file: if stderr_file.is_empty() {
                        None
                    } else {
                        Some(stderr_file)
                    },
                    started_at_ms: rec.started_at_ms,
                }
            })
            .collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        let mut waits: Vec<BashWaitWire> = g
            .waiters
            .values()
            .filter(|w| w.session_id == session_id)
            .map(|w| BashWaitWire {
                call_id: w.call_id.clone(),
                watching_id: w.watching_id.clone(),
                started_at_ms: w.started_at_ms,
                deadline_ms: w.deadline_ms,
            })
            .collect();
        waits.sort_by(|a, b| a.call_id.cmp(&b.call_id));
        BashJobsSnapshot { jobs, waits }
    }

    pub fn tail_view(&self, id: &str) -> Option<BashTailView> {
        let g = self.inner.lock().expect("jobs lock");
        let rec = g.jobs.get(id)?;
        let cap = rec.tee.lock().ok()?.snapshot_capture();
        let text = if cap.frozen { cap.tail } else { cap.head };
        Some(BashTailView {
            text,
            truncated_on_disk: cap.truncated_on_disk,
            alive: rec.alive,
            draining: !rec.alive && !rec.reader_eof,
            exit_code: rec.exit_code,
        })
    }

    pub fn take_notice(&self, session_id: &str, bash_id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        Self::take_notice_from_mailbox(&mut g, session_id, bash_id);
    }

    /// Put notices back after a seam failed to persist them. Newer arrivals stay behind.
    pub fn restore_mailbox(&self, session_id: &str, notices: Vec<ExitNotice>) {
        if notices.is_empty() {
            return;
        }
        let mut g = self.inner.lock().expect("jobs lock");
        let queue = g.mailbox.entry(session_id.to_string()).or_default();
        for notice in notices.into_iter().rev() {
            if queue
                .iter()
                .any(|existing| existing.bash_id == notice.bash_id)
            {
                continue;
            }
            queue.push_front(notice);
        }
    }

    /// Drain unconsumed exits for `session_id` (next request-seam reminder).
    pub fn take_mailbox(&self, session_id: &str) -> Vec<ExitNotice> {
        let mut g = self.inner.lock().expect("jobs lock");
        g.mailbox
            .remove(session_id)
            .map(|d| d.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn mailbox_pending(&self, session_id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.mailbox.get(session_id).is_some_and(|q| !q.is_empty())
    }

    fn take_notice_from_mailbox(g: &mut JobState, session_id: &str, bash_id: &str) {
        if let Some(q) = g.mailbox.get_mut(session_id) {
            q.retain(|n| n.bash_id != bash_id);
            if q.is_empty() {
                g.mailbox.remove(session_id);
            }
        }
    }

    /// Async wait for one job. Does not block a runtime worker.
    /// The lock is dropped before every `.await`.
    pub async fn wait_async(
        &self,
        session_id: &str,
        job_id: &str,
        timeout: Option<Duration>,
        cancel: &CancellationToken,
    ) -> WaitOutcome {
        let deadline = timeout.map(|d| tokio::time::Instant::now() + d);
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            if let Some(outcome) = self.poll_wait(session_id, job_id, cancel, deadline) {
                return outcome;
            }
            notified.as_mut().enable();
            if let Some(outcome) = self.poll_wait(session_id, job_id, cancel, deadline) {
                return outcome;
            }
            tokio::select! {
                biased;
                _ = &mut notified => {}
                _ = cancel.cancelled() => return WaitOutcome::Cancelled,
                _ = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                }, if deadline.is_some() => return WaitOutcome::TimedOut,
            }
        }
    }

    fn poll_wait(
        &self,
        session_id: &str,
        job_id: &str,
        cancel: &CancellationToken,
        deadline: Option<tokio::time::Instant>,
    ) -> Option<WaitOutcome> {
        {
            let g = self.inner.lock().expect("jobs lock");
            match g.jobs.get(job_id) {
                None => return Some(WaitOutcome::UnknownId(job_id.to_string())),
                Some(rec) if rec.session_id != session_id => {
                    return Some(WaitOutcome::UnknownId(job_id.to_string()));
                }
                Some(rec) if !rec.alive => {
                    let notice = rec.to_notice(job_id);
                    drop(g);
                    let mut g = self.inner.lock().expect("jobs lock");
                    Self::take_notice_from_mailbox(&mut g, session_id, job_id);
                    return Some(WaitOutcome::Exited(notice));
                }
                Some(_) => {}
            }
        }
        if cancel.is_cancelled() {
            return Some(WaitOutcome::Cancelled);
        }
        if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
            return Some(WaitOutcome::TimedOut);
        }
        None
    }

    /// Signal a pipe job's whole tree. Signalling never seals the record: the
    /// supervisor still owns the exit, so exactly one notice is written and
    /// `wait` cannot see `killed` before the process is actually gone.
    ///
    /// - running: latch the kill flags, then terminate the tree.
    /// - draining: the shell already exited; terminate whatever is left so the
    ///   pipe closes and the readers can finish.
    /// - sealed: nothing left to signal.
    ///
    /// Human PTY ids are not in this registry and return `None`.
    pub fn kill_registered(&self, id: &str, user_killed: bool) -> Option<KilledJob> {
        let (alive, reader_eof, tree, pid, output_path) = {
            let g = self.inner.lock().expect("jobs lock");
            let rec = g.jobs.get(id)?;
            (
                rec.alive,
                rec.reader_eof,
                rec.tree.clone(),
                rec.pid,
                rec.output_path.clone(),
            )
        };
        if alive {
            if user_killed {
                self.mark_user_kill(id);
            } else {
                self.mark_tool_kill(id);
            }
        }
        if alive || !reader_eof {
            match &tree {
                Some(tree) => tree.kill_tree(),
                None => {
                    if let Some(pid) = pid {
                        kill_process_tree(pid);
                    }
                }
            }
        }
        let (alive, exit_code) = self
            .get(id)
            .map(|(alive, code, _, _)| (alive, code))
            .unwrap_or((false, None));
        Some(KilledJob {
            id: id.to_string(),
            alive,
            reader_eof: self.readers_eof(id),
            exit_code,
            output_path: Some(output_path),
        })
    }

    /// A pipe job the tool has already returned, still running. Born detached.
    /// Live commands use [`supervise_pipe`] instead of this helper.
    pub fn insert_live_pipe_job(
        &self,
        id: &str,
        session_id: &str,
        call_id: &str,
        command: &str,
        workspace_root: &Path,
    ) -> Result<PathBuf, String> {
        if !job_id_is_file_safe(id) {
            return Err(format!("job id cannot be a file name: {id}"));
        }
        let (stdout_path, stderr_path) = pipe_log_paths(workspace_root, id);
        std::fs::create_dir_all(pipe_log_dir(workspace_root)).map_err(|e| e.to_string())?;
        let stdout = BoundedTee::create(stdout_path.clone()).map_err(|e| e.to_string())?;
        let stderr = BoundedTee::create(stderr_path.clone()).map_err(|e| e.to_string())?;
        self.insert(
            id.to_string(),
            AgentJobRecord {
                session_id: session_id.to_string(),
                call_id: call_id.to_string(),
                command_preview: command_preview(command),
                output_path: stdout_path.clone(),
                tee: Arc::new(Mutex::new(stdout)),
                alive: true,
                // No supervisor owns readers for a fixture insert.
                reader_eof: true,
                exit_code: None,
                started_at_ms: now_unix_ms(),
                user_killed: false,
                tool_killed: false,
                workspace_root: workspace_root.to_path_buf(),
                stderr_path,
                stderr_tee: Arc::new(Mutex::new(stderr)),
                pid: None,
                tree: None,
                detached: true,
            },
        );
        Ok(stdout_path)
    }

    /// A pipe job that already returned and then exited. `user_killed` goes
    /// through [`Self::finish_user_killed`] so the mailbox notice is marked.
    pub fn insert_finished_pipe_job(
        &self,
        id: &str,
        session_id: &str,
        call_id: &str,
        command: &str,
        workspace_root: &Path,
        exit_code: Option<u32>,
        user_killed: bool,
    ) -> Result<(), String> {
        self.insert_live_pipe_job(id, session_id, call_id, command, workspace_root)?;
        if user_killed {
            self.mark_user_kill(id);
            self.finish_user_killed(id, exit_code);
        } else {
            self.finish(id, exit_code);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum WaitOutcome {
    Exited(ExitNotice),
    TimedOut,
    Cancelled,
    UnknownId(String),
}

/// Default foreground window for `run`: a command that exits within it returns
/// its `exit_code` directly; one still alive afterwards detaches with
/// `exit_code: null` and stays a `wait` / `kill` target. `run` may shorten it
/// to the enclosing `run_code` budget.
pub const RUN_FOREGROUND_WINDOW: Duration = Duration::from_secs(30);

/// How the supervisor waits before it either returns the exit or detaches.
#[derive(Debug, Clone, Copy)]
pub enum PipeDetach {
    /// Stay attached until exit, nested cancel, or `timeout`.
    /// A timeout while the process is still alive promotes; it does not kill.
    Foreground { timeout: Duration },
    /// If the process is still alive after `window`, detach.
    /// The exit arm stays selected during that window, so a command that has
    /// already finished is `Exited` and is not detached.
    Background { window: Duration },
}

/// One oneshot for the tool future. The supervisor chooses this in the same
/// step as `detached`.
#[derive(Debug)]
pub enum PipeDecision {
    /// Process exited before detach. Not mailed.
    Exited {
        job_id: String,
        exit_code: Option<i32>,
    },
    Background {
        job_id: String,
    },
    Promoted {
        job_id: String,
    },
    Cancelled {
        job_id: String,
    },
    Failed {
        message: String,
    },
}

pub struct PipeRequest {
    pub command: String,
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub session_id: String,
    pub call_id: String,
    /// Selected only while the job is not detached. Dropped in the same step
    /// that sets `detached`. Never given to the stdout or stderr readers.
    pub cancel: CancellationToken,
    pub detach: PipeDetach,
    /// Fired once `spawn()` succeeded. Dropped unsent on every early return
    /// (cancel before spawn, spawn failure), so the receiver always resolves.
    pub on_spawned: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Own a piped child on the calling tokio runtime.
///
/// The tool future waits on the returned oneshot and nothing else. Readers are
/// spawned here and run until EOF; they have no cancel token, so detach cannot
/// stop them and fill the pipe. `detached` is set only in this task, in the
/// same step as `Background` or `Promoted`. An exit before that step is
/// `Exited`: no detach, no mailbox, no exit handler.
pub fn supervise_pipe(
    jobs: Arc<AgentJobRegistry>,
    request: PipeRequest,
) -> tokio::sync::oneshot::Receiver<PipeDecision> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        run_pipe_supervisor(jobs, request, tx).await;
    });
    rx
}

async fn run_pipe_supervisor(
    jobs: Arc<AgentJobRegistry>,
    mut request: PipeRequest,
    tx: tokio::sync::oneshot::Sender<PipeDecision>,
) {
    let on_spawned = request.on_spawned.take();
    if request.cancel.is_cancelled() {
        let _ = tx.send(PipeDecision::Cancelled {
            job_id: String::new(),
        });
        return;
    }

    let id = alloc_pipe_job_id(&jobs, &request.workspace_root);
    let spec = super::shell::shell_command(&request.command);
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in super::pty::AGENT_NON_INTERACTIVE_ENV {
        cmd.env(*key, *value);
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    // Windows: start suspended so the per-job Job Object can be attached before
    // the shell runs a single instruction. That closes the window where the
    // shell could fork a child that escapes the job.
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
        cmd.creation_flags(CREATE_SUSPENDED);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            drop(on_spawned);
            let _ = tx.send(PipeDecision::Failed {
                message: format!("failed to spawn: {e}"),
            });
            return;
        }
    };
    if let Some(spawned) = on_spawned {
        let _ = spawned.send(());
    }
    let pid = child.id().filter(|pid| *pid != 0);
    // Kill scope for the whole tree. Windows attaches a per-job Job Object to
    // the suspended shell; when that is unavailable the child still resumes and
    // tree kill falls back to `taskkill /T /F`.
    #[cfg(windows)]
    let tree = {
        use windows_sys::Win32::Foundation::HANDLE;
        let job = child
            .raw_handle()
            .and_then(|raw| win_job::attach(raw as HANDLE));
        if job.is_none() {
            tracing::warn!(
                pid,
                "per-job Job Object unavailable; tree kill falls back to taskkill"
            );
        }
        if !win_job::resume_main_thread(pid.unwrap_or(0)).await {
            if let Some(job) = job {
                win_job::terminate(job);
            }
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = tx.send(PipeDecision::Failed {
                message: "failed to resume the suspended shell".into(),
            });
            return;
        }
        job.map(|job| {
            Arc::new(JobTree {
                pid: pid.unwrap_or(0),
                job: Some(job),
            })
        })
    };
    #[cfg(not(windows))]
    let tree: Option<Arc<JobTree>> = None;

    if tree.is_none() {
        crate::proc_lifetime::bind_child_to_parent(pid.unwrap_or(0));
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (stdout_path, stderr_path) = pipe_log_paths(&request.workspace_root, &id);
    if let Err(e) = std::fs::create_dir_all(pipe_log_dir(&request.workspace_root)) {
        abandon_pipe_child(&mut child, tree.as_ref(), pid).await;
        let _ = tx.send(PipeDecision::Failed {
            message: format!("failed to create output dir: {e}"),
        });
        return;
    }
    let stdout_tee = match BoundedTee::create(stdout_path.clone()) {
        Ok(tee) => Arc::new(Mutex::new(tee)),
        Err(e) => {
            abandon_pipe_child(&mut child, tree.as_ref(), pid).await;
            let _ = tx.send(PipeDecision::Failed {
                message: format!("failed to create stdout log: {e}"),
            });
            return;
        }
    };
    let stderr_tee = match BoundedTee::create(stderr_path.clone()) {
        Ok(tee) => Arc::new(Mutex::new(tee)),
        Err(e) => {
            abandon_pipe_child(&mut child, tree.as_ref(), pid).await;
            let _ = tx.send(PipeDecision::Failed {
                message: format!("failed to create stderr log: {e}"),
            });
            return;
        }
    };

    let session_id = if request.session_id.is_empty() {
        "_".to_string()
    } else {
        request.session_id.clone()
    };
    jobs.insert(
        id.clone(),
        AgentJobRecord {
            session_id,
            call_id: request.call_id.clone(),
            command_preview: command_preview(&request.command),
            output_path: stdout_path,
            tee: Arc::clone(&stdout_tee),
            alive: true,
            reader_eof: false,
            exit_code: None,
            started_at_ms: now_unix_ms(),
            user_killed: false,
            tool_killed: false,
            workspace_root: request.workspace_root.clone(),
            stderr_path,
            stderr_tee: Arc::clone(&stderr_tee),
            pid,
            tree,
            detached: false,
        },
    );

    // No cancel token: these tasks read until the pipe hits EOF.
    let mut stdout_task = stdout.map(|reader| spawn_pipe_reader(reader, stdout_tee));
    let mut stderr_task = stderr.map(|reader| spawn_pipe_reader(reader, stderr_tee));

    let mut nested = Some(request.cancel);
    let mut detached = false;
    let mut decision = Some(tx);
    let detach_mode = request.detach;
    let timeout_at = match detach_mode {
        PipeDetach::Foreground { timeout } => Some(tokio::time::Instant::now() + timeout),
        PipeDetach::Background { .. } => None,
    };
    let background_window = match detach_mode {
        PipeDetach::Background { window } => Some(window),
        PipeDetach::Foreground { .. } => None,
    };

    loop {
        let cancel_token = nested.clone();
        tokio::select! {
            biased;
            status = child.wait() => {
                let code = status.ok().and_then(|s| s.code());
                // Process exit is the lifecycle fact. Readers may still be
                // draining a pipe a descendant kept open, so they finish after
                // the exit is recorded and the tool is released.
                jobs.finish(&id, code.map(|c| c as u32));
                if !detached {
                    send_decision(
                        &mut decision,
                        PipeDecision::Exited {
                            job_id: id.clone(),
                            exit_code: code,
                        },
                    );
                }
                join_reader(&mut stdout_task).await;
                join_reader(&mut stderr_task).await;
                jobs.finish_readers(&id);
                break;
            }
            _ = async {
                match cancel_token {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            }, if !detached => {
                if let Some(pid) = pid {
                    kill_process_tree(pid);
                }
                let _ = child.start_kill();
                let status = child.wait().await.ok();
                let code = status.and_then(|s| s.code());
                jobs.finish(&id, code.map(|c| c as u32));
                join_reader(&mut stdout_task).await;
                join_reader(&mut stderr_task).await;
                jobs.finish_readers(&id);
                send_decision(
                    &mut decision,
                    PipeDecision::Cancelled { job_id: id.clone() },
                );
                break;
            }
            _ = async {
                match timeout_at {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            }, if !detached && timeout_at.is_some() => {
                if settle_detach_or_exit(
                    &jobs,
                    &mut child,
                    &id,
                    &mut stdout_task,
                    &mut stderr_task,
                    &mut nested,
                    &mut detached,
                    &mut decision,
                    PipeDecision::Promoted { job_id: id.clone() },
                )
                .await
                {
                    break;
                }
            }
            _ = tokio::time::sleep(background_window.unwrap_or_default()), if !detached && background_window.is_some() => {
                if settle_detach_or_exit(
                    &jobs,
                    &mut child,
                    &id,
                    &mut stdout_task,
                    &mut stderr_task,
                    &mut nested,
                    &mut detached,
                    &mut decision,
                    PipeDecision::Background { job_id: id.clone() },
                )
                .await
                {
                    break;
                }
            }
        }
    }
}

/// Returns true when the process had already exited and the oneshot is `Exited`.
async fn settle_detach_or_exit(
    jobs: &AgentJobRegistry,
    child: &mut tokio::process::Child,
    id: &str,
    stdout_task: &mut Option<tokio::task::JoinHandle<()>>,
    stderr_task: &mut Option<tokio::task::JoinHandle<()>>,
    nested: &mut Option<CancellationToken>,
    detached: &mut bool,
    decision: &mut Option<tokio::sync::oneshot::Sender<PipeDecision>>,
    detach_decision: PipeDecision,
) -> bool {
    match child.try_wait() {
        Ok(Some(status)) => {
            let code = status.code();
            jobs.finish(id, code.map(|c| c as u32));
            send_decision(
                decision,
                PipeDecision::Exited {
                    job_id: id.to_string(),
                    exit_code: code,
                },
            );
            join_reader(stdout_task).await;
            join_reader(stderr_task).await;
            jobs.finish_readers(id);
            true
        }
        Err(e) => {
            jobs.finish(id, None);
            send_decision(
                decision,
                PipeDecision::Failed {
                    message: format!("failed to poll child: {e}"),
                },
            );
            join_reader(stdout_task).await;
            join_reader(stderr_task).await;
            jobs.finish_readers(id);
            true
        }
        Ok(None) => {
            jobs.mark_detached(id);
            *detached = true;
            *nested = None;
            send_decision(decision, detach_decision);
            false
        }
    }
}

fn send_decision(
    slot: &mut Option<tokio::sync::oneshot::Sender<PipeDecision>>,
    decision: PipeDecision,
) {
    if let Some(tx) = slot.take() {
        let _ = tx.send(decision);
    }
}

/// Kill a child that failed before its readers were spawned.
async fn abandon_pipe_child(
    child: &mut tokio::process::Child,
    tree: Option<&Arc<JobTree>>,
    pid: Option<u32>,
) {
    match tree {
        Some(tree) => tree.kill_tree(),
        None => {
            if let Some(pid) = pid {
                kill_process_tree(pid);
            }
        }
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

async fn join_reader(task: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(task) = task.take() {
        let _ = task.await;
    }
}

/// Read until EOF. No cancel token: after detach, stopping here fills the pipe
/// and the child blocks in `write`, so the exit mailbox never arrives.
fn spawn_pipe_reader(
    mut reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    tee: Arc<Mutex<BoundedTee>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 8192];
        // WP-210 F: UTF-8 first, then the ANSI code page (CP936 …); split
        // characters wait for the next read instead of becoming U+FFFD.
        let mut decoder = super::decode::StreamDecoder::platform();
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = decoder.push(&buf[..n]);
                    if !text.is_empty()
                        && let Ok(mut tee) = tee.lock()
                    {
                        tee.push_raw(&text);
                    }
                }
            }
        }
        let rest = decoder.finish();
        if !rest.is_empty()
            && let Ok(mut tee) = tee.lock()
        {
            tee.push_raw(&rest);
        }
    })
}

/// Windows per-job Job Object support.
///
/// A job created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` gives one kill scope
/// per command: `TerminateJobObject` reaches every descendant even after the
/// shell itself exited, and a runtime crash still reaps the tree when the last
/// handle closes. The shell is spawned suspended, assigned to the job, and only
/// then resumed, so no descendant can be created before the assignment.
#[cfg(windows)]
mod win_job {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    /// Create a kill-on-close job and assign `process` to it. Returns the raw
    /// handle as `usize` (a HANDLE is not `Sync`) or `None` when the OS
    /// refuses, e.g. this process already sits in a job that forbids nesting.
    pub(super) fn attach(process: HANDLE) -> Option<usize> {
        if process.is_null() {
            return None;
        }
        // SAFETY: plain Win32 calls; the job handle is closed on every failure.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let sized = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                sized,
            );
            if set == 0 {
                CloseHandle(job);
                return None;
            }
            if AssignProcessToJobObject(job, process) == 0 {
                CloseHandle(job);
                return None;
            }
            Some(job as usize)
        }
    }

    /// Resume the only thread of a process spawned with `CREATE_SUSPENDED`. The
    /// thread is in the snapshot immediately; the short retry only covers
    /// snapshot raciness.
    pub(super) async fn resume_main_thread(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        for _ in 0..100 {
            if let Some(tid) = main_thread_id(pid) {
                // SAFETY: the thread id was observed for this pid a moment ago.
                unsafe {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, tid);
                    if !thread.is_null() {
                        let _ = ResumeThread(thread);
                        CloseHandle(thread);
                        return true;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        false
    }

    pub(super) fn terminate(job: usize) {
        // SAFETY: the caller owns the handle for as long as the job lives.
        unsafe {
            TerminateJobObject(job as HANDLE, 1);
        }
    }

    fn main_thread_id(pid: u32) -> Option<u32> {
        // SAFETY: the snapshot handle is closed below; the entry is sized first.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
                return None;
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut found = None;
            let mut more = Thread32First(snapshot, &mut entry);
            while more != 0 {
                if entry.th32OwnerProcessID == pid {
                    found = Some(entry.th32ThreadID);
                    break;
                }
                more = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            found
        }
    }
}

pub fn command_preview(command: &str) -> String {
    let collapsed: String = command.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX: usize = 80;
    if collapsed.chars().count() <= MAX {
        collapsed
    } else {
        let kept: String = collapsed.chars().take(MAX).collect();
        format!("{kept}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn rec(
        session: &str,
        call: &str,
        workspace: &Path,
        path: PathBuf,
        tee: BoundedTee,
    ) -> AgentJobRecord {
        let stderr_path = path.with_extension("stderr");
        let stderr = BoundedTee::create(stderr_path.clone()).unwrap();
        AgentJobRecord {
            session_id: session.into(),
            call_id: call.into(),
            command_preview: "sleep".into(),
            output_path: path,
            tee: Arc::new(Mutex::new(tee)),
            alive: true,
            reader_eof: true,
            exit_code: None,
            started_at_ms: now_unix_ms(),
            user_killed: false,
            tool_killed: false,
            workspace_root: workspace.to_path_buf(),
            stderr_path,
            stderr_tee: Arc::new(Mutex::new(stderr)),
            pid: None,
            tree: None,
            detached: false,
        }
    }

    fn sleep_secs(seconds: u64) -> String {
        #[cfg(windows)]
        {
            if crate::config::git_install::find_git_bash().is_some() {
                format!("sleep {seconds}")
            } else {
                format!("Start-Sleep -Seconds {seconds}")
            }
        }
        #[cfg(not(windows))]
        {
            format!("sleep {seconds}")
        }
    }

    fn delayed_blob() -> String {
        #[cfg(windows)]
        {
            if crate::config::git_install::find_git_bash().is_some() {
                "sleep 2; head -c 262144 /dev/zero".into()
            } else {
                "Start-Sleep -Seconds 2; Write-Output -NoNewline ('x' * 262144)".into()
            }
        }
        #[cfg(not(windows))]
        {
            "sleep 2; head -c 262144 /dev/zero".into()
        }
    }

    fn pipe_request(
        dir: &Path,
        command: &str,
        call_id: &str,
        cancel: CancellationToken,
        detach: PipeDetach,
    ) -> PipeRequest {
        PipeRequest {
            command: command.to_string(),
            cwd: dir.to_path_buf(),
            workspace_root: dir.to_path_buf(),
            session_id: "s1".into(),
            call_id: call_id.into(),
            cancel,
            detach,
            on_spawned: None,
        }
    }

    async fn recv_decision(rx: tokio::sync::oneshot::Receiver<PipeDecision>) -> PipeDecision {
        tokio::time::timeout(Duration::from_secs(15), rx)
            .await
            .expect("supervisor decision timed out")
            .expect("supervisor dropped the decision")
    }

    fn pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        #[cfg(unix)]
        {
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            };
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if handle.is_null() {
                    return false;
                }
                CloseHandle(handle);
                true
            }
        }
    }

    #[tokio::test]
    async fn wait_async_times_out_while_job_stays_alive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        reg.insert(
            "bg_timeout1".into(),
            rec("s1", "call_a", dir.path(), path, tee),
        );
        let cancel = CancellationToken::new();
        match reg
            .wait_async(
                "s1",
                "bg_timeout1",
                Some(Duration::from_millis(40)),
                &cancel,
            )
            .await
        {
            WaitOutcome::TimedOut => {}
            other => panic!("{other:?}"),
        }
        assert!(reg.get("bg_timeout1").unwrap().0);
    }

    #[test]
    fn undetached_finish_skips_mailbox_and_exit_handler() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_cb = Arc::clone(&hits);
        reg.set_exit_handler(Arc::new(move |_| {
            hits_cb.fetch_add(1, Ordering::SeqCst);
        }));
        reg.insert(
            "bg_attached".into(),
            rec("s1", "call_a", dir.path(), path, tee),
        );
        reg.finish("bg_attached", Some(0));
        assert!(!reg.is_detached("bg_attached"));
        assert!(reg.take_mailbox("s1").is_empty());
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        let path2 = dir.path().join("k.output");
        let tee2 = BoundedTee::create(path2.clone()).unwrap();
        let mut returned = rec("s1", "call_b", dir.path(), path2, tee2);
        returned.detached = true;
        reg.insert("bg_returned".into(), returned);
        reg.finish("bg_returned", Some(0));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let mail = reg.take_mailbox("s1");
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].bash_id, "bg_returned");
    }

    #[tokio::test]
    async fn foreground_timeout_promotes_without_killing() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let rx = supervise_pipe(
            Arc::clone(&jobs),
            pipe_request(
                dir.path(),
                &sleep_secs(5),
                "call_promote",
                CancellationToken::new(),
                PipeDetach::Foreground {
                    timeout: Duration::from_millis(400),
                },
            ),
        );
        let decision = recv_decision(rx).await;
        let PipeDecision::Promoted { job_id } = decision else {
            panic!("expected promoted, got {decision:?}");
        };
        let pid = jobs.job_pid(&job_id).expect("pid");
        assert!(pid_alive(pid), "promote must leave the process running");
        assert!(jobs.is_detached(&job_id));
        assert!(!jobs.mailbox_pending("s1"));
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while !jobs.mailbox_pending("s1") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert!(
            jobs.mailbox_pending("s1"),
            "promoted exit must reach the mailbox"
        );
        let mail = jobs.take_mailbox("s1");
        assert_eq!(mail.len(), 1);
        assert!(!mail[0].user_killed);
    }

    #[tokio::test]
    async fn foreground_cancel_kills_and_skips_mailbox() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let cancel = CancellationToken::new();
        let cancel2 = cancel.clone();
        let rx = supervise_pipe(
            Arc::clone(&jobs),
            pipe_request(
                dir.path(),
                &sleep_secs(30),
                "call_cancel",
                cancel2,
                PipeDetach::Foreground {
                    timeout: Duration::from_secs(30),
                },
            ),
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel.cancel();
        let decision = recv_decision(rx).await;
        let PipeDecision::Cancelled { job_id } = decision else {
            panic!("expected cancelled, got {decision:?}");
        };
        assert!(!job_id.is_empty());
        assert!(jobs.take_mailbox("s1").is_empty());
        if let Some(pid) = jobs.job_pid(&job_id) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while pid_alive(pid) && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(!pid_alive(pid), "cancel before detach must kill the tree");
        }
    }

    #[tokio::test]
    async fn detached_read_continues_after_nested_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let cancel = CancellationToken::new();
        let cancel2 = cancel.clone();
        let rx = supervise_pipe(
            Arc::clone(&jobs),
            pipe_request(
                dir.path(),
                &delayed_blob(),
                "parent:code:9",
                cancel2,
                PipeDetach::Background {
                    window: Duration::from_millis(500),
                },
            ),
        );
        let decision = recv_decision(rx).await;
        let PipeDecision::Background { job_id } = decision else {
            panic!("expected background, got {decision:?}");
        };
        let (_, _, _, path) = jobs.get(&job_id).unwrap();
        cancel.cancel();
        let early = std::fs::read(&path).unwrap_or_default();
        assert!(
            early.len() < 4096,
            "blob must not have started before nested cancel, got {} bytes",
            early.len()
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !jobs.mailbox_pending("s1") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert!(
            jobs.mailbox_pending("s1"),
            "readers must keep draining after nested cancel"
        );
        // The notice now fires at process exit, before EOF: the log is only
        // complete once the readers are done.
        while !jobs.readers_eof(&job_id) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(jobs.readers_eof(&job_id), "readers must reach EOF");
        let late = std::fs::read(&path).unwrap_or_default();
        assert!(
            late.len() > 200_000,
            "stdout after detach was only {} bytes",
            late.len()
        );
        assert!(!path.file_name().unwrap().to_string_lossy().contains(':'));
    }

    #[tokio::test]
    async fn kill_registered_kills_pid_and_marks_user_killed() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let rx = supervise_pipe(
            Arc::clone(&jobs),
            pipe_request(
                dir.path(),
                &sleep_secs(30),
                "call_kill",
                CancellationToken::new(),
                PipeDetach::Background {
                    window: Duration::from_millis(500),
                },
            ),
        );
        let decision = recv_decision(rx).await;
        let PipeDecision::Background { job_id } = decision else {
            panic!("expected background, got {decision:?}");
        };
        let pid = jobs.job_pid(&job_id).expect("pid");
        assert!(pid_alive(pid), "the job must be running before the signal");
        // Signalling does not seal the record: the supervisor still owns the
        // exit, so the notice cannot say "killed" before the process is gone.
        // The registry state, not a recycled pid, is the oracle for that.
        let _killed = jobs.kill_registered(&job_id, true).expect("registered job");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while jobs.get(&job_id).is_some_and(|(alive, ..)| alive)
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !jobs.get(&job_id).expect("record").0,
            "the supervisor must reap the killed job"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !jobs.mailbox_pending("s1") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            jobs.mailbox_pending("s1"),
            "a killed detached job must still reach the mailbox"
        );
        assert!(
            !jobs.get(&job_id).expect("record").0,
            "the supervisor must record the exit"
        );
        let mail = jobs.take_mailbox("s1");
        assert_eq!(mail.len(), 1);
        assert!(mail[0].user_killed);
        assert_eq!(mail[0].bash_id, job_id);
        assert!(jobs.kill_registered("term_not-a-pipe-job", true).is_none());
    }

    #[tokio::test]
    async fn process_exit_is_not_reader_eof() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        let mut record = rec("s1", "call_a", dir.path(), path, tee);
        // A live supervisor owns the readers: they have not hit EOF yet.
        record.reader_eof = false;
        reg.insert("bg_drain1".into(), record);

        assert_eq!(reg.running("s1").len(), 1);
        reg.finish("bg_drain1", Some(0));

        assert!(
            reg.running("s1").is_empty(),
            "an exited job must leave the running list at once"
        );
        assert!(reg.readers_open("bg_drain1"), "the pipe is still draining");
        assert!(
            !reg.discard_attached("bg_drain1"),
            "a draining job keeps its record and logs"
        );
        assert!(!reg.mailbox_pending("s1"), "attached exits are not mailed");

        let cancel = CancellationToken::new();
        match reg
            .wait_async("s1", "bg_drain1", Some(Duration::from_millis(200)), &cancel)
            .await
        {
            WaitOutcome::Exited(notice) => assert_eq!(notice.exit_code, Some(0)),
            other => panic!("expected an exit, got {other:?}"),
        }

        reg.finish_readers("bg_drain1");
        assert!(reg.readers_eof("bg_drain1"));
        assert!(reg.discard_attached("bg_drain1"));
        assert!(!reg.mailbox_pending("s1"));
    }

    #[test]
    fn second_finish_does_not_mail_twice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        let mut record = rec("s1", "call_a", dir.path(), path, tee);
        record.detached = true;
        reg.insert("bg_twice".into(), record);

        reg.finish("bg_twice", Some(3));
        reg.finish("bg_twice", Some(3));
        let mail = reg.take_mailbox("s1");
        assert_eq!(mail.len(), 1, "one exit, one notice");
        assert_eq!(mail[0].exit_code, Some(3));
    }
}
