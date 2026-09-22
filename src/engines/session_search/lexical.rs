//! Always-on lexical lane: the sparse FTS5 index.
//!
//! Ported from the validated eval chain (`sparse.rs` + `chunk.rs` + `echo.rs`
//! next door): exact substring → proximity → routed BM25 → n-gram fallback,
//! with `|` alternatives and an exact match span per hit. The index lives at
//! `<data_root>/session-index/sparse.db`; this module owns its lifecycle:
//! build when missing, reconcile (change-id gated) when the store moved on.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::session::SessionDataReader;
use crate::types::{LitecodeError, Result};

use super::sparse::{self, Lane};
use super::{SessionHitLane, SessionTextHit, SessionTextQuery, filter_hits};

/// How many hits one query fetches; the page paginates over this list.
const FETCH_DEPTH: usize = 200;
/// Below this many searchable rows the index builds inline (tests and fresh
/// workspaces: milliseconds). Above it the build goes to a background thread and
/// the first search returns empty until it lands — the tool's timeout must not
/// be spent on a full corpus build.
const INLINE_BUILD_MAX_ROWS: usize = 1500;

/// Lexical search over the sparse index. Always-on.
/// `|` separates alternatives; any alternative may match (handled in the lane).
pub fn search_lexical(    reader: &SessionDataReader,
    query: &SessionTextQuery,
) -> Result<Vec<SessionTextHit>> {
    let needle = query.query.trim();
    if needle.is_empty() {
        return Err(LitecodeError::Config(
            "session search query is required".into(),
        ));
    }

    let data_root = reader.data_root();
    let path = sparse::sparse_index_path(data_root);
    if sparse::needs_rebuild(&path)? {
        let rows = match reader.searchable_rows_blocking(None) {
            Ok(rows) => rows,
            // A missing/unreadable store is an empty corpus, not an error.
            Err(_) => return Ok(Vec::new()),
        };
        if rows.len() <= INLINE_BUILD_MAX_ROWS {
            with_refresh_lock(|| {
                sparse::build_index(
                    &rows,
                    data_root,
                    reader.latest_change_id_blocking().unwrap_or(0),
                )
            })?;
        } else {
            spawn_background_build(reader.clone(), data_root.to_path_buf());
            return Ok(Vec::new());
        }
    } else if sparse::is_stale(reader, &path)? {
        let rows = match reader.searchable_rows_blocking(None) {
            Ok(rows) => rows,
            Err(_) => return Ok(Vec::new()),
        };
        with_refresh_lock(|| sparse::refresh_index(reader, &rows, data_root))?;
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
    Ok(filter_hits(ranked, query))
}

/// Build or reconcile the sparse index, blocking. Warmup paths and the eval
/// boards call this; agent searches self-heal lazily (inline for small corpora,
/// background for large ones).
pub fn ensure_sparse_index(reader: &SessionDataReader) -> Result<()> {
    let data_root = reader.data_root();
    let path = sparse::sparse_index_path(data_root);
    if sparse::needs_rebuild(&path)? {
        let rows = reader.searchable_rows_blocking(None)?;
        with_refresh_lock(|| {
            sparse::build_index(
                &rows,
                data_root,
                reader.latest_change_id_blocking().unwrap_or(0),
            )
        })?;
    } else if sparse::is_stale(reader, &path)? {
        let rows = reader.searchable_rows_blocking(None)?;
        with_refresh_lock(|| sparse::refresh_index(reader, &rows, data_root))?;
    }
    Ok(())
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
            sparse::build_index(
                &rows,
                reader.data_root(),
                reader.latest_change_id_blocking().unwrap_or(0),
            )
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
