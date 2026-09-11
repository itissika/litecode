//! Process-scoped subagent job registry: spawn, wait, stop, mailbox.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::workspace::workspace_root_from_paths;
use crate::runtime::{BindingSource, RuntimeHandle, TurnOptions, spawn_turn};
use crate::session::manager::SessionManager;
use crate::types::LitecodeError;

pub const MAX_SUBAGENTS_PER_PARENT: u32 = 4;
const MAX_RETAINED_TERMINAL_JOBS_PER_PARENT: usize = 32;

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

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

#[derive(Debug, Clone)]
pub struct ExitNotice {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub agent_name: String,
    pub prompt_preview: String,
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

pub struct SpawnDeps {
    /// Live runtime handle of the parent turn. The child re-applies settings
    /// from the global DB at spawn time (same as a main-session turn) and
    /// resolves its own LLM binding from the agent profile — never from the
    /// parent session's provider.
    pub runtime: RuntimeHandle,
    pub depth: u32,
    pub sessions: Arc<SessionManager>,
}

pub struct LaunchSpec {
    pub agent_name: String,
    pub prompt: String,
    pub model_id_override: Option<String>,
    pub max_steps_override: Option<u32>,
}

struct JobRecord {
    parent_session_id: String,
    call_id: String,
    agent_name: String,
    prompt_preview: String,
    alive: bool,
    ok: bool,
    stopped: bool,
    final_text: String,
    started_at_ms: i64,
    cancel: CancellationToken,
}

impl JobRecord {
    fn to_notice(&self, child_id: &str) -> ExitNotice {
        ExitNotice {
            child_session_id: child_id.to_string(),
            parent_session_id: self.parent_session_id.clone(),
            agent_name: self.agent_name.clone(),
            prompt_preview: self.prompt_preview.clone(),
            ok: self.ok,
            stopped: self.stopped,
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
    slots: HashMap<String, u32>,
}

pub struct SubagentHub {
    inner: Mutex<JobState>,
    cv: Condvar,
    exit_handler: Mutex<Option<Arc<dyn Fn(ExitNotice) + Send + Sync>>>,
    jobs_changed: Mutex<Option<Arc<dyn Fn(String) + Send + Sync>>>,
    sessions: Mutex<Option<Arc<SessionManager>>>,
}

impl Default for SubagentHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(JobState {
                jobs: HashMap::new(),
                mailbox: HashMap::new(),
                waiters: HashMap::new(),
                slots: HashMap::new(),
            }),
            cv: Condvar::new(),
            exit_handler: Mutex::new(None),
            jobs_changed: Mutex::new(None),
            sessions: Mutex::new(None),
        }
    }

    pub fn attach_sessions(&self, sessions: Arc<SessionManager>) {
        *self.sessions.lock().expect("sessions lock") = Some(sessions);
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

    pub fn try_acquire_slot(&self, parent_session_id: &str) -> Result<(), String> {
        let mut g = self.inner.lock().expect("jobs lock");
        let n = g.slots.entry(parent_session_id.to_string()).or_insert(0);
        if *n >= MAX_SUBAGENTS_PER_PARENT {
            let mut running: Vec<RunningJob> = g
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
            running.sort_by(|a, b| a.id.cmp(&b.id));
            return Err(format!(
                "subagent capacity exceeded: {MAX_SUBAGENTS_PER_PARENT} subagents are already \
                 running for this session. Wait for one to finish (subagent_wait) or stop one \
                 (subagent_stop), then retry.\n{}",
                super::status::format_running_list(&running)
            ));
        }
        *n += 1;
        Ok(())
    }

    fn release_slot(&self, parent_session_id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        if let Some(n) = g.slots.get_mut(parent_session_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                g.slots.remove(parent_session_id);
            }
        }
    }

    /// Drop every job/mailbox/waiter/slot owned by a deleted parent session.
    /// Child sessions themselves are removed by SessionManager's durable cascade.
    pub fn purge_parent(&self, parent_session_id: &str) {
        let mut g = self.inner.lock().expect("jobs lock");
        g.jobs
            .retain(|_, record| record.parent_session_id != parent_session_id);
        g.mailbox.remove(parent_session_id);
        g.waiters
            .retain(|_, waiter| waiter.session_id != parent_session_id);
        g.slots.remove(parent_session_id);
        drop(g);
        self.cv.notify_all();
    }

    /// Drop the job record (and any queued exit notice) for a session that was
    /// deleted directly. `purge_parent` clears the deleted session's own
    /// children; this clears its record under its own parent.
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
            if rec.alive {
                if let Some(n) = g.slots.get_mut(&rec.parent_session_id) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        g.slots.remove(&rec.parent_session_id);
                    }
                }
                rec.cancel.cancel();
            }
            rec.parent_session_id
        };
        self.cv.notify_all();
        self.emit_jobs_changed(&parent);
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
        self.try_acquire_slot(parent_session_id).expect("slot");
        let mut g = self.inner.lock().expect("jobs lock");
        g.jobs.insert(
            child_id.to_string(),
            JobRecord {
                parent_session_id: parent_session_id.to_string(),
                call_id: "call_test".into(),
                agent_name: agent_name.into(),
                prompt_preview: prompt_preview(prompt),
                alive: true,
                ok: false,
                stopped: false,
                final_text: String::new(),
                started_at_ms: now_unix_ms(),
                cancel: CancellationToken::new(),
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

    pub fn finish(&self, child_id: &str, ok: bool, stopped: bool, final_text: String) {
        let parent = {
            let g = self.inner.lock().expect("jobs lock");
            g.jobs
                .get(child_id)
                .map(|r| r.parent_session_id.clone())
                .unwrap_or_default()
        };
        let notice = {
            let mut g = self.inner.lock().expect("jobs lock");
            let Some(rec) = g.jobs.get_mut(child_id) else {
                return;
            };
            if !rec.alive {
                rec.ok = rec.ok || ok;
                rec.stopped = rec.stopped || stopped;
                if rec.final_text.is_empty() {
                    rec.final_text = final_text;
                }
                return;
            }
            rec.alive = false;
            rec.ok = ok;
            rec.stopped = rec.stopped || stopped;
            rec.final_text = final_text;
            let notice = rec.to_notice(child_id);
            g.mailbox
                .entry(notice.parent_session_id.clone())
                .or_default()
                .push_back(notice.clone());
            Self::prune_terminal_jobs_locked(&mut g, &notice.parent_session_id);
            notice
        };
        if !parent.is_empty() {
            self.release_slot(&parent);
        }
        self.cv.notify_all();
        self.emit_jobs_changed(&notice.parent_session_id);
        if !ok && !stopped {
            tracing::error!(
                parent_session_id = %notice.parent_session_id,
                child_session_id = %notice.child_session_id,
                agent = %notice.agent_name,
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

            if let Some(id) = watch_id
                && let Some(rec) = g.jobs.get(id)
                && !rec.alive
            {
                let notice = rec.to_notice(id);
                Self::take_notice_from_mailbox(&mut g, parent_session_id, id);
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

    pub fn stop(
        self: &Arc<Self>,
        parent_session_id: &str,
        child_id: &str,
    ) -> Result<ExitNotice, String> {
        let cancel = {
            let mut g = self.inner.lock().expect("jobs lock");
            match g.jobs.get_mut(child_id) {
                None => return Err(child_id.to_string()),
                Some(rec) if rec.parent_session_id != parent_session_id => {
                    return Err(child_id.to_string());
                }
                Some(rec) if !rec.alive => {
                    let notice = rec.to_notice(child_id);
                    drop(g);
                    self.take_notice(parent_session_id, child_id);
                    return Ok(notice);
                }
                Some(rec) => {
                    // Mark under the same lock that observed `alive`: a racing
                    // finish() can never relabel a normally completed child as
                    // stopped, and once stop wins the outcome stays stopped.
                    rec.stopped = true;
                    rec.cancel.clone()
                }
            }
        };
        cancel.cancel();
        if let Some(sessions) = self.sessions.lock().expect("sessions lock").clone() {
            sessions.cancel_turn_sync(child_id);
        }
        self.notice_snapshot(child_id)
            .ok_or_else(|| child_id.to_string())
    }

    pub async fn spawn(
        self: &Arc<Self>,
        parent_session_id: &str,
        call_id: &str,
        spec: LaunchSpec,
        deps: SpawnDeps,
    ) -> Result<String, String> {
        if call_id.is_empty() {
            tracing::error!(parent_session_id, "subagent_launch missing tool call_id");
            return Err(
                "subagent_launch requires an active tool call_id (missing execution context)"
                    .into(),
            );
        }
        if let Err(error) = self.try_acquire_slot(parent_session_id) {
            tracing::warn!(
                parent_session_id,
                agent = %spec.agent_name,
                error = %error,
                "subagent spawn rejected"
            );
            return Err(error);
        }
        self.attach_sessions(Arc::clone(&deps.sessions));

        // First-class turn path: re-read live settings from the global DB
        // (same reload a main-session turn start runs) and spawn through the
        // unified `spawn_turn` entry. The child resolves its own LLM binding
        // from its agent profile — never from the parent session's provider
        // and never from the parent tool list's config snapshot.
        let mut runtime = deps.runtime.clone();
        if let Err(e) = runtime.apply_non_engine() {
            tracing::error!(parent_session_id, error = %e, "subagent settings reload failed");
            self.release_slot(parent_session_id);
            return Err(format!("settings reload failed: {e}"));
        }
        // Disk is source of truth for workspace MCP / custom-tool defs (the
        // watcher skips reload while a turn runs). Same apply point as the
        // main-session turn start so both entries read the same disk state.
        runtime.sync_workspace_tool_readiness();

        let project = workspace_root_from_paths(runtime.resolved.paths())
            .to_string_lossy()
            .to_string();
        let seed_model = spec.model_id_override.as_deref().or_else(|| {
            runtime
                .resolved
                .agents()
                .get(&spec.agent_name)
                .map(|p| p.model_ref.as_str())
                .filter(|s| !s.is_empty())
        });

        let child_session_id = match deps.sessions.open_child_session(
            &project,
            &spec.agent_name,
            seed_model,
            parent_session_id,
            call_id,
        ) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(
                    parent_session_id,
                    agent = %spec.agent_name,
                    error = %e,
                    "subagent child session creation failed"
                );
                self.release_slot(parent_session_id);
                return Err(format!("child session creation failed: {e}"));
            }
        };

        if !deps.sessions.publish_internal(
            parent_session_id,
            crate::runtime::observer::InternalEvent::SubagentBound {
                call_id: call_id.to_string(),
                child_session_id: child_session_id.clone(),
            },
        ) {
            tracing::warn!(
                parent_session_id,
                child_session_id = %child_session_id,
                "subagent bound event dropped (parent session missing)"
            );
        }

        let abort = |sessions: &SessionManager,
                     child_id: &str,
                     hub: &SubagentHub,
                     parent: &str,
                     reason: &str| {
            tracing::warn!(
                parent_session_id = parent,
                child_session_id = child_id,
                agent = %spec.agent_name,
                reason,
                "subagent spawn aborted"
            );
            let _ = sessions.remove_session(child_id);
            hub.release_slot(parent);
        };

        let turn_id = Uuid::new_v4().to_string();
        let opts = TurnOptions {
            binding: BindingSource::Agent {
                name: spec.agent_name.clone(),
                model_id_override: spec.model_id_override.clone(),
            },
            depth: deps.depth + 1,
            max_steps_override: spec.max_steps_override,
        };
        let mut turn_handle = match spawn_turn(
            &runtime,
            child_session_id.clone(),
            Arc::clone(&deps.sessions),
            spec.prompt.clone(),
            crate::permission::deny_permission_sink(),
            turn_id.clone(),
            opts,
        ) {
            Ok(h) => h,
            Err(e) => {
                abort(
                    &deps.sessions,
                    &child_session_id,
                    self,
                    parent_session_id,
                    &format!("turn spawn failed: {e}"),
                );
                return Err(format!("turn spawn failed: {e}"));
            }
        };
        let step_max = turn_handle.step_max;
        let child_cancel = turn_handle.cancel.clone();
        // The hub's joiner thread owns turn finalization (exactly-once via
        // WorkerGuard); the session-side handle keeps no join, matching the
        // inline-drive contract of the main-session fanout path.
        let join = turn_handle.handle.take();

        if let Err(e) = deps.sessions.reserve_turn(
            &child_session_id,
            turn_id.clone(),
            step_max,
            &spec.agent_name,
            &project,
        ) {
            abort(
                &deps.sessions,
                &child_session_id,
                self,
                parent_session_id,
                &format!("reserve_turn failed: {e}"),
            );
            return Err(format!("reserve_turn failed: {e}"));
        }
        if let Err(e) = deps
            .sessions
            .start_turn(
                &child_session_id,
                turn_handle,
                &spec.agent_name,
                &project,
                Arc::clone(&deps.sessions),
            )
            .await
        {
            abort(
                &deps.sessions,
                &child_session_id,
                self,
                parent_session_id,
                &format!("start_turn failed: {e}"),
            );
            return Err(format!("start_turn failed: {e}"));
        }

        {
            let mut g = self.inner.lock().expect("jobs lock");
            g.jobs.insert(
                child_session_id.clone(),
                JobRecord {
                    parent_session_id: parent_session_id.to_string(),
                    call_id: call_id.to_string(),
                    agent_name: spec.agent_name.clone(),
                    prompt_preview: prompt_preview(&spec.prompt),
                    alive: true,
                    ok: false,
                    stopped: false,
                    final_text: String::new(),
                    started_at_ms: now_unix_ms(),
                    cancel: child_cancel.clone(),
                },
            );
            self.cv.notify_all();
        }
        self.emit_jobs_changed(parent_session_id);

        let hub = Arc::clone(self);
        let sessions = Arc::clone(&deps.sessions);
        let child_id = child_session_id.clone();
        let parent_for_thread = parent_session_id.to_string();
        let turn_id_thread = turn_id.clone();
        tracing::info!(
            parent_session_id,
            child_session_id = %child_session_id,
            agent = %spec.agent_name,
            call_id,
            "subagent worker starting"
        );
        let spawn_result = std::thread::Builder::new()
            .name(format!("subagent-{child_id}"))
            .spawn(move || {
                let mut guard = WorkerGuard::new(
                    Arc::clone(&hub),
                    Arc::clone(&sessions),
                    child_id.clone(),
                    turn_id_thread.clone(),
                );
                // The turn runs on its own thread (spawned by `spawn_turn`);
                // this joiner owns finalization — exactly once via WorkerGuard,
                // including panics surfaced as a join error.
                let result = match join {
                    Some(j) => match j.join() {
                        Ok(r) => r,
                        Err(_) => Err(LitecodeError::ToolExecution(
                            "subagent worker panicked".into(),
                        )),
                    },
                    None => Err(LitecodeError::ToolExecution(
                        "subagent turn join handle missing".into(),
                    )),
                };
                if !child_cancel.is_cancelled() && !matches!(&result, Err(LitecodeError::Canceled))
                {
                    let mut still_running = true;
                    for _ in 0..200 {
                        if !sessions.is_turn_running_blocking(&child_id) {
                            still_running = false;
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    if still_running {
                        tracing::warn!(
                            parent_session_id = %parent_for_thread,
                            child_session_id = %child_id,
                            "subagent worker exiting while session turn still marked running"
                        );
                    }
                }
                let stopped =
                    child_cancel.is_cancelled() || matches!(&result, Err(LitecodeError::Canceled));
                let (ok, text) = match result {
                    Ok(text) => (true, text),
                    Err(LitecodeError::Canceled) => (false, "subagent cancelled".into()),
                    Err(e) => (false, format!("agent error: {e}")),
                };
                guard.finish(ok, stopped, text);
            });
        if let Err(error) = spawn_result {
            tracing::error!(
                parent_session_id,
                child_session_id = %child_session_id,
                agent = %spec.agent_name,
                error = %error,
                "failed to spawn subagent worker thread"
            );
            let _ = deps.sessions.remove_session(&child_session_id);
            self.finish(
                &child_session_id,
                false,
                false,
                format!("failed to spawn worker thread: {error}"),
            );
            return Err(format!("failed to spawn worker thread: {error}"));
        }

        Ok(child_session_id)
    }
}

/// Finalize a child worker exactly once, even if the worker unwinds.
struct WorkerGuard {
    hub: Arc<SubagentHub>,
    sessions: Arc<SessionManager>,
    child_id: String,
    turn_id: String,
    armed: bool,
}

impl WorkerGuard {
    fn new(
        hub: Arc<SubagentHub>,
        sessions: Arc<SessionManager>,
        child_id: String,
        turn_id: String,
    ) -> Self {
        Self {
            hub,
            sessions,
            child_id,
            turn_id,
            armed: true,
        }
    }

    fn finish(&mut self, ok: bool, stopped: bool, final_text: String) {
        if !self.armed {
            return;
        }
        self.hub.finish(&self.child_id, ok, stopped, final_text);
        let _ = self.sessions.finish_turn(&self.child_id, &self.turn_id);
        self.armed = false;
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.armed {
            tracing::error!(
                child_session_id = %self.child_id,
                "subagent worker panicked"
            );
            self.finish(false, false, "subagent worker panicked".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_caps_in_flight_per_parent() {
        let hub = SubagentHub::new();
        for _ in 0..MAX_SUBAGENTS_PER_PARENT {
            hub.try_acquire_slot("parent-1").expect("slot within cap");
        }
        assert!(hub.try_acquire_slot("parent-1").is_err());
        hub.try_acquire_slot("parent-2").expect("other parent");
        for _ in 0..MAX_SUBAGENTS_PER_PARENT {
            hub.release_slot("parent-1");
        }
        hub.try_acquire_slot("parent-1").expect("slot frees");
    }

    #[test]
    fn purge_parent_drops_jobs_and_mailbox() {
        let hub = SubagentHub::new();
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        hub.finish("child-a", true, false, "done".into());
        assert!(hub.mailbox_pending("p1"));
        hub.purge_parent("p1");
        assert!(!hub.mailbox_pending("p1"));
        assert!(hub.running("p1").is_empty());
        assert!(hub.notice_snapshot("child-a").is_none());
        // Slot must be reusable for a replacement session id? The slot entry
        // itself is keyed by the deleted parent, so a later launch under a new
        // parent id starts from zero.
        hub.try_acquire_slot("p2").expect("new parent slot");
    }

    #[test]
    fn forget_child_drops_record_and_notice() {
        let hub = SubagentHub::new();
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        hub.finish("child-a", true, false, "done".into());
        assert!(hub.mailbox_pending("p1"));
        hub.forget_child("child-a");
        assert!(hub.notice_snapshot("child-a").is_none());
        assert!(!hub.mailbox_pending("p1"));
        assert!(hub.running("p1").is_empty());
    }

    #[test]
    fn forget_child_releases_slot_for_live_record() {
        let hub = SubagentHub::new();
        hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
        hub.forget_child("child-a");
        assert!(hub.running("p1").is_empty());
        for _ in 0..MAX_SUBAGENTS_PER_PARENT {
            hub.try_acquire_slot("p1").expect("slot freed by forget");
        }
        assert!(hub.try_acquire_slot("p1").is_err());
    }

    #[test]
    fn terminal_jobs_are_bounded_per_parent() {
        let hub = SubagentHub::new();
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
        let hub = SubagentHub::new();
        let out = hub.wait(
            "p",
            Some("missing"),
            Some(Duration::from_millis(10)),
            &CancellationToken::new(),
            false,
        );
        assert!(matches!(out, WaitOutcome::UnknownId(_)));
    }
}
