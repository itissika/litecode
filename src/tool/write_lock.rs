use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// Resource key: a file path, or the workspace-wide coarse lock.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKey {
    File(String),
    /// Coarse workspace lock. Production bash no longer acquires this.
    Workspace,
}

#[derive(Debug, Clone)]
pub struct LockInfo {
    pub session_id: String,
}

/// Process-wide write-lock registry.
///
/// File-level exclusion for structured writers (`write` / `edit`) across sessions.
/// [`ResourceKey::Workspace`] remains a coarse primitive; production bash does not take it.
///
/// Backed by a sync `std::sync::Mutex`: the critical section only touches a
/// HashMap and never `.await`s, so acquire/release can be synchronous. That
/// removes two problems from the old tokio lock + `Drop`/`spawn` release:
/// (1) drop with no current runtime silently leaked the lock; (2) fire-and-forget
/// release raced the next acquire in the same session (previously papered over
/// with re-entrancy).
pub struct WorkspaceWriteLock {
    locks: std::sync::Mutex<HashMap<ResourceKey, LockInfo>>,
}

impl WorkspaceWriteLock {
    pub fn new() -> Self {
        Self {
            locks: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Try to acquire a set of resource locks.
    ///
    /// **All-or-nothing:** every requested key must be free, otherwise
    /// `Err(holder session_id)` and **no** lock is taken.
    ///
    /// **Same-session re-entrant:** requesting a key this session already holds
    /// succeeds.
    ///
    /// **Coarse-lock exclusion:** `ResourceKey::Workspace` conflicts with any
    /// held lock (Workspace or File) and vice versa. Distinct `File` keys do
    /// not conflict. Production tools only acquire File keys.
    pub fn try_acquire(&self, keys: &[ResourceKey], session_id: &str) -> Result<(), String> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());

        // Pass 1: detect conflicts.
        // A requested key conflicts with a held lock when:
        //   - the same key is held by another session (same File or Workspace)
        //   - either side is the Workspace coarse lock
        for key in keys {
            for (held, info) in locks.iter() {
                if info.session_id == session_id {
                    continue;
                }
                let conflict = key == held
                    || matches!(key, ResourceKey::Workspace)
                    || matches!(held, ResourceKey::Workspace);
                if conflict {
                    return Err(info.session_id.clone());
                }
            }
        }

        // Pass 2: no conflicts; acquire idle keys (keys this session already holds stay).
        for key in keys {
            locks.entry(key.clone()).or_insert(LockInfo {
                session_id: session_id.to_string(),
            });
        }

        Ok(())
    }

    /// Release every lock held by this session.
    ///
    /// Only locks owned by `session_id` are dropped. Callers that hold locks
    /// across calls must use this session's id — this method does not care how
    /// the lock was acquired.
    pub fn release_all(&self, session_id: &str) {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.retain(|_key, info| info.session_id != session_id);
    }
}

impl Default for WorkspaceWriteLock {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide singleton write lock.
///
/// Cross-session exclusion requires every turn to share one lock. That used to
/// be implicit (serve built one `RuntimeHandle` and cloned it). It is now a
/// real process singleton: any number of `RuntimeHandle` / `RuntimeContext`
/// values share the same `Arc<WorkspaceWriteLock>`, so a refactor cannot
/// silently create a second lock and drop exclusion.
static PROCESS_WRITE_LOCK: LazyLock<Arc<WorkspaceWriteLock>> =
    LazyLock::new(|| Arc::new(WorkspaceWriteLock::new()));

/// Handle to the process-wide write lock (shared by all `RuntimeContext`s).
pub fn process_write_lock() -> Arc<WorkspaceWriteLock> {
    PROCESS_WRITE_LOCK.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_release() {
        let lock = WorkspaceWriteLock::new();
        assert!(
            lock.try_acquire(&[ResourceKey::File("/a".into())], "s1")
                .is_ok()
        );
        lock.release_all("s1");
        // After release, another session can acquire.
        assert!(
            lock.try_acquire(&[ResourceKey::File("/a".into())], "s2")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_conflict_returns_holder() {
        let lock = WorkspaceWriteLock::new();
        lock.try_acquire(&[ResourceKey::File("/a".into())], "s1")
            .unwrap();
        let err = lock
            .try_acquire(&[ResourceKey::File("/a".into())], "s2")
            .unwrap_err();
        assert_eq!(err, "s1");
    }

    #[tokio::test]
    async fn test_no_partial_acquire() {
        let lock = WorkspaceWriteLock::new();
        lock.try_acquire(&[ResourceKey::File("/a".into())], "s1")
            .unwrap();
        // Two keys, one conflicting → acquire none of them.
        let err = lock
            .try_acquire(
                &[
                    ResourceKey::File("/a".into()),
                    ResourceKey::File("/b".into()),
                ],
                "s2",
            )
            .unwrap_err();
        assert_eq!(err, "s1");
        // /b should still be free
        assert!(
            lock.try_acquire(&[ResourceKey::File("/b".into())], "s3")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_reentrant_same_session() {
        let lock = WorkspaceWriteLock::new();
        lock.try_acquire(&[ResourceKey::File("/a".into())], "s1")
            .unwrap();
        // Same-session re-entry should succeed
        assert!(
            lock.try_acquire(&[ResourceKey::File("/a".into())], "s1")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_workspace_coarse_lock() {
        let lock = WorkspaceWriteLock::new();
        lock.try_acquire(&[ResourceKey::Workspace], "s1").unwrap();
        // Other session bash is blocked
        assert!(lock.try_acquire(&[ResourceKey::Workspace], "s2").is_err());
        // Other session write/edit is also blocked (workspace coarse lock)
        assert!(
            lock.try_acquire(&[ResourceKey::File("/x".into())], "s2")
                .is_err()
        );
    }
}
