//! Session-scoped agent bash jobs: running list, wait, and exit mailbox.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::tee::{BoundedTee, TeeCapture};

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn output_file_rel(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(".litecode/bash/{name}")
}

#[derive(Debug, Clone)]
pub struct ExitNotice {
    pub bash_id: String,
    pub session_id: String,
    pub exit_code: Option<u32>,
    pub output_path: PathBuf,
    pub command_preview: String,
    /// True when the human stopped the job from the bash card (not a natural exit
    /// and not `kill_shell`).
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

struct JobState {
    jobs: HashMap<String, AgentJobRecord>,
    mailbox: HashMap<String, VecDeque<ExitNotice>>,
    waiters: HashMap<String, Waiter>,
    generation: u64,
}

pub struct AgentJobRegistry {
    inner: Mutex<JobState>,
    cv: Condvar,
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
            cv: Condvar::new(),
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

    pub fn insert(&self, id: String, record: AgentJobRecord) {
        let session_id = record.session_id.clone();
        {
            let mut g = self.inner.lock().expect("jobs lock");
            g.jobs.insert(id, record);
            g.generation += 1;
            self.cv.notify_all();
        }
        self.emit_jobs_changed(&session_id);
    }

    pub fn snapshot_capture(&self, id: &str) -> Option<TeeCapture> {
        let g = self.inner.lock().expect("jobs lock");
        let rec = g.jobs.get(id)?;
        rec.tee.lock().ok().map(|t| t.snapshot_capture())
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

    fn finish_inner(&self, id: &str, exit_code: Option<u32>, user_killed: bool) {
        let notice = {
            let mut g = self.inner.lock().expect("jobs lock");
            let step = {
                let Some(rec) = g.jobs.get_mut(id) else {
                    return;
                };
                if !rec.alive {
                    rec.exit_code = rec.exit_code.or(exit_code);
                    rec.user_killed = rec.user_killed || user_killed;
                    Err((
                        rec.session_id.clone(),
                        rec.user_killed,
                        rec.exit_code,
                    ))
                } else {
                    rec.alive = false;
                    rec.exit_code = exit_code;
                    rec.user_killed = rec.user_killed || user_killed;
                    Ok(rec.to_notice(id))
                }
            };
            match step {
                Err((session_id, patch, code)) => {
                    if patch && let Some(q) = g.mailbox.get_mut(&session_id) {
                        for n in q.iter_mut() {
                            if n.bash_id == id {
                                n.user_killed = true;
                                n.exit_code = code;
                            }
                        }
                    }
                    return;
                }
                Ok(notice) => {
                    g.mailbox
                        .entry(notice.session_id.clone())
                        .or_default()
                        .push_back(notice.clone());
                    g.generation += 1;
                    notice
                }
            }
        };
        self.cv.notify_all();
        self.emit_jobs_changed(&notice.session_id);
        if let Some(handler) = self.exit_handler.lock().expect("exit handler lock").clone() {
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
            self.cv.notify_all();
        }
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
            self.cv.notify_all();
            waiter.session_id
        };
        self.emit_jobs_changed(&session_id);
    }

    pub fn wire_snapshot(&self, session_id: &str) -> BashJobsSnapshot {
        let g = self.inner.lock().expect("jobs lock");
        let mut jobs: Vec<BashJobWire> = g
            .jobs
            .iter()
            .filter(|(_, rec)| rec.alive && rec.session_id == session_id)
            .map(|(id, rec)| BashJobWire {
                id: id.clone(),
                call_id: rec.call_id.clone(),
                command_preview: rec.command_preview.clone(),
                output_file: output_file_rel(&rec.output_path),
                started_at_ms: rec.started_at_ms,
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

    /// Drain unconsumed exits for `session_id` (next tool result reminder).
    pub fn take_mailbox(&self, session_id: &str) -> Vec<ExitNotice> {
        let mut g = self.inner.lock().expect("jobs lock");
        g.mailbox
            .remove(session_id)
            .map(|d| d.into_iter().collect())
            .unwrap_or_default()
    }

    fn take_notice_from_mailbox(g: &mut JobState, session_id: &str, bash_id: &str) {
        if let Some(q) = g.mailbox.get_mut(session_id) {
            q.retain(|n| n.bash_id != bash_id);
            if q.is_empty() {
                g.mailbox.remove(session_id);
            }
        }
    }

    pub fn wait(
        &self,
        session_id: &str,
        watch_id: Option<&str>,
        timeout: Option<Duration>,
        cancel: &CancellationToken,
        any_session_exit: bool,
    ) -> WaitOutcome {
        if let Some(id) = watch_id {
            let g = self.inner.lock().expect("jobs lock");
            match g.jobs.get(id) {
                None => return WaitOutcome::UnknownId(id.to_string()),
                Some(rec) if rec.session_id != session_id => {
                    return WaitOutcome::UnknownId(id.to_string());
                }
                Some(rec) if !rec.alive => {
                    let notice = rec.to_notice(id);
                    drop(g);
                    let mut g = self.inner.lock().expect("jobs lock");
                    Self::take_notice_from_mailbox(&mut g, session_id, id);
                    return WaitOutcome::Exited(notice);
                }
                Some(_) => {}
            }
        }

        let deadline = timeout.map(|d| Instant::now() + d);
        let mut g = self.inner.lock().expect("jobs lock");
        loop {
            if cancel.is_cancelled() {
                return WaitOutcome::Cancelled;
            }

            if any_session_exit
                && let Some(notice) = g.mailbox.get_mut(session_id).and_then(|q| q.pop_front())
            {
                if g.mailbox.get(session_id).is_some_and(|q| q.is_empty()) {
                    g.mailbox.remove(session_id);
                }
                return WaitOutcome::Exited(notice);
            }

            if let Some(id) = watch_id
                && let Some(rec) = g.jobs.get(id)
                && !rec.alive
            {
                let notice = rec.to_notice(id);
                Self::take_notice_from_mailbox(&mut g, session_id, id);
                return WaitOutcome::Exited(notice);
            }

            if let Some(deadline) = deadline {
                let now = Instant::now();
                if now >= deadline {
                    return WaitOutcome::TimedOut;
                }
                let remaining = deadline.saturating_duration_since(now);
                let slice = remaining.min(Duration::from_millis(50));
                let (guard, _) = self.cv.wait_timeout(g, slice).expect("jobs condvar");
                g = guard;
            } else {
                let (guard, _) = self
                    .cv
                    .wait_timeout(g, Duration::from_millis(50))
                    .expect("jobs condvar");
                g = guard;
            }
        }
    }
}

#[derive(Debug)]
pub enum WaitOutcome {
    Exited(ExitNotice),
    TimedOut,
    Cancelled,
    UnknownId(String),
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
    use crate::terminal::{INLINE_FULL, INLINE_TAIL};

    fn rec(session: &str, call: &str, path: PathBuf, tee: BoundedTee) -> AgentJobRecord {
        AgentJobRecord {
            session_id: session.into(),
            call_id: call.into(),
            command_preview: "sleep".into(),
            output_path: path,
            tee: Arc::new(Mutex::new(tee)),
            alive: true,
            exit_code: None,
            started_at_ms: now_unix_ms(),
            user_killed: false,
        }
    }

    #[test]
    fn wait_id_returns_when_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        reg.insert("bg_a".into(), rec("s1", "call_a", path, tee));
        let cancel = CancellationToken::new();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(30));
                reg.finish("bg_a", Some(0));
            });
            match reg.wait(
                "s1",
                Some("bg_a"),
                Some(Duration::from_secs(2)),
                &cancel,
                false,
            ) {
                WaitOutcome::Exited(n) => {
                    assert_eq!(n.bash_id, "bg_a");
                    assert_eq!(n.exit_code, Some(0));
                }
                other => panic!("{other:?}"),
            }
        });
    }

    #[test]
    fn wait_sec_times_out() {
        let reg = AgentJobRegistry::new();
        let cancel = CancellationToken::new();
        match reg.wait("s1", None, Some(Duration::from_millis(40)), &cancel, true) {
            WaitOutcome::TimedOut => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wait_any_session_exit_wakes_without_watch_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        reg.insert("bg_b".into(), rec("s1", "call_b", path, tee));
        let cancel = CancellationToken::new();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(30));
                reg.finish("bg_b", Some(0));
            });
            match reg.wait("s1", None, Some(Duration::from_secs(2)), &cancel, true) {
                WaitOutcome::Exited(n) => assert_eq!(n.bash_id, "bg_b"),
                other => panic!("{other:?}"),
            }
        });
        assert!(reg.take_mailbox("s1").is_empty());
    }

    #[test]
    fn wire_snapshot_keeps_running_jobs_and_waiters() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.output");
        let path_b = dir.path().join("b.output");
        let path_done = dir.path().join("done.output");
        let tee_a = BoundedTee::create(path_a.clone()).unwrap();
        let tee_b = BoundedTee::create(path_b.clone()).unwrap();
        let tee_done = BoundedTee::create(path_done.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        reg.insert("bg_a".into(), rec("s1", "call_a", path_a, tee_a));
        reg.insert("bg_b".into(), rec("s2", "call_b", path_b, tee_b));
        reg.insert(
            "bg_done".into(),
            rec("s1", "call_done", path_done, tee_done),
        );
        reg.finish("bg_done", Some(0));
        reg.begin_wait("s1", "wait_1", Some("bg_a"), Some(Duration::from_secs(5)));
        let snap = reg.wire_snapshot("s1");
        assert_eq!(snap.jobs.len(), 1);
        assert_eq!(snap.jobs[0].id, "bg_a");
        assert_eq!(snap.jobs[0].call_id, "call_a");
        assert_eq!(snap.jobs[0].output_file, ".litecode/bash/a.output");
        assert!(snap.jobs[0].started_at_ms > 0);
        assert_eq!(snap.waits.len(), 1);
        assert_eq!(snap.waits[0].call_id, "wait_1");
        assert_eq!(snap.waits[0].watching_id.as_deref(), Some("bg_a"));
        assert!(snap.waits[0].deadline_ms.is_some());
        reg.end_wait("wait_1");
        assert!(reg.wire_snapshot("s1").waits.is_empty());
    }

    #[test]
    fn empty_call_id_does_not_register_waiter() {
        let reg = AgentJobRegistry::new();
        reg.begin_wait("s1", "", Some("bg_a"), None);
        assert!(reg.wire_snapshot("s1").waits.is_empty());
    }

    #[test]
    fn tail_view_uses_full_window_then_frozen_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.output");
        let mut tee = BoundedTee::create(path.clone()).unwrap();
        tee.push_raw("hello-live");
        let reg = AgentJobRegistry::new();
        reg.insert("bg_t".into(), rec("s1", "call_t", path, tee));
        let live = reg.tail_view("bg_t").expect("tail");
        assert_eq!(live.text, "hello-live");
        assert!(!live.truncated_on_disk);
        assert!(live.alive);

        let path2 = dir.path().join("big.output");
        let mut tee2 = BoundedTee::create(path2.clone()).unwrap();
        tee2.push_raw(&"x".repeat(INLINE_FULL + 64));
        reg.insert("bg_big".into(), rec("s1", "call_big", path2, tee2));
        let frozen = reg.tail_view("bg_big").expect("frozen tail");
        assert!(frozen.alive);
        assert!(frozen.text.len() <= INLINE_TAIL);
        assert!(!frozen.text.contains("hello-live"));
    }

    #[test]
    fn jobs_changed_fires_on_insert_finish_and_wait() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = Arc::new(AgentJobRegistry::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        reg.set_jobs_changed_handler(Arc::new(move |sid| {
            seen_cb.lock().unwrap().push(sid);
        }));
        reg.insert("bg_c".into(), rec("s1", "call_c", path, tee));
        reg.begin_wait("s1", "w1", Some("bg_c"), None);
        reg.end_wait("w1");
        reg.finish("bg_c", Some(0));
        let events = seen.lock().unwrap().clone();
        assert_eq!(events, vec!["s1", "s1", "s1", "s1"]);
    }

    #[test]
    fn mark_user_kill_before_finish_keeps_mailbox_notice_user_killed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        reg.insert("bg_u".into(), rec("s1", "call_u", path, tee));
        reg.mark_user_kill("bg_u");
        reg.finish("bg_u", Some(143));
        let mail = reg.take_mailbox("s1");
        assert_eq!(mail.len(), 1);
        assert!(mail[0].user_killed);
        assert_eq!(mail[0].exit_code, Some(143));
    }
}
