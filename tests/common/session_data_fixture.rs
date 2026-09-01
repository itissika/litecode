//! Real WAL `SessionData` harness for integration tests.

use std::sync::Arc;

use litecode::config::TurnGuard;
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::session::manager::SessionManager;
use litecode::session::{SessionData, SessionDataReader, WorkspaceWriteLease};
use litecode::types::Item;
use tempfile::TempDir;

pub struct SessionDataFixture {
    pub dir: TempDir,
    _lease: WorkspaceWriteLease,
    pub data: Arc<SessionData>,
}

impl SessionDataFixture {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let db = dir.path().join("sessions.db");
        let lease = WorkspaceWriteLease::acquire(dir.path()).expect("acquire workspace lease");
        let data = SessionData::open(&lease, &db).expect("open SessionData");
        Self {
            dir,
            _lease: lease,
            data,
        }
    }

    pub fn from_db_path(dir: TempDir, db_path: &std::path::Path) -> Self {
        let lease = WorkspaceWriteLease::acquire(dir.path()).expect("acquire workspace lease");
        let data = SessionData::open(&lease, db_path).expect("open SessionData");
        Self {
            dir,
            _lease: lease,
            data,
        }
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        self.data.path().to_path_buf()
    }

    pub fn manager(&self) -> SessionManager {
        SessionManager::from_data(Arc::new(TurnGuard::new()), Arc::clone(&self.data))
    }

    pub fn reader(&self) -> SessionDataReader {
        self.data.reader()
    }

    pub fn create(&self, project: &str, agent_id: &str, model_id: Option<&str>) -> String {
        self.data
            .create_session(project, agent_id, model_id)
            .expect("create session")
    }

    pub fn insert_items(&self, session_id: &str, items: &[Item]) {
        self.data
            .insert_items(session_id, items)
            .expect("insert items");
    }

    pub fn operation_id(label: &str) -> MutationId {
        MutationId(label.to_string())
    }

    pub fn mutate(&self, mutation: SessionMutation) -> litecode::session::CommitReceipt {
        self.data.mutate_blocking(mutation).expect("mutate")
    }
}

impl Default for SessionDataFixture {
    fn default() -> Self {
        Self::new()
    }
}
