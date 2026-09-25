//! Always-on lexical lane: the sparse FTS5 index.
//!
//! Ported from the validated eval chain (`sparse.rs` + `chunk.rs` + `echo.rs`
//! next door): exact substring → proximity → routed BM25 → n-gram fallback,
//! with `|` alternatives and an exact match span per hit. The index lives at
//! `<data_root>/session-index/sparse.db`; this module owns its lifecycle:
//! rebuild when the file is missing or incompatible, otherwise reconcile the
//! final source key set behind every search — dispatched by the query, run on a
//! background thread, and never waited on.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::session::SessionDataReader;
use crate::types::{LitecodeError, Result};

use super::sparse::{self, Lane};
use super::{SessionHitLane, SessionTextHit, SessionTextQuery, filter_hits};

/// How many hits one query fetches; the page paginates over this list.
///
/// This is the lane's **final pool**: the leaf layers each take their own
/// candidate depth above it (see `ranking::LayerSemantics`), and the pool is
/// truncated to this size only after every layer's evidence has been merged and
/// folded to rows.
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
    /// The index was answerable and the query ran against it. An empty hit list is
    /// now a fact about the corpus as the index holds it: the index may be behind
    /// the store (the refresh for that runs behind the query, never in front of
    /// it), but nothing was skipped and nothing was guessed.
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
            // Provenance, carried up intact: the ranking above this lane is
            // built from *why* a row matched, not from a number that mixes the
            // reasons together.
            role: h.role,
            evidence: h.evidence,
            rank: h.rank,
        })
        .collect();
    Ok((filter_hits(ranked, query), LaneState::Ready))
}

/// Answer from the index, and say whether the lane can answer at all.
///
/// The delta is never applied in front of the query: a usable index is answered
/// from as it stands, and a refresh is dispatched behind it. A session corpus
/// moves on every turn, so "catch up first" meant every search in a live
/// workspace waited for the turn that never pauses — the same reason the
/// semantic lane never refreshes on demand.
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
    if !must_rebuild {
        // The index is answerable as it stands. Nothing above this line waits,
        // and neither does the caller: the refresh runs behind the query.
        spawn_sparse_refresh(reader);
        return LaneState::Ready;
    }
    let prepared = with_refresh_lock(|| -> Result<LaneState> {
        // Re-checked inside the lock, not before it: another build may have
        // landed while this one waited, and building over it would redo work
        // that was already done. Nothing here reconciles — that is the
        // background refresh's job, and a search must not wait for it. The diff
        // itself is never skipped, only deferred: the refresh runs it on every
        // pass, so a missed notification cannot hide a row.
        if !sparse::needs_rebuild(&path)? {
            spawn_sparse_refresh(reader);
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
/// boards call this; agent searches never do — they answer from the index as it
/// stands and let the refresh run behind them.
pub fn ensure_sparse_index(reader: &SessionDataReader) -> Result<()> {
    let data_root = reader.data_root();
    if !sparse::needs_rebuild(&sparse::sparse_index_path(data_root)).unwrap_or(true) {
        // Already usable: this caller asked for a catch-up, so the delta is
        // applied here, on its thread, under the one refresh lock.
        with_refresh_lock(|| sparse::refresh_from_source(reader, data_root))?;
        return Ok(());
    }
    match prepare_index(reader, data_root) {
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
/// other. Searches only take it to build, never to check for a delta.
fn with_refresh_lock<T, E>(
    f: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// One background worker per data root. A build and a refresh are never both
/// wanted at once (a build makes the file compatible, a refresh only runs when
/// it already is), and a pile of searches must not pile up work: the second
/// caller is answered by the first caller's pass.
fn begin_background(data_root: &std::path::Path) -> bool {
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(data_root.to_path_buf())
}

fn end_background(data_root: &std::path::Path) {
    in_flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(data_root);
}

fn in_flight() -> &'static Mutex<HashSet<PathBuf>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Full build for a corpus too large to build inline: one background thread per
/// workspace, fire-and-forget. The next search finds the finished index.
fn spawn_background_build(reader: SessionDataReader, data_root: PathBuf) {
    if !begin_background(&data_root) {
        return;
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
        end_background(&data_root);
    });
}

/// Bring the final source key set to the index, behind whoever asked.
///
/// Fire-and-forget by design: the caller is a search that must not wait, or the
/// idle tick that keeps the window small. One pass at a time per data root — the
/// diff is never skipped, only deferred, so a missed notification cannot hide a
/// row.
pub fn spawn_sparse_refresh(reader: &SessionDataReader) {
    let data_root = reader.data_root().to_path_buf();
    if !begin_background(&data_root) {
        return;
    }
    let reader = reader.clone();
    std::thread::spawn(move || {
        let result = with_refresh_lock(|| sparse::refresh_from_source(&reader, reader.data_root()));
        match result {
            Ok(0) => {}
            Ok(changed) => tracing::debug!(changed, "sparse session index refreshed"),
            Err(error) => tracing::warn!(error = %error, "sparse session index refresh failed"),
        }
        end_background(&data_root);
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

    /// A usable index answers now; the delta is refreshed behind the query, not
    /// in front of it. `Ready` here is what keeps a search in a moving corpus
    /// from waiting for the turn that never pauses.
    #[test]
    fn a_usable_index_is_answered_without_waiting_for_the_delta() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("first row")]).unwrap();
        let reader = SessionDataReader::open(&db);
        assert_eq!(
            prepare_index(&reader, reader.data_root()),
            LaneState::Ready,
            "a small corpus is built inline on the first search"
        );

        data.insert_items(&id, &[user_text("second row")]).unwrap();

        assert_eq!(
            prepare_index(&reader, reader.data_root()),
            LaneState::Ready,
            "a moved store is answered from the index as it stands"
        );
    }
}
