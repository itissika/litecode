//! Parent-side routing state for completed child turns.
//!
//! Child lifecycle and outcome remain Session facts. This inbox stores only
//! references that have not reached a safe parent injection point yet.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRef {
    pub parent_session_id: String,
    pub child_session_id: String,
    pub turn_id: String,
    /// WP-098 R-W: the turn ended `cancelled` (mailbox only, no parent wake).
    pub cancelled: bool,
    /// WP-098 v1.1: same value as `turn/end.cancelled_by`; for subagent_wait.
    pub cancelled_by: Option<crate::types::StopOrigin>,
}

#[derive(Default)]
pub struct CompletionInbox {
    inner: Mutex<HashMap<String, VecDeque<CompletionRef>>>,
}

impl CompletionInbox {
    /// WP-098 R-L: at most one unread result per child. A newer turn replaces
    /// the unread older one in place; the same turn again is a no-op.
    /// Returns true when the inbox changed.
    pub fn push(&self, completion: CompletionRef) -> bool {
        let mut inner = self.inner.lock().expect("completion inbox lock");
        let queue = inner
            .entry(completion.parent_session_id.clone())
            .or_default();
        match queue
            .iter_mut()
            .find(|existing| existing.child_session_id == completion.child_session_id)
        {
            Some(existing) if existing.turn_id == completion.turn_id => false,
            Some(existing) => {
                *existing = completion;
                true
            }
            None => {
                queue.push_back(completion);
                true
            }
        }
    }

    pub fn pending(&self, parent_session_id: &str) -> bool {
        self.inner
            .lock()
            .expect("completion inbox lock")
            .get(parent_session_id)
            .is_some_and(|queue| !queue.is_empty())
    }

    pub fn take_all(&self, parent_session_id: &str) -> Vec<CompletionRef> {
        self.inner
            .lock()
            .expect("completion inbox lock")
            .remove(parent_session_id)
            .map(VecDeque::into_iter)
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    pub fn restore_front(&self, parent_session_id: &str, completions: Vec<CompletionRef>) {
        if completions.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("completion inbox lock");
        let queue = inner.entry(parent_session_id.to_string()).or_default();
        for completion in completions.into_iter().rev() {
            // A result pushed for the same child since the take is newer: keep it.
            if queue
                .iter()
                .any(|existing| existing.child_session_id == completion.child_session_id)
            {
                continue;
            }
            queue.push_front(completion);
        }
    }

    pub fn purge_parent(&self, parent_session_id: &str) {
        self.inner
            .lock()
            .expect("completion inbox lock")
            .remove(parent_session_id);
    }

    pub fn forget_child(&self, child_session_id: &str) {
        let mut inner = self.inner.lock().expect("completion inbox lock");
        inner.retain(|_, queue| {
            queue.retain(|completion| completion.child_session_id != child_session_id);
            !queue.is_empty()
        });
    }
}

/// WP-090 K5: parent-side notice that never wakes the parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentNotice {
    ChildStopped {
        child_session_id: String,
        by: crate::types::StopOrigin,
    },
    ChildDeleted {
        child_session_id: String,
        by: crate::types::StopOrigin,
    },
}

impl ParentNotice {
    pub fn child_session_id(&self) -> &str {
        match self {
            Self::ChildStopped {
                child_session_id, ..
            }
            | Self::ChildDeleted {
                child_session_id, ..
            } => child_session_id,
        }
    }

    pub fn by(&self) -> crate::types::StopOrigin {
        match self {
            Self::ChildStopped { by, .. } | Self::ChildDeleted { by, .. } => *by,
        }
    }

    /// WP-098 dedup key: same kind about the same child.
    fn same_slot(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.child_session_id() == other.child_session_id()
    }

    /// Reminder reason, e.g. "stopped by user" / "deleted by parent".
    pub fn reason_text(&self) -> String {
        let verb = match self {
            Self::ChildStopped { .. } => "stopped",
            Self::ChildDeleted { .. } => "deleted",
        };
        format!("{verb} by {}", self.by().as_str())
    }
}

/// Separate from [`CompletionInbox`] so notices never count as `has_pending`.
#[derive(Default)]
pub struct NoticeInbox {
    inner: Mutex<HashMap<String, VecDeque<ParentNotice>>>,
}

impl NoticeInbox {
    pub fn push(&self, parent_session_id: &str, notice: ParentNotice) {
        let mut inner = self.inner.lock().expect("notice inbox lock");
        let queue = inner.entry(parent_session_id.to_string()).or_default();
        match queue
            .iter_mut()
            .find(|existing| existing.same_slot(&notice))
        {
            Some(existing) => *existing = notice,
            None => queue.push_back(notice),
        }
    }

    pub fn pending(&self, parent_session_id: &str) -> bool {
        self.inner
            .lock()
            .expect("notice inbox lock")
            .get(parent_session_id)
            .is_some_and(|queue| !queue.is_empty())
    }

    pub fn take_all(&self, parent_session_id: &str) -> Vec<ParentNotice> {
        self.inner
            .lock()
            .expect("notice inbox lock")
            .remove(parent_session_id)
            .map(VecDeque::into_iter)
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    pub fn restore_front(&self, parent_session_id: &str, notices: Vec<ParentNotice>) {
        if notices.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("notice inbox lock");
        let queue = inner.entry(parent_session_id.to_string()).or_default();
        for notice in notices.into_iter().rev() {
            if !queue.iter().any(|existing| existing.same_slot(&notice)) {
                queue.push_front(notice);
            }
        }
    }

    pub fn purge_parent(&self, parent_session_id: &str) {
        self.inner
            .lock()
            .expect("notice inbox lock")
            .remove(parent_session_id);
    }
}

#[cfg(test)]
mod tests {}
