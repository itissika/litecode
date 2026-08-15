//! Session-scoped agent bash jobs: running list, wait, and exit mailbox.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::tee::{BoundedTee, TeeCapture};

#[derive(Debug, Clone)]
pub struct ExitNotice {
    pub bash_id: String,
    pub session_id: String,
    pub exit_code: Option<u32>,
    pub output_path: PathBuf,
    pub command_preview: String,
}

#[derive(Debug, Clone)]
pub struct RunningJob {
    pub id: String,
    pub command_preview: String,
    pub output_path: PathBuf,
}

pub struct AgentJobRecord {
    pub session_id: String,
    pub command_preview: String,
    pub output_path: PathBuf,
    pub tee: Arc<Mutex<BoundedTee>>,
    pub alive: bool,
    pub exit_code: Option<u32>,
}

struct JobState {
    jobs: HashMap<String, AgentJobRecord>,
    mailbox: HashMap<String, VecDeque<ExitNotice>>,
    generation: u64,
}

pub struct AgentJobRegistry {
    inner: Mutex<JobState>,
    cv: Condvar,
    exit_handler: Mutex<Option<Arc<dyn Fn(ExitNotice) + Send + Sync>>>,
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
                generation: 0,
            }),
            cv: Condvar::new(),
            exit_handler: Mutex::new(None),
        }
    }

    pub fn set_exit_handler(&self, handler: Arc<dyn Fn(ExitNotice) + Send + Sync>) {
        *self.exit_handler.lock().expect("exit handler lock") = Some(handler);
    }

    pub fn insert(&self, id: String, record: AgentJobRecord) {
        let mut g = self.inner.lock().expect("jobs lock");
        g.jobs.insert(id, record);
        g.generation += 1;
        self.cv.notify_all();
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
        g.jobs.get(id).map(|rec| ExitNotice {
            bash_id: id.to_string(),
            session_id: rec.session_id.clone(),
            exit_code: rec.exit_code,
            output_path: rec.output_path.clone(),
            command_preview: rec.command_preview.clone(),
        })
    }

    pub fn finish(&self, id: &str, exit_code: Option<u32>) {
        let notice = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(rec) = g.jobs.get_mut(id) else {
                return;
            };
            if !rec.alive {
                rec.exit_code = rec.exit_code.or(exit_code);
                return;
            }
            rec.alive = false;
            rec.exit_code = exit_code;
            let notice = ExitNotice {
                bash_id: id.to_string(),
                session_id: rec.session_id.clone(),
                exit_code,
                output_path: rec.output_path.clone(),
                command_preview: rec.command_preview.clone(),
            };
            g.mailbox
                .entry(notice.session_id.clone())
                .or_default()
                .push_back(notice.clone());
            g.generation += 1;
            notice
        };
        self.cv.notify_all();
        if let Some(handler) = self.exit_handler.lock().expect("exit handler lock").clone() {
            handler(notice);
        }
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
                    let notice = ExitNotice {
                        bash_id: id.to_string(),
                        session_id: rec.session_id.clone(),
                        exit_code: rec.exit_code,
                        output_path: rec.output_path.clone(),
                        command_preview: rec.command_preview.clone(),
                    };
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
                let notice = ExitNotice {
                    bash_id: id.to_string(),
                    session_id: rec.session_id.clone(),
                    exit_code: rec.exit_code,
                    output_path: rec.output_path.clone(),
                    command_preview: rec.command_preview.clone(),
                };
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

    #[test]
    fn wait_id_returns_when_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.output");
        let tee = BoundedTee::create(path.clone()).unwrap();
        let reg = AgentJobRegistry::new();
        reg.insert(
            "bg_a".into(),
            AgentJobRecord {
                session_id: "s1".into(),
                command_preview: "sleep".into(),
                output_path: path,
                tee: Arc::new(Mutex::new(tee)),
                alive: true,
                exit_code: None,
            },
        );
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
        reg.insert(
            "bg_b".into(),
            AgentJobRecord {
                session_id: "s1".into(),
                command_preview: "sleep".into(),
                output_path: path,
                tee: Arc::new(Mutex::new(tee)),
                alive: true,
                exit_code: None,
            },
        );
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
}
