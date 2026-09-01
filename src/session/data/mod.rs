//! Session persistence owner: one writer actor, bounded read pool, blob store.

mod blob;
pub mod command;
mod reader;
pub(crate) mod sqlite;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::session::workspace_lock::WorkspaceWriteLease;
use crate::types::{LitecodeError, Result};

pub use blob::{gc_unreferenced, put_bytes, put_text, read_bytes};
pub use command::{
    CommitKind, CommitReceipt, MutationId, ReadValue, SessionChange, SessionMutation, SessionRead,
    SessionRevision,
};
pub use reader::SessionReadPool;
pub use sqlite::fts;
pub use writer::{FaultKind, WRITER_QUEUE_CAPACITY};
use writer::{WriterHandle, WriterHooks};

/// Read/write owner of `sessions.db` for one workspace.
pub struct SessionData {
    path: PathBuf,
    data_root: PathBuf,
    writer: WriterHandle,
    reader: SessionReadPool,
}

/// Read-only face. Cannot be upgraded to a writer.
#[derive(Clone)]
pub struct SessionDataReader {
    reader: SessionReadPool,
    data_root: PathBuf,
}

/// Boundary data for an isolated read-only worker.
#[derive(Clone)]
pub struct SessionDataReaderConfig {
    path: PathBuf,
}

impl SessionData {
    pub fn open(_lease: &WorkspaceWriteLease, db_path: &Path) -> Result<Arc<Self>> {
        Self::open_inner(db_path)
    }

    pub fn open_ephemeral() -> Result<Arc<Self>> {
        let (writer, path) = WriterHandle::spawn_ephemeral()?;
        Ok(Arc::new(Self {
            path: path.clone(),
            data_root: std::env::temp_dir().join("litecode"),
            reader: SessionReadPool::new(path),
            writer,
        }))
    }

    fn open_inner(db_path: &Path) -> Result<Arc<Self>> {
        let data_root = db_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let writer = WriterHandle::spawn(db_path, data_root.clone())?;
        Ok(Arc::new(Self {
            path: db_path.to_path_buf(),
            data_root,
            reader: SessionReadPool::new(db_path.to_path_buf()),
            writer,
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn reader(&self) -> SessionDataReader {
        SessionDataReader {
            reader: self.reader.clone(),
            data_root: self.data_root.clone(),
        }
    }

    pub fn hooks(&self) -> Arc<WriterHooks> {
        Arc::clone(&self.writer.hooks)
    }

    pub async fn mutate(&self, mutation: SessionMutation) -> Result<CommitReceipt> {
        self.writer.submit(mutation).await
    }

    pub fn mutate_blocking(&self, mutation: SessionMutation) -> Result<CommitReceipt> {
        self.writer.submit_blocking(mutation)
    }

    pub fn try_mutate(
        &self,
        mutation: SessionMutation,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<CommitReceipt>>> {
        self.writer.try_submit_nowait(mutation)
    }

    pub async fn read(&self, query: SessionRead) -> Result<ReadValue> {
        self.reader.execute(query).await
    }

    pub fn read_blocking(&self, query: SessionRead) -> Result<ReadValue> {
        self.reader.execute_blocking(query)
    }

    pub fn revision_blocking(&self, session_id: &str) -> Result<u64> {
        match self.read_blocking(SessionRead::Revision {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Revision(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected revision read".into(),
            )),
        }
    }

    pub fn working_set_blocking(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::session::working::WorkingRow>> {
        match self.read_blocking(SessionRead::WorkingSet {
            session_id: session_id.to_string(),
        })? {
            ReadValue::WorkingSet(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected working set".into(),
            )),
        }
    }

    pub fn events_blocking(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::session::event::SessionEvent>> {
        match self.read_blocking(SessionRead::Events {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Events(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected events".into())),
        }
    }

    pub fn events_range_blocking(
        &self,
        session_id: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<crate::session::event::SessionEvent>> {
        match self.read_blocking(SessionRead::EventsRange {
            session_id: session_id.to_string(),
            from,
            to,
        })? {
            ReadValue::Events(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected events".into())),
        }
    }

    pub fn meta_blocking(&self, session_id: &str) -> Result<crate::session::model::SessionMeta> {
        match self.read_blocking(SessionRead::Meta {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Meta(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected meta".into())),
        }
    }

    pub fn meter_blocking(
        &self,
        session_id: &str,
    ) -> Result<crate::session::store::SessionContextMeter> {
        match self.read_blocking(SessionRead::ContextMeter {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Meter(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected meter".into())),
        }
    }

    pub fn transcript_blocking(&self, session_id: &str) -> Result<crate::types::Transcript> {
        match self.read_blocking(SessionRead::Transcript {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Transcript(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected transcript".into(),
            )),
        }
    }

    pub fn list_sessions_blocking(
        &self,
    ) -> Result<Vec<(String, String, i64, String, String, Option<String>)>> {
        match self.read_blocking(SessionRead::ListSessions)? {
            ReadValue::List(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected list".into())),
        }
    }

    pub fn list_session_ids_blocking(&self) -> Result<Vec<String>> {
        match self.read_blocking(SessionRead::ListSessionIds)? {
            ReadValue::Ids(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected ids".into())),
        }
    }

    pub fn list_gc_blocking(&self) -> Result<Vec<(String, i64)>> {
        match self.read_blocking(SessionRead::ListSessionsForGc)? {
            ReadValue::GcList(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected gc list".into())),
        }
    }

    pub fn checkpoint_seq_blocking(&self, session_id: &str) -> Result<i64> {
        match self.read_blocking(SessionRead::CheckpointSeq {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Count(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected checkpoint".into(),
            )),
        }
    }

    pub fn create_session(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
    ) -> Result<String> {
        Ok(self
            .mutate_blocking(SessionMutation::Create {
                operation_id: MutationId::new(),
                project: project.to_string(),
                agent_id: agent_id.to_string(),
                model_id: model_id.map(str::to_string),
                parent_session_id: None,
                parent_call_id: None,
            })?
            .session_id)
    }

    pub fn insert_items(&self, session_id: &str, items: &[crate::types::Item]) -> Result<()> {
        let expected = self.revision_blocking(session_id)?;
        self.mutate_blocking(SessionMutation::InsertDetails {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            items: items.to_vec(),
            turn_id: String::new(),
        })?;
        Ok(())
    }

    pub fn compact_from(
        &self,
        session_id: &str,
        summary: &crate::types::Item,
        kept_from: Option<i64>,
        token_estimate: i64,
    ) -> Result<()> {
        let expected = self.revision_blocking(session_id)?;
        self.mutate_blocking(SessionMutation::Compact {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            summary: summary.clone(),
            token_estimate,
            kept_from: kept_from.map(|s| s as crate::session::event::Seq),
            expected_prefix: None,
        })?;
        Ok(())
    }

    pub fn list_child_ids_blocking(&self, parent_session_id: &str) -> Result<Vec<String>> {
        match self.read_blocking(SessionRead::ListChildIds {
            parent_session_id: parent_session_id.to_string(),
        })? {
            ReadValue::Ids(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected child ids".into())),
        }
    }

    pub fn shutdown(&self) {
        self.writer.shutdown();
    }
}

impl SessionDataReader {
    pub fn open(db_path: &Path) -> Self {
        let data_root = db_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            reader: SessionReadPool::new(db_path.to_path_buf()),
            data_root,
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Identity of the owner-provided read database.  Consumers may forward
    /// this to an isolated read-only worker but must not derive it from a
    /// workspace root.
    pub fn path(&self) -> &Path {
        self.reader.path()
    }

    pub fn worker_config(&self) -> SessionDataReaderConfig {
        SessionDataReaderConfig {
            path: self.path().to_path_buf(),
        }
    }

    pub fn from_worker_config(config: SessionDataReaderConfig) -> Self {
        Self::open(&config.path)
    }

    pub async fn read(&self, query: SessionRead) -> Result<ReadValue> {
        self.reader.execute(query).await
    }

    pub fn read_blocking(&self, query: SessionRead) -> Result<ReadValue> {
        self.reader.execute_blocking(query)
    }

    pub fn list_session_ids_blocking(&self) -> Result<Vec<String>> {
        match self.read_blocking(SessionRead::ListSessionIds)? {
            ReadValue::Ids(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected ids".into())),
        }
    }

    pub fn list_sessions_blocking(
        &self,
    ) -> Result<Vec<(String, String, i64, String, String, Option<String>)>> {
        match self.read_blocking(SessionRead::ListSessions)? {
            ReadValue::List(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected list".into())),
        }
    }

    pub fn meta_blocking(&self, session_id: &str) -> Result<crate::session::model::SessionMeta> {
        match self.read_blocking(SessionRead::Meta {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Meta(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected meta".into())),
        }
    }

    pub fn resolve_ref_blocking(&self, refer: &str) -> Result<Option<String>> {
        match self.read_blocking(SessionRead::ResolveRef {
            refer: refer.to_string(),
        })? {
            ReadValue::OptionalId(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected resolve".into())),
        }
    }

    pub fn surface_seqs_blocking(&self, session_id: &str) -> Result<Vec<i64>> {
        match self.read_blocking(SessionRead::SurfaceSeqs {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Seqs(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected seqs".into())),
        }
    }

    pub fn searchable_rows_blocking(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::session::transcript_file::SearchableRow>> {
        match self.read_blocking(SessionRead::SearchableRows {
            session_id: session_id.map(str::to_string),
        })? {
            ReadValue::Searchable(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected searchable".into(),
            )),
        }
    }

    pub fn fts_search_blocking(
        &self,
        query: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, i64, String)>> {
        match self.read_blocking(SessionRead::FtsSearch {
            query: query.to_string(),
            session_id: session_id.map(str::to_string),
            limit,
        })? {
            ReadValue::FtsHits(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected fts".into())),
        }
    }

    pub fn change_log_since_blocking(
        &self,
        last_change_id: i64,
    ) -> Result<Vec<crate::session::data::command::SessionChange>> {
        match self.read_blocking(SessionRead::ChangeLogSince { last_change_id })? {
            ReadValue::Changes(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected changes".into())),
        }
    }

    pub fn latest_change_id_blocking(&self) -> Result<i64> {
        match self.read_blocking(SessionRead::LatestChangeId)? {
            ReadValue::Count(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected change id".into())),
        }
    }

    pub fn list_child_ids_blocking(&self, parent_session_id: &str) -> Result<Vec<String>> {
        match self.read_blocking(SessionRead::ListChildIds {
            parent_session_id: parent_session_id.to_string(),
        })? {
            ReadValue::Ids(v) => Ok(v),
            _ => Err(LitecodeError::SessionStorage("unexpected child ids".into())),
        }
    }

    pub fn transcript_file_blocking(
        &self,
        session_id: &str,
    ) -> Result<crate::session::transcript_file::TranscriptFile> {
        let rows = self.searchable_rows_blocking(Some(session_id))?;
        crate::session::transcript_file::load_transcript_file(session_id, &rows, &self.data_root)
    }
}

impl SessionDataReaderConfig {
    pub fn from_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for SessionDataReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDataReader")
            .field("path", &self.reader.path())
            .finish()
    }
}

impl Drop for SessionData {
    fn drop(&mut self) {
        self.shutdown();
    }
}
