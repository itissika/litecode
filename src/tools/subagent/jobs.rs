//! Job registry for subagent workers: running list, exit mailbox, waiters,
//! wire snapshot. Does not own sessions or turns.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

const MAX_RETAINED_TERMINAL_JOBS_PER_PARENT: usize = 32;

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone)]
pub struct ExitNotice {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub agent_name: String,
    pub prompt_preview: String,
    /// Durable `turn/end.reason`: completed | cancelled | error | max_steps | unknown.
    pub reason: String,
    pub ok: bool,
    pub stopped: bool,
    pub final_text: String,
}

#[derive(Debug, Clone)]
pub struct RunningJob {
    pub id: String,
    pub agent_name: String,
    pub prompt_preview: String,
    pub started_at_ms: i64,
}

struct JobRecord {
    parent_session_id: String,
    call_id: String,
    agent_name: String,
    prompt_preview: String,
    alive: bool,
    reason: String,
    final_text: String,
    started_at_ms: i64,
}

impl JobRecord {
    fn to_notice(&self, child_id: &str) -> ExitNotice {
        ExitNotice {
            child_session_id: child_id.to_string(),
            parent_session_id: self.parent_session_id.clone(),
            agent_name: self.agent_name.clone(),
            prompt_preview: self.prompt_preview.clone(),
            reason: self.reason.clone(),
            ok: self.reason == "completed",
            stopped: self.reason == "cancelled",
            final_text: self.final_text.clone(),
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
pub struct SubagentJobWire {
    pub id: String,
    #[serde(default)]
    pub call_id: String,
    pub agent_name: String,
    pub prompt_preview: String,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SubagentWaitWire {
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watching_id: Option<String>,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SubagentJobsSnapshot {
    #[serde(default)]
    pub jobs: Vec<SubagentJobWire>,
    #[serde(default)]
    pub waits: Vec<SubagentWaitWire>,
}

struct JobState {
    jobs: HashMap<String, JobRecord>,
    mailbox: HashMap<String, VecDeque<ExitNotice>>,
    waiters: HashMap<String, Waiter>,
}

pub struct SubagentJobBoard {
    inner: Mutex<JobState>,
    cv: Condvar,
    exit_handler: Mutex<Option<Arc<dyn Fn(ExitNotice) + Send + Sync>>>,
    jobs_changed: Mutex<Option<Arc<dyn Fn(String) + Send + Sync>>>,
}

impl Default for SubagentJobBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentJobBoard {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(JobState {
                jobs: HashMap::new(),
                mailbox: HashMap::new(),
                waiters: HashMap::new(),
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
        if session_id.is_empty() {
            return;
        }
        if let Some(handler) = self.jobs_changed.lock().expect("jobs changed lock").clone() {
            handler(session_id.to_string());
        }
    }

    /// Drop the job record (and any queued exit notice) for a removed session.
    /// Protocol delete and the hub's `SessionRemoved` watcher call this.
    pub fn forget_child(&self, child_id: &str) {
        let parent = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(rec) = g.jobs.remove(child_id) else {
                return;
            };
            if let Some(q) = g.mailbox.get_mut(&rec.parent_session_id) {
                q.retain(|n| n.child_session_id != child_id);
                if q.is_empty() {
                    g.mailbox.remove(&rec.parent_session_id);
                }
            }
            rec.parent_session_id
        };
        self.cv.notify_all();
        self.emit_jobs_changed(&parent);
    }

    /// Drop every job whose parent is `parent_session_id`, plus a record keyed
    /// as that id (the parent may itself have been a child).
    pub fn purge_parent(&self, parent_session_id: &str) {
        let children: Vec<String> = {
            let g = self.inner.lock().expect("jobs lock");
            g.jobs
                .iter()
                .filter(|(_, rec)| rec.parent_session_id == parent_session_id)
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in children {
            self.forget_child(&id);
        }
        self.forget_child(parent_session_id);
        let mut g = self.inner.lock().expect("jobs lock");
        g.mailbox.remove(parent_session_id);
    }

    fn prune_terminal_jobs_locked(g: &mut JobState, parent_session_id: &str) {
        let mut terminal: Vec<(String, i64)> = g
            .jobs
            .iter()
            .filter(|(_, record)| !record.alive && record.parent_session_id == parent_session_id)
            .map(|(id, record)| (id.clone(), record.started_at_ms))
            .collect();
        if terminal.len() <= MAX_RETAINED_TERMINAL_JOBS_PER_PARENT {
            return;
        }
        terminal.sort_by_key(|(_, started_at_ms)| *started_at_ms);
        let remove_count = terminal.len() - MAX_RETAINED_TERMINAL_JOBS_PER_PARENT;
        for (id, _) in terminal.into_iter().take(remove_count) {
            g.jobs.remove(&id);
            if let Some(queue) = g.mailbox.get_mut(parent_session_id) {
                queue.retain(|notice| notice.child_session_id != id);
            }
        }
        if g.mailbox
            .get(parent_session_id)
            .is_some_and(|queue| queue.is_empty())
        {
            g.mailbox.remove(parent_session_id);
        }
    }

    pub fn running(&self, parent_session_id: &str) -> Vec<RunningJob> {
        let g = self.inner.lock().expect("jobs lock");
        let mut out: Vec<RunningJob> = g
            .jobs
            .iter()
            .filter(|(_, rec)| rec.alive && rec.parent_session_id == parent_session_id)
            .map(|(id, rec)| RunningJob {
                id: id.clone(),
                agent_name: rec.agent_name.clone(),
                prompt_preview: rec.prompt_preview.clone(),
                started_at_ms: rec.started_at_ms,
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    #[cfg(test)]
    pub fn insert_running_for_test(
        &self,
        parent_session_id: &str,
        child_id: &str,
        agent_name: &str,
        prompt: &str,
    ) {
        let mut g = self.inner.lock().expect("jobs lock");
        g.jobs.insert(
            child_id.to_string(),
            JobRecord {
                parent_session_id: parent_session_id.to_string(),
                call_id: "call_test".into(),
                agent_name: agent_name.into(),
                prompt_preview: prompt_preview(prompt),
                alive: true,
                reason: String::new(),
                final_text: String::new(),
                started_at_ms: now_unix_ms(),
            },
        );
        self.cv.notify_all();
    }

    pub fn is_alive(&self, child_id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(child_id).is_some_and(|r| r.alive)
    }

    pub fn notice_snapshot(&self, child_id: &str) -> Option<ExitNotice> {
        let g = self.inner.lock().expect("jobs lock");
        g.jobs.get(child_id).map(|rec| rec.to_notice(child_id))
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
            self.cv.notify_all();
            waiter.session_id
        };
        self.emit_jobs_changed(&session_id);
    }

    pub fn wire_snapshot(&self, session_id: &str) -> SubagentJobsSnapshot {
        let g = self.inner.lock().expect("jobs lock");
        let mut jobs: Vec<SubagentJobWire> = g
            .jobs
            .iter()
            .filter(|(_, rec)| rec.alive && rec.parent_session_id == session_id)
            .map(|(id, rec)| SubagentJobWire {
                id: id.clone(),
                call_id: rec.call_id.clone(),
                agent_name: rec.agent_name.clone(),
                prompt_preview: rec.prompt_preview.clone(),
                started_at_ms: rec.started_at_ms,
            })
            .collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        let mut waits: Vec<SubagentWaitWire> = g
            .waiters
            .values()
            .filter(|w| w.session_id == session_id)
            .map(|w| SubagentWaitWire {
                call_id: w.call_id.clone(),
                watching_id: w.watching_id.clone(),
                started_at_ms: w.started_at_ms,
                deadline_ms: w.deadline_ms,
            })
            .collect();
        waits.sort_by(|a, b| a.call_id.cmp(&b.call_id));
        SubagentJobsSnapshot { jobs, waits }
    }

    pub fn take_notice(&self, parent_session_id: &str, child_id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        Self::take_notice_from_mailbox(&mut g, parent_session_id, child_id);
    }

    pub fn take_mailbox(&self, parent_session_id: &str) -> Vec<ExitNotice> {
        let mut g = self.inner.lock().expect("jobs lock");
        g.mailbox
            .remove(parent_session_id)
            .map(|d| d.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn mailbox_pending(&self, parent_session_id: &str) -> bool {
        let g = self.inner.lock().expect("jobs lock");
        g.mailbox
            .get(parent_session_id)
            .is_some_and(|q| !q.is_empty())
    }

    fn take_notice_from_mailbox(g: &mut JobState, parent_session_id: &str, child_id: &str) {
        if let Some(q) = g.mailbox.get_mut(parent_session_id) {
            q.retain(|n| n.child_session_id != child_id);
            if q.is_empty() {
                g.mailbox.remove(parent_session_id);
            }
        }
    }

    /// Register a child turn the session already started.
    pub fn register_child(
        &self,
        child_session_id: &str,
        parent_session_id: &str,
        call_id: &str,
        agent_name: &str,
        prompt_preview: String,
    ) {
        {
            let mut g = self.inner.lock().expect("jobs lock");
            // A new turn supersedes the child's previous exit notice: waiting on
            // the child means waiting for its current turn.
            if let Some(queue) = g.mailbox.get_mut(parent_session_id) {
                queue.retain(|notice| notice.child_session_id != child_session_id);
                if queue.is_empty() {
                    g.mailbox.remove(parent_session_id);
                }
            }
            g.jobs.insert(
                child_session_id.to_string(),
                JobRecord {
                    parent_session_id: parent_session_id.to_string(),
                    call_id: call_id.to_string(),
                    agent_name: agent_name.to_string(),
                    prompt_preview,
                    alive: true,
                    reason: String::new(),
                    final_text: String::new(),
                    started_at_ms: now_unix_ms(),
                },
            );
            self.cv.notify_all();
        }
        self.emit_jobs_changed(parent_session_id);
    }

    pub fn finish(&self, child_id: &str, ok: bool, stopped: bool, final_text: String) {
        let reason = if stopped {
            "cancelled"
        } else if ok {
            "completed"
        } else {
            "error"
        };
        self.finish_matching(child_id, None, reason, final_text);
    }

    /// Settle only if the live record still belongs to `call_id`. A stale
    /// watcher from a previous turn must not overwrite a newer send.
    pub fn finish_for_call(
        &self,
        child_id: &str,
        call_id: &str,
        reason: &str,
        final_text: String,
    ) {
        self.finish_matching(child_id, Some(call_id), reason, final_text);
    }

    fn finish_matching(
        &self,
        child_id: &str,
        call_id: Option<&str>,
        reason: &str,
        final_text: String,
    ) {
        let notice = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(rec) = g.jobs.get_mut(child_id) else {
                return;
            };
            if call_id.is_some_and(|id| rec.call_id != id) {
                return;
            }
            if !rec.alive {
                if rec.reason == "unknown" && reason != "unknown" {
                    rec.reason = reason.to_string();
                    if rec.final_text.is_empty() {
                        rec.final_text = final_text;
                    }
                } else if rec.final_text.is_empty() && !final_text.is_empty() {
                    rec.final_text = final_text;
                }
                return;
            }
            rec.alive = false;
            rec.reason = reason.to_string();
            rec.final_text = final_text;
            let notice = rec.to_notice(child_id);
            g.mailbox
                .entry(notice.parent_session_id.clone())
                .or_default()
                .push_back(notice.clone());
            Self::prune_terminal_jobs_locked(&mut g, &notice.parent_session_id);
            notice
        };
        self.after_settle(notice);
    }

    fn after_settle(&self, notice: ExitNotice) {
        self.cv.notify_all();
        self.emit_jobs_changed(&notice.parent_session_id);
        if matches!(notice.reason.as_str(), "error" | "max_steps") {
            tracing::error!(
                parent_session_id = %notice.parent_session_id,
                child_session_id = %notice.child_session_id,
                agent = %notice.agent_name,
                reason = %notice.reason,
                error = %notice.final_text,
                "subagent worker failed"
            );
        }
        if let Some(handler) = self.exit_handler.lock().expect("exit handler lock").clone() {
            handler(notice);
        }
    }

    pub fn wait(
        &self,
        parent_session_id: &str,
        watch_id: Option<&str>,
        timeout: Option<Duration>,
        cancel: &CancellationToken,
        any_session_exit: bool,
    ) -> WaitOutcome {
        if let Some(id) = watch_id {
            let g = self.inner.lock().expect("jobs lock");
            match g.jobs.get(id) {
                None => return WaitOutcome::UnknownId(id.to_string()),
                Some(rec) if rec.parent_session_id != parent_session_id => {
                    return WaitOutcome::UnknownId(id.to_string());
                }
                Some(rec) if !rec.alive => {
                    let notice = rec.to_notice(id);
                    drop(g);
                    let mut g = self.inner.lock().expect("jobs lock");
                    Self::take_notice_from_mailbox(&mut g, parent_session_id, id);
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
                && let Some(notice) = g
                    .mailbox
                    .get_mut(parent_session_id)
                    .and_then(|q| q.pop_front())
            {
                if g.mailbox
                    .get(parent_session_id)
                    .is_some_and(|q| q.is_empty())
                {
                    g.mailbox.remove(parent_session_id);
                }
                return WaitOutcome::Exited(notice);
            }

            if let Some(id) = watch_id {
                match g.jobs.get(id) {
                    None => return WaitOutcome::UnknownId(id.to_string()),
                    Some(rec) if rec.parent_session_id != parent_session_id => {
                        return WaitOutcome::UnknownId(id.to_string());
                    }
                    Some(rec) if !rec.alive => {
                        let notice = rec.to_notice(id);
                        Self::take_notice_from_mailbox(&mut g, parent_session_id, id);
                        return WaitOutcome::Exited(notice);
                    }
                    Some(_) => {}
                }
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

    /// Report whether a live job can be cancelled, or the settled outcome.
    ///
    /// Cancelling the turn itself is `SessionManager::cancel_turn_sync`; the
    /// registry does not pre-stamp `cancelled`.
    pub fn mark_stop(&self, parent_session_id: &str, child_id: &str) -> Result<StopMark, String> {
        let mut g = self.inner.lock().expect("jobs lock");
        match g.jobs.get_mut(child_id) {
            None => Err(child_id.to_string()),
            Some(rec) if rec.parent_session_id != parent_session_id => Err(child_id.to_string()),
            Some(rec) if !rec.alive => {
                let notice = rec.to_notice(child_id);
                drop(g);
                self.take_notice(parent_session_id, child_id);
                Ok(StopMark::AlreadyEnded(notice))
            }
            Some(rec) => {
                let notice = rec.to_notice(child_id);
                drop(g);
                Ok(StopMark::CancelRequested(notice))
            }
        }
    }

}

/// Outcome of a stop request against the registry.
#[derive(Debug)]
pub enum StopMark {
    /// The job already settled; the notice carries the real outcome.
    AlreadyEnded(ExitNotice),
    /// The job was running; the caller must ask the session to cancel the turn.
    CancelRequested(ExitNotice),
}

#[derive(Debug)]
pub enum WaitOutcome {
    Exited(ExitNotice),
    TimedOut,
    Cancelled,
    UnknownId(String),
}

/// Compact running list for wait / stop answers.
pub fn format_running_list(jobs: &[RunningJob]) -> String {
    if jobs.is_empty() {
        return "running: 0\n".into();
    }
    let mut out = format!("running: {}\n", jobs.len());
    let now = now_unix_ms();
    for j in jobs {
        out.push_str(&format!(
            "- {}  {}  {}  {}\n",
            j.id,
            j.agent_name,
            elapsed_label(j.started_at_ms, now),
            j.prompt_preview
        ));
    }
    out
}

/// `<system-reminder>` appended to a tool result / idle auto-turn when child
/// jobs have exited since the last drain.
pub fn format_exit_reminder(notices: &[ExitNotice], jobs: &[RunningJob]) -> String {
    let mut inner = String::new();
    for n in notices {
        if n.stopped {
            inner.push_str(&format!(
                "Subagent {} ({}) was stopped.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        } else if n.ok {
            inner.push_str(&format!(
                "Subagent {} ({}) finished.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        } else if n.reason == "unknown" {
            inner.push_str(&format!(
                "Subagent {} ({}) ended with unknown outcome.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        } else {
            inner.push_str(&format!(
                "Subagent {} ({}) failed.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        }
        if !n.final_text.is_empty() {
            let preview: String = n.final_text.chars().take(240).collect();
            inner.push_str(&format!("output_preview: {preview}\n"));
        }
    }
    inner.push_str(&format_running_list(jobs));
    inner.push_str(
        "Use session_search to read the child transcript. subagent_wait / subagent_stop for remaining workers.\n",
    );
    format!("<system-reminder>\n{}</system-reminder>", inner.trim_end())
}

/// Compact elapsed label for the running list: "45s", "3m12s", "1h02m".
fn elapsed_label(started_at_ms: i64, now_ms: i64) -> String {
    let secs = now_ms.saturating_sub(started_at_ms).max(0) as u64 / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Collapse whitespace and cap a prompt for the running list / exit notice.
pub fn prompt_preview(prompt: &str) -> String {
    let collapsed: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
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
    fn elapsed_label_is_compact() {
        assert_eq!(elapsed_label(0, 0), "0s");
        assert_eq!(elapsed_label(0, 45_000), "45s");
        assert_eq!(elapsed_label(0, 192_000), "3m12s");
        assert_eq!(elapsed_label(0, 3_720_000), "1h02m");
    }

    #[test]
    fn forget_child_drops_record_and_notice() {
        let hub = SubagentJobBoard::new();
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        hub.finish("child-a", true, false, "done".into());
        assert!(hub.mailbox_pending("p1"));
        hub.forget_child("child-a");
        assert!(hub.notice_snapshot("child-a").is_none());
        assert!(!hub.mailbox_pending("p1"));
        assert!(hub.running("p1").is_empty());
    }

    #[test]
    fn forget_child_drops_live_record() {
        let hub = SubagentJobBoard::new();
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        assert!(hub.is_alive("child-a"));
        hub.forget_child("child-a");
        assert!(hub.running("p1").is_empty());
        assert!(!hub.is_alive("child-a"));
        assert!(hub.notice_snapshot("child-a").is_none());
    }

    #[test]
    fn terminal_jobs_are_bounded_per_parent() {
        let hub = SubagentJobBoard::new();
        for i in 0..(MAX_RETAINED_TERMINAL_JOBS_PER_PARENT + 5) {
            let id = format!("child-{i}");
            hub.insert_running_for_test("p1", &id, "reviewer", "x");
            hub.finish(&id, true, false, "done".into());
        }
        let g = hub.inner.lock().expect("jobs lock");
        let terminal = g
            .jobs
            .values()
            .filter(|record| record.parent_session_id == "p1")
            .count();
        assert!(
            terminal <= MAX_RETAINED_TERMINAL_JOBS_PER_PARENT,
            "terminal records must stay bounded, got {terminal}"
        );
    }

    #[test]
    fn wait_unknown_id() {
        let hub = SubagentJobBoard::new();
        let out = hub.wait(
            "p",
            Some("missing"),
            Some(Duration::from_millis(10)),
            &CancellationToken::new(),
            false,
        );
        assert!(matches!(out, WaitOutcome::UnknownId(_)));
    }

    #[test]
    fn running_list_has_count_not_capacity() {
        let jobs = vec![RunningJob {
            id: "child-a".into(),
            agent_name: "reviewer".into(),
            prompt_preview: "go".into(),
            started_at_ms: now_unix_ms(),
        }];
        let text = format_running_list(&jobs);
        assert!(text.starts_with("running: 1\n"), "{text}");
        assert!(!text.contains('/'), "{text}");
        assert_eq!(format_running_list(&[]), "running: 0\n");
    }

    #[test]
    fn finish_stores_raw_reason() {
        let hub = SubagentJobBoard::new();
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        hub.finish_for_call("child-a", "call_test", "max_steps", "almost".into());
        let notice = hub.notice_snapshot("child-a").expect("notice");
        assert_eq!(notice.reason, "max_steps");
        assert!(!notice.ok && !notice.stopped);
        assert_eq!(notice.final_text, "almost");
    }

    #[test]
    fn wait_returns_when_watched_child_is_forgotten() {
        let hub = std::sync::Arc::new(SubagentJobBoard::new());
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        let hub_forget = std::sync::Arc::clone(&hub);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            hub_forget.forget_child("child-a");
        });
        let out = hub.wait(
            "p1",
            Some("child-a"),
            Some(Duration::from_secs(2)),
            &CancellationToken::new(),
            false,
        );
        assert!(
            matches!(out, WaitOutcome::UnknownId(ref id) if id == "child-a"),
            "deleting a watched child must wake wait, got {out:?}"
        );
    }
}
