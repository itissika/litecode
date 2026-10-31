//! Shared retrieval progress record (WP-131 frozen interface).
//!
//! One [`CorpusProgress`] per corpus (code / session) and a merged `total`,
//! serialized verbatim as `retrieval: {total, code, session}` in
//! `GET /api/workspace/engines/detail`. Field names and enum values are the
//! frozen contract — do not rename. Session (WP-135) builds its own record with
//! the constructors below; merging happens only through [`RetrievalProgress::merge`].
//!
//! Invariants (enforced by the constructors):
//! * `phase.is_some()` ⇔ `state == Refreshing`
//! * `chunks_total.is_none()` ⇔ `phase == Some(Scanning)` (per corpus)
//! * `Embedding` ⇒ `chunks_total > 0` and `chunks_done <= chunks_total`
//! * `error.is_some()` ⇔ `state == Failed`

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorpusState {
    Ready,
    Refreshing,
    NeedsRebuild,
    Failed,
}

impl CorpusState {
    /// Merge priority: failed > needs_rebuild > refreshing > ready.
    fn rank(self) -> u8 {
        match self {
            CorpusState::Failed => 3,
            CorpusState::NeedsRebuild => 2,
            CorpusState::Refreshing => 1,
            CorpusState::Ready => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorpusPhase {
    Scanning,
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressEngine {
    Code,
    Session,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    ModelMissing,
    EngineUnavailable,
    EngineStopped,
    IndexFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusError {
    pub engine: ProgressEngine,
    pub reason: FailReason,
    pub message: String,
}

impl CorpusError {
    pub fn new(engine: ProgressEngine, reason: FailReason, message: impl Into<String>) -> Self {
        Self {
            engine,
            reason,
            message: message.into(),
        }
    }
}

/// Progress of one corpus, or the merged `total`. Fields are the JSON contract;
/// production code builds values only through the constructors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusProgress {
    pub state: CorpusState,
    pub phase: Option<CorpusPhase>,
    pub chunks_done: u64,
    pub chunks_total: Option<u64>,
    /// Chunks known to be waiting (exact only; 0 when only file counts are known).
    pub chunks_pending: u64,
    /// Files actually waiting to be processed. Not part of `chunks_total`.
    #[serde(default)]
    pub files_pending: u64,
    pub error: Option<CorpusError>,
}

impl CorpusProgress {
    /// Ready. `chunks_total == chunks_done`; `pending > 0` ⇒ UI orange.
    pub fn ready(chunks_done: u64, chunks_pending: u64) -> Self {
        Self {
            state: CorpusState::Ready,
            phase: None,
            chunks_done,
            chunks_total: Some(chunks_done),
            chunks_pending,
            files_pending: 0,
            error: None,
        }
    }

    /// Chunking in progress, total not yet known. `chunks_done` counts chunks
    /// already in the index (denominator = whole index).
    pub fn scanning(chunks_done: u64, chunks_pending: u64) -> Self {
        Self {
            state: CorpusState::Refreshing,
            phase: Some(CorpusPhase::Scanning),
            chunks_done,
            chunks_total: None,
            chunks_pending,
            files_pending: 0,
            error: None,
        }
    }

    /// Embedding with an exact total. Total is at least 1; done is clamped.
    pub fn embedding(chunks_done: u64, chunks_total: u64, chunks_pending: u64) -> Self {
        let total = chunks_total.max(1);
        Self {
            state: CorpusState::Refreshing,
            phase: Some(CorpusPhase::Embedding),
            chunks_done: chunks_done.min(total),
            chunks_total: Some(total),
            chunks_pending,
            files_pending: 0,
            error: None,
        }
    }

    /// Index must be rebuilt before it is usable.
    pub fn needs_rebuild(chunks_done: u64, chunks_total: u64) -> Self {
        Self {
            state: CorpusState::NeedsRebuild,
            phase: None,
            chunks_done: chunks_done.min(chunks_total),
            chunks_total: Some(chunks_total),
            chunks_pending: 0,
            files_pending: 0,
            error: None,
        }
    }

    /// Failed (model missing / engine unavailable / stopped / index failed).
    pub fn failed(error: CorpusError, chunks_done: u64) -> Self {
        Self {
            state: CorpusState::Failed,
            phase: None,
            chunks_done,
            chunks_total: Some(chunks_done),
            chunks_pending: 0,
            files_pending: 0,
            error: Some(error),
        }
    }

    /// Set the number of files waiting (does not touch chunk counts).
    pub fn with_files_pending(mut self, files_pending: u64) -> Self {
        self.files_pending = files_pending;
        self
    }

    /// Any queued work (exact chunks or files).
    pub fn has_pending(&self) -> bool {
        self.chunks_pending > 0 || self.files_pending > 0
    }

    /// 0..=100 from exact counts. `None` while the total is unknown (scanning).
    /// In flight never claims 100; only ready with nothing pending is 100.
    pub fn percent(&self) -> Option<u8> {
        match self.state {
            CorpusState::Failed => Some(0),
            CorpusState::Ready if !self.has_pending() => Some(100),
            // Pending files with unknown chunk counts: not synced, never 100.
            CorpusState::Ready if self.chunks_pending == 0 => Some(99),
            CorpusState::Ready => {
                let denom = self.chunks_done + self.chunks_pending;
                Some(ratio(self.chunks_done, denom).min(99))
            }
            CorpusState::Refreshing | CorpusState::NeedsRebuild => {
                let total = self.chunks_total?;
                Some(ratio(self.chunks_done, total).min(99))
            }
        }
    }
}

fn ratio(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    ((done.min(total) as u128 * 100) / total as u128) as u8
}

/// Merge corpora into `total`. See module doc / WP-131-backend.md.
pub fn merge_total(parts: &[&CorpusProgress]) -> CorpusProgress {
    let state = parts
        .iter()
        .map(|p| p.state)
        .max_by_key(|s| s.rank())
        .unwrap_or(CorpusState::Ready);
    let chunks_done = parts.iter().map(|p| p.chunks_done).sum();
    let chunks_pending = parts.iter().map(|p| p.chunks_pending).sum();
    let files_pending = parts.iter().map(|p| p.files_pending).sum();
    let chunks_total = parts
        .iter()
        .map(|p| p.chunks_total)
        .try_fold(0u64, |acc, t| t.map(|t| acc + t));
    let phase = (state == CorpusState::Refreshing).then(|| {
        if parts.iter().any(|p| p.phase == Some(CorpusPhase::Scanning)) {
            CorpusPhase::Scanning
        } else {
            CorpusPhase::Embedding
        }
    });
    let error = (state == CorpusState::Failed)
        .then(|| parts.iter().find_map(|p| p.error.clone()))
        .flatten();
    CorpusProgress {
        state,
        phase,
        chunks_done,
        chunks_total,
        chunks_pending,
        files_pending,
        error,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalProgress {
    pub total: CorpusProgress,
    pub code: CorpusProgress,
    pub session: CorpusProgress,
}

impl RetrievalProgress {
    pub fn merge(code: CorpusProgress, session: CorpusProgress) -> Self {
        let total = merge_total(&[&code, &session]);
        Self {
            total,
            code,
            session,
        }
    }
}

#[cfg(test)]
mod tests {}
