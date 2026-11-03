//! Read-only executor. Short-lived `query_only` connections only.

use std::path::PathBuf;

use crate::types::{LitecodeError, Result};

use super::command::{ReadValue, SessionRead};
use super::sqlite::{conn, read as sql_read};

#[derive(Clone)]
pub struct SessionReadPool {
    path: PathBuf,
    data_root: PathBuf,
}

impl SessionReadPool {
    pub fn new(path: PathBuf) -> Self {
        let data_root = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self { path, data_root }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub async fn execute(&self, query: SessionRead) -> Result<ReadValue> {
        let path = self.path.clone();
        let data_root = self.data_root.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn::open_readonly(&path)?;
            sql_read::execute(&conn, query, &data_root)
        })
        .await
        .map_err(|e| LitecodeError::SessionStorage(format!("read join: {e}")))?
    }

    pub fn execute_blocking(&self, query: SessionRead) -> Result<ReadValue> {
        let conn = conn::open_readonly(&self.path)?;
        sql_read::execute(&conn, query, &self.data_root)
    }
}
