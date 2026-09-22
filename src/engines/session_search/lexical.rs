//! Always-on lexical lane: the sparse FTS5 index.
//!
//! Ported from the validated eval chain (`sparse.rs` + `chunk.rs` + `echo.rs`
//! next door): exact substring → proximity → routed BM25 → n-gram fallback,
//! with `|` alternatives and an exact match span per hit. The index lives at
//! `<data_root>/session-index/sparse.db`; this module owns its lifecycle:
//! rebuild when the file is missing or incompatible, otherwise reconcile the
//! final source key set before every search.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::session::SessionDataReader;
use crate::types::{LitecodeError, Result};

use super::sparse::{self, Lane};
use super::{SessionHitLane, SessionTextHit, SessionTextQuery, filter_hits};

/// How many hits one query fetches; the page paginates over this list.
const FETCH_DEPTH: usize = 200;
/// Below this many searchable rows the index builds inline, in the search that
/// needed it. Above it the build goes to a background thread and searches are
/// reported as unanswerable until it lands.
///
/// This used to be 250_000, on a ~200ms forty-thousand-row measurement. That
/// measurement does not survive the chunked corpus: the parity fixture builds
/// 4_000 rows in seconds, and this workspace's ~50k rows measure 71 seconds and
/// a 286MB file — inside the caller's own thread, where nothing can interrupt
/// it. Inline stops where a build still finishes in a second or two.
const INLINE_BUILD_MAX_ROWS: usize = 2_000;

/// What the lane can say about whether it answered the question.
///
/// The distinction this type exists to preserve: "I searched and the corpus has
/// no such row" and "I did not search" produce the same empty list and mean
/// opposite things. A caller that sees no hits concludes the history does not
/// contain the answer and stops looking; that is the right conclusion for an
/// empty result and a permanent-looking false negative for an index that was
/// merely late.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneState {
    /// The index was current and the query ran against it. An empty hit list is
    /// now a fact about the corpus.
    Ready,
    /// No index could be produced in time to answer, so a build is running in the
    /// background. The query did **not** run.
    Building,
    /// The index could not be read, built, or reconciled. The query did **not**
    /// run.
    Failed(String),
}

impl LaneState {
    /// Why this lane did not answer, or `None` if it did.
    pub fn unanswered(&self) -> Option<String> {
        match self {
            Self::Ready => None,
            Self::Building => Some(
                "the session index is still being built, so this search did not run; \
                 retry in a moment"
                    .to_string(),
            ),
            Self::Failed(why) => Some(format!(
                "the session index could not be prepared, so this search did not run: {why}"
            )),
        }
    }
}

/// Lexical search over the sparse index. Always-on.
/// `|` separates alternatives; any alternative may match (handled in the lane).
pub fn search_lexical(
    reader: &SessionDataReader,
    query: &SessionTextQuery,
) -> Result<(Vec<SessionTextHit>, LaneState)> {
    let needle = query.query.trim();
    if needle.is_empty() {
        return Err(LitecodeError::Config(
            "session search query is required".into(),
        ));
    }

    let data_root = reader.data_root();
    let path = sparse::sparse_index_path(data_root);
    let state = prepare_index(reader, data_root);
    if state.unanswered().is_some() {
        // The query is not run. Returning the empty list here would be the bug
        // this whole type exists to prevent.
        return Ok((Vec::new(), state));
    }

    let index = sparse::open_read_only(&path)?.with_scope(query.include_session_id.as_deref());
    let hits = index.search(Lane::Final, needle, FETCH_DEPTH)?;
    let ranked: Vec<SessionTextHit> = hits
        .into_iter()
        .map(|h| SessionTextHit {
            session_id: h.session_id,
            seq: h.seq,
            item_type: h.item_type,
            summary: h.summary,
            score: h.score,
            // The exact literal span: the view turns it into a physical line via
            // `line_for_hit`, so a hit points at the match, not at the row.
            char_start: h.char_start,
            char_end: h.char_end,
            lane: SessionHitLane::Text,
        })
        .collect();
    Ok((filter_hits(ranked, query), LaneState::Ready))
}

/// Bring the index up to date, and say whether the lane is now allowed to answer.
///
/// Everything that can go wrong here is reported as a state rather than as a
/// silent empty result, including the case that used to matter most: a corpus too
/// large to build inline used to give the caller an empty list for the first few
/// searches, which reads exactly like a corpus with no matching rows.
fn prepare_index(reader: &SessionDataReader, data_root: &std::path::Path) -> LaneState {
    let path = sparse::sparse_index_path(data_root);
    // A missing or incompatible index over a corpus too large to build inline is
    // dispatched to the background thread *before* the lock below: the answer is
    // `Building` now, not after a build that runs tens of seconds in this thread.
    // Waiting for one here is what turned a rebuild into a search that hung. The
    // lock section re-checks the state, so the two can only agree.
    let must_rebuild = match sparse::needs_rebuild(&path) {
        Ok(needs) => needs,
        Err(error) => return LaneState::Failed(error.to_string()),
    };
    if must_rebuild
        && let Ok(keys) = reader.searchable_keys_blocking(None)
        && keys.len() > INLINE_BUILD_MAX_ROWS
    {
        spawn_background_build(reader.clone(), data_root.to_path_buf());
        return LaneState::Building;
    }
    let prepared = with_refresh_lock(|| -> Result<LaneState> {
        // Re-checked inside the lock, not before it. Two searches arriving during
        // one stale index would otherwise both reconcile, and the loser of the
        // race would redo work the winner had just finished. The lock makes them
        // take turns; a reconcile over an already-current index is a no-op.
        //
        // A rebuild is only for a missing or incompatible file. Everything else —
        // including "the store moved on" — is a key-set reconcile that runs on
        // every prepare before the query. There is no change-id gate: a missed
        // notification must never be able to skip the diff.
        if !sparse::needs_rebuild(&path)? {
            sparse::refresh_from_source(reader, data_root)?;
            return Ok(LaneState::Ready);
        }

        // The inline/background decision only needs how many rows there are, so it
        // asks for keys first and reads bodies only when it will actually build.
        // Only an *absent* store is an empty corpus. A store that exists but cannot
        // be read is a failed lane, not an empty one: answering "the history has
        // nothing" there would be the exact false negative this lane exists to
        // prevent, and it would overwrite a good index with an empty one.
        let keys = match reader.searchable_keys_blocking(None) {
            Ok(keys) => keys,
            Err(_) if !reader.path().is_file() => {
                sparse::build_index(&[], data_root)?;
                return Ok(LaneState::Ready);
            }
            Err(err) => return Err(err.into()),
        };

        if keys.len() > INLINE_BUILD_MAX_ROWS {
            spawn_background_build(reader.clone(), data_root.to_path_buf());
            return Ok(LaneState::Building);
        }
        let rows = reader.searchable_rows_for_blocking(&keys)?;
        sparse::build_index(&rows, data_root)?;
        Ok(LaneState::Ready)
    });

    match prepared {
        Ok(state) => state,
        // Not being able to prepare the index is not being able to answer, and a
        // caller must never be handed an empty list for it.
        Err(error) => LaneState::Failed(error.to_string()),
    }
}

/// Build or reconcile the sparse index, blocking. Warmup paths and the eval
/// boards call this; agent searches self-heal lazily (inline for small corpora,
/// background for large ones, which answer `Building` until it lands).
pub fn ensure_sparse_index(reader: &SessionDataReader) -> Result<()> {
    match prepare_index(reader, reader.data_root()) {
        // `Building` is a real answer to "is it ready" — it is not — even though a
        // build was just set going. Reporting success here would let a caller
        // believe an index it cannot query yet is in place.
        LaneState::Building => Err(LitecodeError::IndexNotReady(
            LaneState::Building.unanswered().unwrap_or_default(),
        )),
        LaneState::Failed(why) => Err(LitecodeError::IndexNotReady(why)),
        LaneState::Ready => Ok(()),
    }
}

/// One writer at a time: builds and reconciles are rare but must not race each
/// other (or two agent searches).
fn with_refresh_lock<T, E>(f: impl FnOnce() -> std::result::Result<T, E>) -> std::result::Result<T, E> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// Full build for a corpus too large to build inline: one background thread per
/// workspace, fire-and-forget. The next search finds the finished index.
fn spawn_background_build(reader: SessionDataReader, data_root: PathBuf) {
    static BUILDING: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let building = BUILDING.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let mut guard = building.lock().unwrap_or_else(|e| e.into_inner());
        if !guard.insert(data_root.clone()) {
            return;
        }
    }
    std::thread::spawn(move || {
        let result = with_refresh_lock(|| {
            let rows = reader.searchable_rows_blocking(None)?;
            sparse::build_index(&rows, reader.data_root())
        });
        match result {
            Ok(()) => tracing::info!(path = %data_root.display(), "sparse session index ready"),
            Err(error) => tracing::warn!(error = %error, "sparse session index build failed"),
        }
        let building = BUILDING.get_or_init(|| Mutex::new(HashSet::new()));
        building
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&data_root);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::user_text;
    use tempfile::TempDir;

    /// A corpus too large to build inline is answered `Building` and built in the
    /// background, instead of the search waiting out the whole build.
    #[test]
    fn a_large_corpus_is_not_built_inside_the_search() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        let rows: Vec<_> = (0..INLINE_BUILD_MAX_ROWS + 1)
            .map(|i| user_text(&format!("row number {i}")))
            .collect();
        data.insert_items(&id, &rows).unwrap();
        let reader = SessionDataReader::open(&db);

        assert_eq!(
            prepare_index(&reader, reader.data_root()),
            LaneState::Building,
            "the caller is answered now, not after the build"
        );
    }
}
