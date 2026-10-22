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
}

#[derive(Debug, Clone)]
pub struct RunningJob {
    pub id: String,
    pub command_preview: String,
    pub output_path: PathBuf,
}

pub struct AgentJobRecord {
    pub session_id: String,
    pub call_id: String,
    pub command_preview: String,
    pub output_path: PathBuf,
    pub tee: Arc<Mutex<BoundedTee>>,
    pub alive: bool,
    pub exit_code: Option<u32>,
    pub started_at_ms: i64,
    pub user_killed: bool,
    workspace_root: PathBuf,
    /// Stderr log. `bash/tail` reads stdout; the tool result reads both.
    stderr_path: PathBuf,
    stderr_tee: Arc<Mutex<BoundedTee>>,
    pid: Option<u32>,
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
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct KilledJob {
    pub id: String,
    pub alive: bool,
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

    /// Remove a job that never detached. A cancelled call must not leave a row
    /// or a mailbox notice. Returns false when the id is missing or already detached.
    pub fn discard_attached(&self, id: &str) -> bool {
        let session_id = {
            let mut g = self.inner.lock().expect("jobs lock");
            let detached = match g.jobs.get(id) {
                Some(rec) => rec.detached,
                None => return false,
            };
            if detached {
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

    #[cfg(test)]
    pub(crate) fn session_has_job(&self, session_id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.values().any(|rec| rec.session_id == session_id)
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

    /// Kill a pipe job by pid and seal it. Human PTY ids are not in this
    /// registry and return `None`. When `user_killed` is set, a detached job's
    /// mailbox notice stays for `reminder/bash_exit`.
    pub fn kill_registered(&self, id: &str, user_killed: bool) -> Option<KilledJob> {
        let (pid, output_path) = {
            let g = self.inner.lock().expect("jobs lock");
            let rec = g.jobs.get(id)?;
            (rec.pid, rec.output_path.clone())
        };
        if user_killed {
            self.mark_user_kill(id);
        }
        if let Some(pid) = pid {
            kill_process_tree(pid);
        }
        let exit_code = self.get(id).and_then(|(_, code, _, _)| code);
        if user_killed {
            self.finish_user_killed(id, exit_code);
        } else {
            self.finish(id, exit_code);
        }
        let (alive, exit_code) = self
            .get(id)
            .map(|(alive, code, _, _)| (alive, code))
            .unwrap_or((false, exit_code));
        Some(KilledJob {
            id: id.to_string(),
            alive,
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
                exit_code: None,
                started_at_ms: now_unix_ms(),
                user_killed: false,
                workspace_root: workspace_root.to_path_buf(),
                stderr_path,
                stderr_tee: Arc::new(Mutex::new(stderr)),
                pid: None,
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
    crate::proc_lifetime::bind_child_to_parent(pid.unwrap_or(0));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (stdout_path, stderr_path) = pipe_log_paths(&request.workspace_root, &id);
    if let Err(e) = std::fs::create_dir_all(pipe_log_dir(&request.workspace_root)) {
        if let Some(pid) = pid {
            kill_process_tree(pid);
        }
        let _ = child.start_kill();
        let _ = tx.send(PipeDecision::Failed {
            message: format!("failed to create output dir: {e}"),
        });
        return;
    }
    let stdout_tee = match BoundedTee::create(stdout_path.clone()) {
        Ok(tee) => Arc::new(Mutex::new(tee)),
        Err(e) => {
            if let Some(pid) = pid {
                kill_process_tree(pid);
            }
            let _ = child.start_kill();
            let _ = tx.send(PipeDecision::Failed {
                message: format!("failed to create stdout log: {e}"),
            });
            return;
        }
    };
    let stderr_tee = match BoundedTee::create(stderr_path.clone()) {
        Ok(tee) => Arc::new(Mutex::new(tee)),
        Err(e) => {
            if let Some(pid) = pid {
                kill_process_tree(pid);
            }
            let _ = child.start_kill();
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
            exit_code: None,
            started_at_ms: now_unix_ms(),
            user_killed: false,
            workspace_root: request.workspace_root.clone(),
            stderr_path,
            stderr_tee: Arc::clone(&stderr_tee),
            pid,
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
                join_reader(&mut stdout_task).await;
                join_reader(&mut stderr_task).await;
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
                join_reader(&mut stdout_task).await;
                join_reader(&mut stderr_task).await;
                jobs.finish(&id, code.map(|c| c as u32));
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
            join_reader(stdout_task).await;
            join_reader(stderr_task).await;
            jobs.finish(id, code.map(|c| c as u32));
            send_decision(
                decision,
                PipeDecision::Exited {
                    job_id: id.to_string(),
                    exit_code: code,
                },
            );
            true
        }
        Err(e) => {
            join_reader(stdout_task).await;
            join_reader(stderr_task).await;
            jobs.finish(id, None);
            send_decision(
                decision,
                PipeDecision::Failed {
                    message: format!("failed to poll child: {e}"),
                },
            );
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
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if let Ok(mut tee) = tee.lock() {
                        tee.push_raw(&text);
                    }
                }
            }
        }
    })
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
        let late = std::fs::read(&path).unwrap_or_default();
        assert!(
            late.len() > 200_000,
            "stdout after detach was only {} bytes",
            late.len()
        );
        assert!(!path.file_name().unwrap().to_string_lossy().contains(':'));
    }

}
