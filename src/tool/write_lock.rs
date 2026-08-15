use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// 资源键：文件路径 或 特殊的 workspace 键
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKey {
    File(String), // 文件路径
    Workspace,    // workspace 粗锁（bash 使用）
}

#[derive(Debug, Clone)]
pub struct LockInfo {
    pub session_id: String,
}

/// 进程级写锁注册表。
///
/// 用于实现 DESIGN §1.3「资源级粗暴挡」：跨 session 的写互斥。
/// - write/edit 工具按文件路径加锁
/// - bash 工具使用 [`ResourceKey::Workspace`] 粗锁，与所有其他写工具互斥
///
/// 底层为同步 `std::sync::Mutex`：临界区仅做 HashMap 读写、绝不跨
/// `.await`，因此取/放锁均可同步完成。这消除了原先基于 `tokio` 异步锁
/// + `Drop` 内 `spawn` 释放带来的两个问题：(1) 若 drop 发生时无 current
/// runtime 则释放被静默丢弃导致锁泄漏；(2) 释放 fire-and-forget 与同
/// session 下一次取锁竞态（原先只能靠可重入短路兜底）。
pub struct WorkspaceWriteLock {
    locks: std::sync::Mutex<HashMap<ResourceKey, LockInfo>>,
}

impl WorkspaceWriteLock {
    pub fn new() -> Self {
        Self {
            locks: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// 尝试获取一组资源锁。
    ///
    /// **原子性（全有或全无）**：仅当所有请求的 key 都能获取时才获取全部；
    /// 若任一 key 冲突，返回 `Err(持有者 session_id)`，且**不会**获取任何锁。
    ///
    /// **同 session 可重入**：同一 session 重复请求同一 key 视为成功。
    ///
    /// **粗锁交叉互斥**：`ResourceKey::Workspace`（bash 使用）与任意已持有的
    /// 锁（无论是 `Workspace` 还是某个 `File`）冲突，反之亦然——即 workspace
    /// 粗锁与所有 write/edit 互斥。不同 `File` 键之间互不冲突，可并发。
    pub fn try_acquire(&self, keys: &[ResourceKey], session_id: &str) -> Result<(), String> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());

        // 第一遍：检测冲突。
        // 任意请求 key 与任意已持有锁之间，只要满足以下任一条件即冲突：
        //   - 同一具体 key 被其他 session 持有（同 File 或同 Workspace）
        //   - 任一侧为 Workspace 粗锁（bash 与所有写操作互斥）
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

        // 第二遍：无冲突，获取所有空闲的 key（同一 session 持有的 key 不变）。
        for key in keys {
            locks.entry(key.clone()).or_insert(LockInfo {
                session_id: session_id.to_string(),
            });
        }

        Ok(())
    }

    /// 释放该 session 持有的所有资源锁。
    ///
    /// 注意：只释放「由该 session 持有」的锁。如果是跨调用持锁的场景，
    /// 调用方应保证只用本 session 的锁——本方法不区分锁是如何获取的，
    /// 只要当前锁属于该 session 就释放。
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

/// 进程级单例写锁。
///
/// 跨 session 写互斥的正确性**依赖**全进程所有 turn 共享同一把锁。原先该
/// 不变量是隐式的（serve 只建一个 `RuntimeHandle` 再 clone）。这里提升为
/// 真正的进程级单例：无论创建多少个 `RuntimeHandle`/`RuntimeContext`，
/// 取到的都是同一个 `Arc<WorkspaceWriteLock>`，从根上杜绝「各建一把锁 →
/// 跨 session 互斥静默失效」的重构风险。
static PROCESS_WRITE_LOCK: LazyLock<Arc<WorkspaceWriteLock>> =
    LazyLock::new(|| Arc::new(WorkspaceWriteLock::new()));

/// 返回进程级单例写锁句柄（供所有 `RuntimeContext` 共享）。
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
        // 释放后，其他 session 可取锁
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
        // 请求两个 key，其中一个冲突 → 全都不获取
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
        // /b 仍应为空闲
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
        // 同一 session 重入应成功
        assert!(
            lock.try_acquire(&[ResourceKey::File("/a".into())], "s1")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_workspace_coarse_lock() {
        let lock = WorkspaceWriteLock::new();
        lock.try_acquire(&[ResourceKey::Workspace], "s1").unwrap();
        // 其他 session 的 bash 被挡
        assert!(lock.try_acquire(&[ResourceKey::Workspace], "s2").is_err());
        // 其他 session 的 write/edit 也被挡（workspace 粗锁互斥）
        assert!(
            lock.try_acquire(&[ResourceKey::File("/x".into())], "s2")
                .is_err()
        );
    }
}
