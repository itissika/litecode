//! Always-on lexical lane: the sparse FTS5 index.
//!
//! Ported from the validated eval chain (`sparse.rs` + `chunk.rs` + `echo.rs`
//! next door): exact substring → proximity → routed BM25 → n-gram fallback,
//! with `|` alternatives and an exact match span per hit. The index lives at
//! `<data_root>/session-index/sparse.db`; this module owns its lifecycle.
//!
//! # Lifecycle
//!
//! One decision, taken from the file alone:
//!
//! | what is on disk           | what happens                  |
//! |---------------------------|-------------------------------|
//! | missing, unopenable       | rebuild                       |
//! | written by another schema | rebuild                       |
//! | current schema            | reconcile the settled key set |
//!
//! The first two rows are the whole of what `schema` decides, and it decides it
//! about the **file**: whether this build can read it at all. It says nothing
//! about the corpus. Freshness is the third row's question, answered by a diff of
//! the settled `(session_id, seq)` set — a no-op when the corpus has not moved,
//! because a settled row is never rewritten in place, so a row can only enter
//! that set or leave it.
//!
//! Keeping the two questions apart is the point. A reconcile cannot fix a file it
//! cannot read, so a maintenance path that can *only* reconcile leaves a
//! stale-schema index failing once per tick, forever.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::session::SessionDataReader;
use crate::session::transcript_file::SearchableRow;
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
    /// the store (the catch-up runs behind the query, never in front of it), but
    /// nothing was skipped and nothing was guessed.
    Ready,
    /// There was no index to answer from, so one is being built in the background.
    /// The query did **not** run.
    Building,
    /// The index could not be built or read. The query did **not** run.
    Failed(String),
}

impl LaneState {
    /// Why this lane did not answer, or `None` if it did.
    pub fn unanswered(&self) -> Option<String> {
        match self {
            Self::Ready => None,
            Self::Building => Some(
                "the session index is being rebuilt, so this search did not run; \
                 a later search will answer from it"
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
/// from as it stands, and the catch-up runs behind it. A session corpus moves on
/// every turn, so "catch up first" meant every search in a live workspace waited
/// for the turn that never pauses.
fn prepare_index(reader: &SessionDataReader, data_root: &Path) -> LaneState {
    let path = sparse::sparse_index_path(data_root);
    if !sparse::needs_rebuild(&path) {
        // Usable as it stands. Nothing here waits, and neither does the caller.
        spawn_sparse_maintenance(reader);
        return LaneState::Ready;
    }
    // The file has to be built. Above the inline budget that is a build measured
    // in tens of seconds, so it is dispatched and the caller is told the query did
    // not run — rather than being handed the empty list that reads as "the
    // history has no such row".
    if too_large_to_build_inline(reader) {
        // A pass that failed is the last word until one runs again, and this is a
        // chance to run it. Reporting the cause rather than "being rebuilt" is
        // what keeps the state honest: `Building` is only ever said while a pass is
        // genuinely in flight, so it cannot become a build that never lands.
        let last_failure = match worker_state(data_root) {
            Some(Worker::Failed(why)) => Some(why),
            _ => None,
        };
        spawn_sparse_maintenance(reader);
        return match last_failure {
            Some(why) => LaneState::Failed(why),
            None => LaneState::Building,
        };
    }
    match ensure_sparse_index(reader) {
        Ok(()) => LaneState::Ready,
        // Not being able to prepare the index is not being able to answer, and a
        // caller must never be handed an empty list for it.
        Err(error) => LaneState::Failed(error.to_string()),
    }
}

/// Whether the corpus is too large to build inside the search that needs it.
///
/// Keys only — integers, never bodies. A store that cannot be read is left for
/// the build to report, which keeps this from having to distinguish "empty" from
/// "unreadable".
fn too_large_to_build_inline(reader: &SessionDataReader) -> bool {
    reader
        .searchable_keys_blocking(None)
        .is_ok_and(|keys| keys.len() > INLINE_BUILD_MAX_ROWS)
}

/// Bring the index to a usable, current state. Blocking.
///
/// The lane's whole maintenance policy, in one place: rebuild when the file
/// cannot be used as it stands, reconcile when it can. Both are idempotent, so a
/// pass that arrives with nothing to do writes nothing.
///
/// This is also the only writer. Every caller that wants the index moved — the
/// idle tick, a warmup, a search with a small corpus to build — comes through
/// here, which is what makes the entry as a whole able to fix a file it can no
/// longer read.
pub fn ensure_sparse_index(reader: &SessionDataReader) -> Result<()> {
    let data_root = reader.data_root();
    with_refresh_lock(|| {
        // Re-checked under the lock, not before it: a pass that was already
        // running when this one was asked for has finished by now, and rebuilding
        // over a finished index reads the whole corpus again for nothing.
        if sparse::needs_rebuild(&sparse::sparse_index_path(data_root)) {
            let rows = settled_rows(reader)?;
            sparse::build_index(&rows, data_root)?;
        } else {
            sparse::refresh_from_source(reader, data_root)?;
        }
        Ok(())
    })
}

/// Every settled source row, or none when there is no store at all.
///
/// Only an *absent* store is an empty corpus. A store that exists but cannot be
/// read is a failure, not an empty one: publishing an empty index for it would
/// answer "the history has nothing" to every query, and it would overwrite a good
/// index to do it.
fn settled_rows(reader: &SessionDataReader) -> Result<Vec<SearchableRow>> {
    match reader.searchable_rows_blocking(None) {
        Ok(rows) => Ok(rows),
        Err(_) if !reader.path().is_file() => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// One writer at a time. A rebuild and a reconcile move the same file, and an
/// inline build must not run over a background pass.
fn with_refresh_lock<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// What a data root's background pass is doing, or what the last one failed with.
#[derive(Clone)]
enum Worker {
    /// A pass is in flight. A request that arrives now is answered by it.
    Running,
    /// Nothing is in flight, and the last pass failed with this cause.
    Failed(String),
}

/// One entry per data root, and the reason a search can tell "a build is coming"
/// from "the last build failed": the first is only ever reported while a pass is
/// genuinely running.
fn workers() -> &'static Mutex<HashMap<PathBuf, Worker>> {
    static WORKERS: OnceLock<Mutex<HashMap<PathBuf, Worker>>> = OnceLock::new();
    WORKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn worker_state(data_root: &Path) -> Option<Worker> {
    workers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(data_root)
        .cloned()
}

/// A pass's claim on its data root, released however the pass ends — including a
/// panic, which must not leave the lane claiming a build forever.
struct Claimed(PathBuf);

impl Drop for Claimed {
    fn drop(&mut self) {
        let mut guard = workers().lock().unwrap_or_else(|e| e.into_inner());
        if matches!(guard.get(&self.0), Some(Worker::Running)) {
            guard.remove(&self.0);
        }
    }
}

/// Run one maintenance pass off the caller's thread.
///
/// One pass per data root at a time: a request that arrives while one is running
/// loses nothing, because the running pass makes the same decision from the same
/// file. That is only true of a single entry point — a build and a reconcile are
/// one decision here, so a dropped duplicate cannot be the wrong one.
pub fn spawn_sparse_maintenance(reader: &SessionDataReader) {
    let data_root = reader.data_root().to_path_buf();
    {
        let mut guard = workers().lock().unwrap_or_else(|e| e.into_inner());
        if matches!(guard.get(&data_root), Some(Worker::Running)) {
            return;
        }
        guard.insert(data_root.clone(), Worker::Running);
    }
    let reader = reader.clone();
    std::thread::spawn(move || {
        // Held for the pass's whole life, and only the claim is released on the
        // way out: a pass that ended by failing has already replaced it with the
        // cause, and that is what a search should read until the next one runs.
        let _claim = Claimed(data_root.clone());
        let result = ensure_sparse_index(&reader);
        report(&data_root, result);
    });
}

/// Report one finished pass, and let the next request for this root start.
///
/// A failure is logged when it is new or has changed. The tick retries every
/// thirty seconds, so a store that cannot be read fails identically each time and
/// says so once; a recovery is worth a line, because the last thing said about
/// this lane would otherwise be a failure that is no longer true.
fn report(data_root: &Path, result: Result<()>) {
    let mut guard = workers().lock().unwrap_or_else(|e| e.into_inner());
    match result {
        Ok(()) => {
            if matches!(guard.get(data_root), Some(Worker::Failed(_))) {
                tracing::info!(path = %data_root.display(), "sparse session index recovered");
            }
            guard.remove(data_root);
        }
        Err(error) => {
            let message = error.to_string();
            let repeated = matches!(guard.get(data_root), Some(Worker::Failed(seen)) if *seen == message);
            if !repeated {
                tracing::warn!(
                    path = %data_root.display(),
                    error = %message,
                    "sparse session index maintenance failed"
                );
            }
            guard.insert(data_root.to_path_buf(), Worker::Failed(message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::user_text;
    use tempfile::TempDir;

    /// A reader over a store holding one row that a search can find, and one that
    /// only this test knows about.
    fn seeded(dir: &Path) -> SessionDataReader {
        let db = dir.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&id, &[user_text("alpha UNIQUE_SESSION_PHRASE omega")])
            .unwrap();
        SessionDataReader::open(&db)
    }

    fn find(reader: &SessionDataReader, needle: &str) -> Vec<SessionTextHit> {
        let query = SessionTextQuery {
            query: needle.into(),
            offset: 0,
            ..Default::default()
        };
        let (hits, state) = search_lexical(reader, &query).unwrap();
        assert_eq!(state, LaneState::Ready, "the lane answered");
        hits
    }

    /// Stamp the file as a previous build's, without disturbing its contents.
    fn restamp_as_old_schema(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute("UPDATE meta SET value = '0' WHERE key = 'schema'", [])
            .unwrap();
    }

    /// The bug this lifecycle was rebuilt around: a file written by another schema
    /// is a **rebuild**, not something to reconcile row by row. Reconciling it
    /// cannot work, so a pass that can only reconcile fails on every tick and the
    /// lane never comes back.
    #[test]
    fn a_file_from_another_schema_is_rebuilt() {
        let dir = TempDir::new().unwrap();
        let reader = seeded(dir.path());
        assert_eq!(find(&reader, "UNIQUE_SESSION_PHRASE").len(), 1);

        let path = sparse::sparse_index_path(reader.data_root());
        restamp_as_old_schema(&path);
        assert!(sparse::needs_rebuild(&path), "an old schema needs a rebuild");

        assert!(
            ensure_sparse_index(&reader).is_ok(),
            "one maintenance pass repairs it"
        );
        assert!(!sparse::needs_rebuild(&path), "and the file is current again");
        assert_eq!(
            find(&reader, "UNIQUE_SESSION_PHRASE").len(),
            1,
            "with its rows intact"
        );
    }

    /// The outage this work exists for: a schema bump left the idle tick calling
    /// the one entry that cannot repair it, so every pass failed and the lane
    /// stayed down until somebody happened to search. The tick's entry is pinned
    /// here: with no search at all, the pass brings an old file back.
    #[test]
    fn the_idle_pass_repairs_an_old_schema_without_a_search() {
        let dir = TempDir::new().unwrap();
        let reader = seeded(dir.path());
        let path = sparse::sparse_index_path(reader.data_root());
        assert!(ensure_sparse_index(&reader).is_ok());

        restamp_as_old_schema(&path);
        assert!(sparse::needs_rebuild(&path), "the file is from an older build");

        // Exactly what `serve`'s 30-second tick does, and nothing else.
        spawn_sparse_maintenance(&reader);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while sparse::needs_rebuild(&path) {
            assert!(
                std::time::Instant::now() < deadline,
                "the idle pass must repair the file on its own"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            find(&reader, "UNIQUE_SESSION_PHRASE").len(),
            1,
            "and the lane answers again"
        );
    }

    /// A file that cannot be opened at all is also a rebuild. Nothing else can
    /// make an unreadable file readable, so reporting the open failure instead
    /// would leave the lane failed for good.
    #[test]
    fn an_unopenable_index_is_rebuilt() {
        let dir = TempDir::new().unwrap();
        let reader = seeded(dir.path());
        assert_eq!(find(&reader, "UNIQUE_SESSION_PHRASE").len(), 1);

        let path = sparse::sparse_index_path(reader.data_root());
        std::fs::write(&path, b"this is not a database").unwrap();
        assert!(sparse::needs_rebuild(&path), "an unreadable file needs a rebuild");

        assert!(
            ensure_sparse_index(&reader).is_ok(),
            "the corrupt file is replaced rather than reported"
        );
        assert!(!sparse::needs_rebuild(&path));
        assert_eq!(find(&reader, "UNIQUE_SESSION_PHRASE").len(), 1);
    }

    /// The third row of the table: a current file is reconciled, and a corpus that
    /// has not moved makes that a no-op.
    #[test]
    fn a_current_index_is_reconciled_without_a_rebuild() {
        let dir = TempDir::new().unwrap();
        let reader = seeded(dir.path());
        assert_eq!(find(&reader, "UNIQUE_SESSION_PHRASE").len(), 1);

        let path = sparse::sparse_index_path(reader.data_root());
        let built = std::fs::metadata(&path).unwrap().len();

        assert!(ensure_sparse_index(&reader).is_ok());
        assert!(ensure_sparse_index(&reader).is_ok());
        assert!(
            std::fs::metadata(&path).unwrap().len() >= built,
            "a settled pass leaves the index in place"
        );
        assert_eq!(find(&reader, "UNIQUE_SESSION_PHRASE").len(), 1);
    }

    /// A missing file is built, and the search that asked for it answers.
    #[test]
    fn a_missing_index_is_built() {
        let dir = TempDir::new().unwrap();
        let reader = seeded(dir.path());
        let path = sparse::sparse_index_path(reader.data_root());
        assert!(!path.is_file(), "nothing has been built yet");

        assert_eq!(find(&reader, "UNIQUE_SESSION_PHRASE").len(), 1);
        assert!(path.is_file());
    }

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

    /// Above the inline budget the build is dispatched, so a failure cannot be
    /// reported by the call that started it. It comes back from the worker's record
    /// instead — and it comes back as the **cause**, not as a build that is forever
    /// about to land.
    #[test]
    fn a_failed_background_build_is_reported_as_its_cause() {
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
        drop(data);
        drop(lease);
        // A row the derivation cannot read: the build that runs above the inline
        // budget will fail on it every time. Its seq sits past the corpus above.
        let broken_seq = INLINE_BUILD_MAX_ROWS + 500;
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute(
                "INSERT INTO transcript_items
                    (session_id, seq, turn_id, turn_seq, item_type, kind, body,
                     token_estimate, created_at, event_type, surface_op, state)
                 VALUES (?1, ?2, 't', 0, 'message', 'item/user', '{{{ not an item',
                         1, 1, 'message', 'append', 'final')",
                rusqlite::params![id, broken_seq as i64],
            )
            .unwrap();
        }
        let reader = SessionDataReader::open(&db);

        assert_eq!(
            prepare_index(&reader, reader.data_root()),
            LaneState::Building,
            "the build is dispatched, so the caller is answered now"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !matches!(worker_state(reader.data_root()), Some(Worker::Failed(_))) {
            assert!(
                std::time::Instant::now() < deadline,
                "the dispatched build must finish, one way or the other"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        match prepare_index(&reader, reader.data_root()) {
            LaneState::Failed(why) => assert!(
                why.contains(&format!("{id}:{broken_seq}")),
                "the caller is told the cause, not that a build is still coming: {why}"
            ),
            other => panic!("a failed pass must be reported as failed, got {other:?}"),
        }
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
