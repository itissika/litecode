use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// Resource key: a file path, or the workspace-wide coarse lock.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKey {
    File(String),
    /// Coarse workspace lock. Production bash no longer acquires this.
    Workspace,
}

#[derive(Debug, Clone)]
pub struct LockInfo {
    pub session_id: String,
    /// How many acquires this session still owes a release for. One call takes
    /// its keys exactly once, but a sibling nested call of the same session may
    /// re-enter a key, so a single `release_keys` must not drop the key early.
    pub holds: usize,
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
    /// succeeds and increments that key's hold count, so the key survives a
    /// sibling call's [`Self::release_keys`].
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

        // Pass 2: no conflicts. A key this session already holds keeps its
        // owner and counts one more hold; an idle key is taken for the first time.
        for key in keys {
            match locks.get_mut(key) {
                Some(info) if info.session_id == session_id => info.holds += 1,
                _ => {
                    locks.insert(
                        key.clone(),
                        LockInfo {
                            session_id: session_id.to_string(),
                            holds: 1,
                        },
                    );
                }
            }
        }

        Ok(())
    }

    /// Release the keys one call acquired, and only those.
    ///
    /// Every call must release exactly what it took: a nested call finishing in
    /// the middle of a `run_code` program must not free a sibling call's file.
    /// A key stays held until its last hold is released.
    pub fn release_keys(&self, session_id: &str, keys: &[ResourceKey]) {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            let last_hold = match locks.get_mut(key) {
                Some(info) if info.session_id == session_id => {
                    info.holds = info.holds.saturating_sub(1);
                    info.holds == 0
                }
                _ => false,
            };
            if last_hold {
                locks.remove(key);
            }
        }
    }

    /// Release every lock held by this session.
    ///
    /// Only locks owned by `session_id` are dropped, regardless of hold count.
    /// This is the turn/session cleanup escape hatch, not the per-call release:
    /// tool calls use [`Self::release_keys`].
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
}
