//! Custom-tool completion mailbox for async jobs.
//!
//! Process spawn / cancel live with the executor; this hub only routes exit
//! notices to the parent session seam (same shape as bash ExitNotice mailbox).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

/// Seam reminder kind name (parity with BashExit / SubagentSettled).
/// Bobo emits `reminder/custom_tool_settled` when draining the mailbox.
pub const CUSTOM_TOOL_SETTLED_REMINDER: &str = "CustomToolSettled";

/// Outcome of an async custom-tool job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomToolOutcome {
    Ok { output: String },
    Error { message: String },
    Cancelled,
}

/// One settled custom-tool job waiting for the next request seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomToolExitNotice {
    pub session_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub job_id: String,
    pub outcome: CustomToolOutcome,
    pub revision_hint: Option<u64>,
}

type ExitHandler = Arc<dyn Fn(CustomToolExitNotice) + Send + Sync>;

struct InFlightJob {
    session_id: String,
    cancel: CancellationToken,
}

#[derive(Default)]
struct MailState {
    mailbox: HashMap<String, VecDeque<CustomToolExitNotice>>,
    /// job_id -> in-flight cancel handle
    jobs: HashMap<String, InFlightJob>,
}

/// Thin hub: mailbox + optional idle-auto-turn exit handler.
///
/// Tools fills execute / cancel / in-flight tracking against this same type.
pub struct CustomToolHub {
    inner: Mutex<MailState>,
    exit_handler: Mutex<Option<ExitHandler>>,
}

impl Default for CustomToolHub {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomToolHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MailState::default()),
            exit_handler: Mutex::new(None),
        }
    }

    pub fn set_exit_handler(&self, handler: ExitHandler) {
        *self.exit_handler.lock().expect("custom exit handler lock") = Some(handler);
    }

    /// Register an in-flight background job; returns a child cancel token linked to `parent`.
    pub fn begin_job(
        &self,
        session_id: &str,
        job_id: &str,
        parent: &CancellationToken,
    ) -> CancellationToken {
        let cancel = parent.child_token();
        let mut g = self.inner.lock().expect("custom mailbox lock");
        g.jobs.insert(
            job_id.to_string(),
            InFlightJob {
                session_id: session_id.to_string(),
                cancel: cancel.clone(),
            },
        );
        cancel
    }

    /// Cancel a running job by id (no-op if already finished).
    pub fn cancel_job(&self, job_id: &str) -> bool {
        let g = self.inner.lock().expect("custom mailbox lock");
        if let Some(job) = g.jobs.get(job_id) {
            job.cancel.cancel();
            true
        } else {
            false
        }
    }

    /// Cancel every in-flight job for a session.
    pub fn cancel_session(&self, session_id: &str) {
        let g = self.inner.lock().expect("custom mailbox lock");
        for job in g.jobs.values() {
            if job.session_id == session_id {
                job.cancel.cancel();
            }
        }
    }

    /// Enqueue a completion and wake idle auto-turn if installed.
    pub fn push_notice(&self, notice: CustomToolExitNotice) {
        let session_id = notice.session_id.clone();
        {
            let mut g = self.inner.lock().expect("custom mailbox lock");
            g.jobs.remove(&notice.job_id);
            let queue = g.mailbox.entry(session_id.clone()).or_default();
            if queue.iter().any(|existing| existing.job_id == notice.job_id) {
                return;
            }
            queue.push_back(notice.clone());
        }
        if let Some(handler) = self.exit_handler.lock().expect("custom exit handler lock").clone() {
            handler(notice);
        }
    }

    pub fn take_mailbox(&self, session_id: &str) -> Vec<CustomToolExitNotice> {
        let mut g = self.inner.lock().expect("custom mailbox lock");
        g.mailbox
            .remove(session_id)
            .map(|d| d.into_iter().collect())
            .unwrap_or_default()
    }

    /// Put notices back after a seam failed to persist them. Newer arrivals stay behind.
    pub fn restore_mailbox(&self, session_id: &str, notices: Vec<CustomToolExitNotice>) {
        if notices.is_empty() {
            return;
        }
        let mut g = self.inner.lock().expect("custom mailbox lock");
        let queue = g.mailbox.entry(session_id.to_string()).or_default();
        for notice in notices.into_iter().rev() {
            if queue
                .iter()
                .any(|existing| existing.job_id == notice.job_id)
            {
                continue;
            }
            queue.push_front(notice);
        }
    }

    pub fn mailbox_pending(&self, session_id: &str) -> bool {
        let g = self.inner.lock().expect("custom mailbox lock");
        g.mailbox.get(session_id).is_some_and(|q| !q.is_empty())
    }

    pub fn purge_session(&self, session_id: &str) {
        let mut g = self.inner.lock().expect("custom mailbox lock");
        g.mailbox.remove(session_id);
        g.jobs.retain(|_, job| job.session_id != session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(session: &str, job: &str) -> CustomToolExitNotice {
        CustomToolExitNotice {
            session_id: session.into(),
            call_id: format!("call-{job}"),
            tool_name: "demo".into(),
            job_id: job.into(),
            outcome: CustomToolOutcome::Ok {
                output: "done".into(),
            },
            revision_hint: None,
        }
    }

    #[test]
    fn pending_stays_until_take() {
        let hub = CustomToolHub::new();
        hub.push_notice(notice("s", "j1"));
        assert!(hub.mailbox_pending("s"));
        assert_eq!(hub.take_mailbox("s").len(), 1);
        assert!(!hub.mailbox_pending("s"));
    }

    #[test]
    fn restore_puts_notices_back_in_front() {
        let hub = CustomToolHub::new();
        hub.push_notice(notice("s", "j1"));
        let drained = hub.take_mailbox("s");
        hub.push_notice(notice("s", "j2"));
        hub.restore_mailbox("s", drained);
        let all = hub.take_mailbox("s");
        assert_eq!(
            all.iter().map(|n| n.job_id.as_str()).collect::<Vec<_>>(),
            vec!["j1", "j2"]
        );
    }

    #[test]
    fn push_dedupes_same_job_id() {
        let hub = CustomToolHub::new();
        hub.push_notice(notice("s", "j1"));
        hub.push_notice(notice("s", "j1"));
        assert_eq!(hub.take_mailbox("s").len(), 1);
    }

    #[test]
    fn seam_drain_builds_settled_reminder_text() {
        use crate::reminder::CustomToolSettledEntry;
        use crate::reminder::{Facts, ReminderKind, SeamCtx, SpineReminderView, sync};
        use std::path::PathBuf;

        let hub = CustomToolHub::new();
        hub.push_notice(CustomToolExitNotice {
            session_id: "s".into(),
            call_id: "c1".into(),
            tool_name: "demo".into(),
            job_id: "j1".into(),
            outcome: CustomToolOutcome::Error {
                message: "boom".into(),
            },
            revision_hint: Some(3),
        });
        assert!(hub.mailbox_pending("s"));
        let notices = hub.take_mailbox("s");
        assert!(!hub.mailbox_pending("s"));
        let entries: Vec<_> = notices
            .iter()
            .map(|n| {
                let (status, detail) = match &n.outcome {
                    CustomToolOutcome::Ok { output } => ("ok".into(), output.clone()),
                    CustomToolOutcome::Error { message } => ("error".into(), message.clone()),
                    CustomToolOutcome::Cancelled => ("cancelled".into(), String::new()),
                };
                CustomToolSettledEntry {
                    job_id: n.job_id.clone(),
                    call_id: n.call_id.clone(),
                    tool_name: n.tool_name.clone(),
                    status,
                    detail,
                }
            })
            .collect();
        let facts = Facts {
            custom_tool_settled: entries,
            ..Facts::default()
        };
        let ctx = SeamCtx {
            session_id: "s".into(),
            turn_id: "t".into(),
            step: 1,
            max_steps: 10,
            cwd: PathBuf::from("/work"),
        };
        let reminders = sync(&ctx, &SpineReminderView::default(), &facts);
        let row = reminders
            .iter()
            .find(|r| r.kind() == ReminderKind::CustomToolSettled)
            .expect("custom settled reminder");
        assert!(row.text().contains("status: error"));
        assert!(row.text().contains("detail: boom"));
        assert!(row.text().contains("tool: demo"));
    }

    #[test]
    fn restore_after_failed_persist_keeps_pending() {
        let hub = CustomToolHub::new();
        hub.push_notice(notice("s", "j1"));
        let drained = hub.take_mailbox("s");
        hub.restore_mailbox("s", drained);
        assert!(hub.mailbox_pending("s"));
        assert_eq!(hub.take_mailbox("s")[0].job_id, "j1");
    }
}
