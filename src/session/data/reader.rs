//! Read-only executor. Short-lived idle `query_only` connections, pooled.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

use super::command::{ReadValue, SessionRead};
use super::sqlite::{conn, read as sql_read};

/// How many idle read connections one workspace keeps warm.
///
/// A pool slot costs one SQLite handle plus its page cache, and the whole point
/// of pooling is to reuse that per-connection state instead of re-opening (and
/// re-warming) for every query. Twelve covers 20 parallel sessions on a normal
/// machine while staying far below any file-descriptor ceiling.
pub const MAX_IDLE_CONNS: usize = 12;

/// One idle read connection plus the data root its queries need.
///
/// Dropping the pool drops every idle connection, which is what lets a Windows
/// tempdir be removed after the last reader goes away.
struct PoolInner {
    path: PathBuf,
    data_root: PathBuf,
    idle: Mutex<Vec<Connection>>,
}

/// Read-only face over `sessions.db`.
///
/// Cheap to clone: every clone shares one set of idle connections.
#[derive(Clone)]
pub struct SessionReadPool {
    inner: Arc<PoolInner>,
}

impl SessionReadPool {
    pub fn new(path: PathBuf) -> Self {
        let data_root = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            inner: Arc::new(PoolInner {
                path,
                data_root,
                idle: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.inner.path
    }

    /// Drop every cached read-only handle so callers shutting down the store
    /// (e.g. workspace switch or DB removal on Windows) hold no open file locks.
    pub fn clear_idle(&self) {
        self.inner
            .idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub async fn execute(&self, query: SessionRead) -> Result<ReadValue> {
        let pool = self.clone();
        tokio::task::spawn_blocking(move || pool.execute_blocking(query))
            .await
            .map_err(|e| LitecodeError::SessionStorage(format!("read join: {e}")))?
    }

    /// Run one read on the calling thread.
    ///
    /// Never blocks on a lock while executing: the pool mutex is held only long
    /// enough to pop or push a connection, so concurrent readers only contend on
    /// SQLite itself.
    pub fn execute_blocking(&self, query: SessionRead) -> Result<ReadValue> {
        let conn = self.checkout()?;
        match sql_read::execute(&conn, query, &self.inner.data_root) {
            Ok(value) => {
                self.checkin(conn);
                Ok(value)
            }
            // A connection that produced an error may be in an unknown state
            // (interrupted statement, corrupted page cache). Never hand it to
            // another query: drop it and let the next checkout open a fresh one.
            Err(error) => Err(error),
        }
    }

    fn checkout(&self) -> Result<Connection> {
        if let Some(conn) = self
            .inner
            .idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop()
        {
            return Ok(conn);
        }
        conn::open_readonly(&self.inner.path)
    }

    fn checkin(&self, conn: Connection) {
        let mut idle = self.inner.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < MAX_IDLE_CONNS {
            idle.push(conn);
        }
        // Beyond the cap the connection is simply dropped here, closing it.
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_IDLE_CONNS, SessionReadPool};
    use crate::session::data::command::{ReadValue, SessionRead};
    use crate::session::data::sqlite::conn::{SharedDb, open_readonly};

    fn pool_with_session() -> (tempfile::TempDir, SessionReadPool, String) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let writer = SharedDb::open_rw(&path).expect("open rw");
        writer
            .conn()
            .execute_batch(
                "INSERT INTO sessions (id, project, created_at, updated_at, agent_id)
                 VALUES ('s1', '/p', 0, 0, 'default');",
            )
            .expect("insert session");
        let pool = SessionReadPool::new(path);
        (dir, pool, "s1".to_string())
    }

    #[test]
    fn a_successful_read_returns_its_connection_to_the_pool() {
        let (_dir, pool, session_id) = pool_with_session();
        let read = || {
            pool.execute_blocking(SessionRead::Revision {
                session_id: session_id.clone(),
            })
            .expect("read")
        };
        assert!(matches!(read(), ReadValue::Revision(0)));
        assert_eq!(
            pool.inner.idle.lock().unwrap().len(),
            1,
            "a pooled read must keep its connection for the next query"
        );
        assert!(
            matches!(read(), ReadValue::Revision(0)),
            "a reused read connection must serve the next query"
        );
        assert_eq!(pool.inner.idle.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_pool_never_grows_past_its_cap() {
        let (_dir, pool, session_id) = pool_with_session();
        // Fill the idle slots directly: a checkout/checkin cycle is serial, so
        // it can only ever produce one idle connection.
        let mut held = Vec::new();
        for _ in 0..MAX_IDLE_CONNS {
            held.push(open_readonly(pool.path()).expect("open ro"));
        }
        *pool.inner.idle.lock().unwrap() = held;
        let _ = pool
            .execute_blocking(SessionRead::Revision { session_id })
            .expect("read");
        assert_eq!(
            pool.inner.idle.lock().unwrap().len(),
            MAX_IDLE_CONNS,
            "a checkin past the cap must drop the connection, not grow the pool"
        );
    }

    #[test]
    fn a_failed_read_is_not_returned_to_the_pool() {
        let (_dir, pool, _session_id) = pool_with_session();
        // Unknown session → a real SQLite-level read error.
        assert!(
            pool.execute_blocking(SessionRead::Revision {
                session_id: "missing".into(),
            })
            .is_err()
        );
        assert!(
            pool.inner.idle.lock().unwrap().is_empty(),
            "a connection that errored must be dropped, never pooled"
        );
    }

    /// A pooled connection must see commits made after it was warmed.
    ///
    /// Reuse is only safe if a connection is not pinned to the snapshot it was
    /// opened in; this pins that assumption for the pool, not just for a
    /// hand-opened connection.
    #[test]
    fn a_pooled_connection_sees_later_commits() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let writer = SharedDb::open_rw(&path).expect("open rw");
        writer
            .conn()
            .execute_batch("CREATE TABLE probe (x INTEGER NOT NULL);")
            .expect("create probe");
        let pool = SessionReadPool::new(path);
        // Warm the single idle slot.
        let _ = pool
            .execute_blocking(SessionRead::ListSessionIds)
            .expect("warm read");
        writer
            .conn()
            .execute_batch("INSERT INTO probe VALUES (1);")
            .expect("insert");
        let conn = pool.checkout().expect("checkout");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1, "a reused read connection must see a later commit");
    }
}
