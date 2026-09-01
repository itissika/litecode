//! Bounded read-only executor. Short-lived `query_only` connections only.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::types::{LitecodeError, Result};

use super::command::{ReadValue, SessionRead};
use super::sqlite::{conn, read as sql_read};

const READ_CONCURRENCY: usize = 4;

#[derive(Clone)]
pub struct SessionReadPool {
    path: PathBuf,
    data_root: PathBuf,
    limit: Arc<Semaphore>,
}

impl SessionReadPool {
    pub fn new(path: PathBuf) -> Self {
        let data_root = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            path,
            data_root,
            limit: Arc::new(Semaphore::new(READ_CONCURRENCY)),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub async fn execute(&self, query: SessionRead) -> Result<ReadValue> {
        let path = self.path.clone();
        let data_root = self.data_root.clone();
        let permit = self
            .limit
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| LitecodeError::SessionDataClosed)?;
        let result = tokio::task::spawn_blocking(move || {
            let conn = conn::open_readonly(&path)?;
            sql_read::execute(&conn, query, &data_root)
        })
        .await
        .map_err(|e| LitecodeError::SessionStorage(format!("read join: {e}")))?;
        drop(permit);
        result
    }

    pub fn execute_blocking(&self, query: SessionRead) -> Result<ReadValue> {
        let permit = loop {
            match self.limit.try_acquire() {
                Ok(permit) => break permit,
                Err(tokio::sync::TryAcquireError::Closed) => {
                    return Err(LitecodeError::SessionDataClosed);
                }
                Err(tokio::sync::TryAcquireError::NoPermits) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        };
        let conn = conn::open_readonly(&self.path)?;
        let result = sql_read::execute(&conn, query, &self.data_root);
        drop(permit);
        result
    }
}
