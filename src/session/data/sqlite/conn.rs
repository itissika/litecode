//! Shared write connection. Lives only on the SessionData writer thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::types::{LitecodeError, Result};

use super::schema;

pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SharedDb {
    conn: Connection,
    path: PathBuf,
}

impl SharedDb {
    pub fn open_rw(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .map_err(|e| LitecodeError::SessionStorage(format!("open sessions.db: {e}")))?;
        schema::configure_write(&conn)?;
        schema::ensure_session_schema(&conn)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn open_ephemeral() -> Result<Self> {
        Self::open_shared_memory(&format!(
            "file:session-mem-{}?mode=memory&cache=shared",
            ulid::Ulid::new()
        ))
    }

    /// Shared-cache in-memory URI so the writer and read pool share one DB.
    pub fn open_shared_memory(uri: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| {
            LitecodeError::SessionStorage(format!("open shared-memory sessions.db: {e}"))
        })?;
        schema::configure_write(&conn)?;
        schema::ensure_session_schema(&conn)?;
        Ok(Self {
            conn,
            path: PathBuf::from(uri),
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| LitecodeError::SessionStorage(format!("wal checkpoint: {e}")))?;
        Ok(())
    }
}

pub fn open_readonly(path: &Path) -> Result<Connection> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    if path
        .to_str()
        .is_some_and(|s| s.starts_with("file:") || s == ":memory:")
    {
        flags |= OpenFlags::SQLITE_OPEN_URI;
    }
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| LitecodeError::SessionStorage(format!("open sessions.db read-only: {e}")))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| LitecodeError::SessionStorage(format!("read busy_timeout: {e}")))?;
    conn.execute_batch("PRAGMA query_only=ON;")
        .map_err(|e| LitecodeError::SessionStorage(format!("query_only: {e}")))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::{SharedDb, open_readonly};

    /// A reused read connection must see commits made after it was opened.
    ///
    /// The read pool opens one connection per query today. Reusing them is only
    /// equivalent if a connection is not stuck on the snapshot it was opened in;
    /// this pins that assumption instead of trusting it.
    #[test]
    fn a_reused_read_connection_sees_later_commits() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let writer = SharedDb::open_rw(&path).expect("open rw");
        writer
            .conn()
            .execute_batch("CREATE TABLE probe (x INTEGER NOT NULL);")
            .expect("create probe");

        let reader = open_readonly(&path).expect("open ro");
        let count = |conn: &rusqlite::Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
                .expect("count")
        };
        assert_eq!(count(&reader), 0);

        writer
            .conn()
            .execute_batch("INSERT INTO probe VALUES (1);")
            .expect("insert");

        assert_eq!(
            count(&reader),
            1,
            "the same read connection must observe a later commit"
        );
    }
}
