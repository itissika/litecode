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
}

#[derive(Default)]
pub struct CompletionInbox {
    inner: Mutex<HashMap<String, VecDeque<CompletionRef>>>,
}

impl CompletionInbox {
    pub fn push(&self, completion: CompletionRef) {
        let mut inner = self.inner.lock().expect("completion inbox lock");
        let queue = inner
            .entry(completion.parent_session_id.clone())
            .or_default();
        if !queue.iter().any(|existing| {
            existing.child_session_id == completion.child_session_id
                && existing.turn_id == completion.turn_id
        }) {
            queue.push_back(completion);
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
    ChildStopped { child_session_id: String },
    ChildDeleted { child_session_id: String },
}

impl ParentNotice {
    pub fn child_session_id(&self) -> &str {
        match self {
            Self::ChildStopped { child_session_id } | Self::ChildDeleted { child_session_id } => {
                child_session_id
            }
        }
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
        if !queue.contains(&notice) {
            queue.push_back(notice);
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
            if !queue.contains(&notice) {
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
mod tests {
    use super::*;

    fn completion(parent: &str, child: &str, turn: &str) -> CompletionRef {
        CompletionRef {
            parent_session_id: parent.into(),
            child_session_id: child.into(),
            turn_id: turn.into(),
        }
    }

    #[test]
    fn inbox_deduplicates_and_drains_by_parent() {
        let inbox = CompletionInbox::default();
        inbox.push(completion("p", "c", "t"));
        inbox.push(completion("p", "c", "t"));
        assert!(inbox.pending("p"));
        assert_eq!(inbox.take_all("p"), vec![completion("p", "c", "t")]);
        assert!(!inbox.pending("p"));
    }

    #[test]
    fn forgetting_child_preserves_other_completions() {
        let inbox = CompletionInbox::default();
        inbox.push(completion("p", "a", "t1"));
        inbox.push(completion("p", "b", "t2"));
        inbox.forget_child("a");
        assert_eq!(inbox.take_all("p"), vec![completion("p", "b", "t2")]);
    }
}
