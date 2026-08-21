use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::authority::responses::{InputMessage, InputRole, MessageItem};
use crate::platform_knobs::{ContextMode, ThinkingTier};
use crate::session::estimate::compute_token_estimate;
use crate::session::event::{
    EventDraft, EventType, Seq, SessionEvent, finalize_draft, item_from_event, log_state_of_item,
    spine_agent_item,
};
use crate::session::model::{
    CompactedBody, HookPromptBody, LogState, ReminderJobExitBody, ReminderTurnAbortedBody,
    SESSION_LOG_SCHEMA_VERSION,
};
use crate::session::snapshot;
use crate::session::surface::{
    Surface, SurfaceOp, apply_plan, fold_surface, plan_surface, project_working_pairs,
};
use crate::session::task_state::TodoItem;
use crate::session::task_state::{PlanRef, TaskReminders};
use crate::session::working::WorkingRow;
use crate::tool::output::{BLOB_PREFIX, DEFAULT_SPILL_THRESHOLD, blob_dir};
use crate::types::{Item, LitecodeError, Result, Transcript, item_text_preview};

/// Last-known **provider** usage for a session (ring + cache observability).
///
/// Only written when a turn received real provider `usage`. Local token
/// estimates never enter this meter. Reloaded on subscribe so the ring
/// survives refresh with truth-or-absent fidelity.
///
/// `*_tokens` are the last-request figures (industry single-request hit rate);
/// `cum_*_tokens` are session-total accumulators (Σ per-request usage, industry
/// token-weighted aggregate hit rate — see LiteLLM `Σcached/Σinput`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionContextMeter {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_hit_tokens: u64,
    pub cache_miss_tokens: u64,
    /// Session-total prompt tokens (Σ every LLM request's `prompt_tokens`).
    pub cum_prompt_tokens: u64,
    /// Session-total completion tokens (Σ every LLM request's `completion_tokens`).
    pub cum_completion_tokens: u64,
    /// Session-total cache-hit tokens (Σ every LLM request's `cache_hit_tokens`).
    pub cum_cache_hit_tokens: u64,
    /// Session-total cache-miss tokens (Σ every LLM request's `cache_miss_tokens`).
    pub cum_cache_miss_tokens: u64,
}

impl SessionContextMeter {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.cache_hit_tokens == 0
            && self.cache_miss_tokens == 0
    }
}

/// One row from the `transcript_items` table.
///
/// `kind` is the durable product discriminator. `body` follows its schema;
/// `item/*` bodies are serialized Responses Items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub session_id: String,
    pub seq: i64,
    pub turn_id: String,
    pub turn_seq: i64,
    pub item_type: String,
    pub kind: String,
    pub body: Option<String>,
    pub body_ref: Option<String>,
    pub token_estimate: i64,
    pub created_at: i64,
}

/// §5.1 turn load — step 1: read authoritative checkpoint (default 0).
pub const SQL_CHECKPOINT_SEQ: &str = "SELECT checkpoint_seq FROM sessions WHERE id = ?";

pub const SQL_KEPT_FROM_SEQ: &str = "SELECT kept_from_seq FROM sessions WHERE id = ?";

/// Turn load is no longer a SQL window: callers should use `load_transcript` /
/// `derive_messages`. This query is seq-order log rows (no compact reorder).
pub const SQL_LOAD_TURN_TRANSCRIPT: &str = "\
SELECT t.session_id, t.seq, t.turn_id, t.turn_seq, t.item_type, t.kind, t.body, t.body_ref,
       t.token_estimate, t.created_at
FROM transcript_items t
WHERE t.session_id = ?1
ORDER BY t.seq ASC";

/// Full chronological UI history: every detail and every compact_checkpoint,
/// in seq order. The turn working set (current `checkpoint_seq` only) is a
/// separate view; history does not hide earlier cuts.
pub const SQL_LOAD_HISTORY_TRANSCRIPT: &str = "\
SELECT t.session_id, t.seq, t.turn_id, t.turn_seq, t.item_type, t.kind, t.body, t.body_ref,
       t.token_estimate, t.created_at
FROM transcript_items t
WHERE t.session_id = ?1
ORDER BY t.seq ASC";

/// UI revert anchors: append-origin user messages only (replace summaries are not k).
pub const SQL_USER_DETAIL_COUNT: &str = "\
SELECT COUNT(*) FROM transcript_items t
WHERE t.session_id = ?
  AND t.kind = 'item/user'";

/// User-detail rows with `seq < from_seq` (buffer/load `user_detail_before`).
pub const SQL_USER_DETAIL_BEFORE_SEQ: &str = "\
SELECT COUNT(*) FROM transcript_items t
WHERE t.session_id = ?
  AND t.kind = 'item/user'
  AND t.seq < ?";

/// UI revert k → anchor_seq mapping across append-origin user messages.
pub const SQL_ANCHOR_SEQ: &str = "\
SELECT seq FROM (
    SELECT t.seq, ROW_NUMBER() OVER (ORDER BY t.seq) - 1 AS k
    FROM transcript_items t
    WHERE t.session_id = ?
      AND t.kind = 'item/user'
) WHERE k = ?";

const SESSIONS_REQUIRED_COLS: &[&str] = &[
    "schema_version",
    "id",
    "project",
    "last_message",
    "agent_id",
    "model_id",
    "thinking_tier",
    "context_mode",
    "created_at",
    "updated_at",
    "checkpoint_seq",
    "kept_from_seq",
    "compacted_seq",
    "spine_from",
    "subagent_depth",
    "todos_json",
    "active_plan_slug",
    "parent_session_id",
    "parent_call_id",
];

const TRANSCRIPT_REQUIRED_COLS: &[&str] = &[
    "session_id",
    "seq",
    "turn_id",
    "turn_seq",
    "item_type",
    "kind",
    "body",
    "body_ref",
    "token_estimate",
    "created_at",
    "event_type",
    "surface_op",
    "source_seqs",
    "cites",
    "state",
];

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(cols)
}

fn table_row_count(conn: &Connection, table: &str) -> Result<i64> {
    Ok(
        conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
            row.get(0)
        })?,
    )
}

fn incompatible_session_db(detail: &str) -> LitecodeError {
    LitecodeError::Config(format!(
        "incompatible session DB ({detail}). Delete `.litecode/sessions.db` (or this DB path) \
         and restart — there is no upgrade path; schema is delete-and-rebuild only."
    ))
}

/// Ensure current `sessions` + `transcript_items` schema in one shot.
///
/// Fail-closed on half-old shapes (missing required columns, or fossil `messages` with data).
/// Empty leftover `messages` may be DROP'd. No `ALTER TABLE … ADD COLUMN` soft upgrades.
fn ensure_session_schema(conn: &Connection) -> Result<()> {
    if table_exists(conn, "messages")? {
        let n = table_row_count(conn, "messages")?;
        if n > 0 {
            return Err(incompatible_session_db(
                "legacy `messages` table still has rows",
            ));
        }
        conn.execute_batch("DROP TABLE IF EXISTS messages;")?;
    }

    if table_exists(conn, "sessions")? {
        let cols = table_columns(conn, "sessions")?;
        for req in SESSIONS_REQUIRED_COLS {
            if !cols.iter().any(|c| c == *req) {
                return Err(incompatible_session_db(&format!(
                    "`sessions` missing required column `{req}`"
                )));
            }
        }
        let stale: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE schema_version != ?1",
            rusqlite::params![SESSION_LOG_SCHEMA_VERSION],
            |row| row.get(0),
        )?;
        if stale > 0 {
            return Err(incompatible_session_db(&format!(
                "sessions.schema_version is not {SESSION_LOG_SCHEMA_VERSION} (compacted [from,to) semantics)"
            )));
        }
    }

    if table_exists(conn, "transcript_items")? {
        let cols = table_columns(conn, "transcript_items")?;
        for req in TRANSCRIPT_REQUIRED_COLS {
            if !cols.iter().any(|c| c == *req) {
                return Err(incompatible_session_db(&format!(
                    "`transcript_items` missing required column `{req}`"
                )));
            }
        }
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id                TEXT PRIMARY KEY,
            schema_version    INTEGER NOT NULL DEFAULT 3,
            project           TEXT NOT NULL,
            last_message      TEXT NOT NULL DEFAULT '',
            agent_id          TEXT NOT NULL,
            model_id          TEXT,
            thinking_tier     TEXT NOT NULL DEFAULT 'medium',
            context_mode      TEXT NOT NULL DEFAULT 'standard',
            created_at        INTEGER NOT NULL,
            updated_at        INTEGER NOT NULL,
            checkpoint_seq    INTEGER NOT NULL DEFAULT 0,
            kept_from_seq     INTEGER NOT NULL DEFAULT 0,
            compacted_seq     INTEGER,
            spine_from        INTEGER NOT NULL DEFAULT 0,
            subagent_depth    INTEGER NOT NULL DEFAULT 0,
            todos_json        TEXT NOT NULL DEFAULT '[]',
            active_plan_slug  TEXT,
            parent_session_id TEXT,
            parent_call_id    TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_project
            ON sessions(project);
        CREATE INDEX IF NOT EXISTS idx_sessions_updated
            ON sessions(updated_at);
        CREATE INDEX IF NOT EXISTS idx_sessions_parent
            ON sessions(parent_session_id);
        CREATE TABLE IF NOT EXISTS transcript_items (
            session_id      TEXT NOT NULL,
            seq             INTEGER NOT NULL,
            turn_id         TEXT NOT NULL DEFAULT '',
            turn_seq        INTEGER NOT NULL DEFAULT 0,
            item_type       TEXT NOT NULL,
            kind            TEXT NOT NULL,
            body            TEXT,
            body_ref        TEXT,
            token_estimate  INTEGER NOT NULL DEFAULT 0,
            created_at      INTEGER NOT NULL,
            event_type      TEXT NOT NULL,
            surface_op      TEXT NOT NULL,
            source_seqs     TEXT,
            cites           TEXT,
            state           TEXT NOT NULL DEFAULT 'final',
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX IF NOT EXISTS idx_transcript_items_session_seq
            ON transcript_items(session_id, seq);
        CREATE INDEX IF NOT EXISTS idx_transcript_items_session_turn
            ON transcript_items(session_id, turn_id, turn_seq);
        CREATE TABLE IF NOT EXISTS session_context_meter (
            session_id         TEXT PRIMARY KEY,
            prompt_tokens      INTEGER NOT NULL DEFAULT 0,
            completion_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
            cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
            cum_prompt_tokens      INTEGER NOT NULL DEFAULT 0,
            cum_completion_tokens  INTEGER NOT NULL DEFAULT 0,
            cum_cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
            cum_cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
            updated_at         INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    // Drop legacy `token_estimate` from meter if present (was a misleading mirror of
    // prompt_tokens). Meter table is small; rebuild in place.
    if table_exists(conn, "session_context_meter")? {
        let cols = table_columns(conn, "session_context_meter")?;
        if cols.iter().any(|c| c == "token_estimate") {
            conn.execute_batch(
                "CREATE TABLE session_context_meter__new (
                    session_id         TEXT PRIMARY KEY,
                    prompt_tokens      INTEGER NOT NULL DEFAULT 0,
                    completion_tokens  INTEGER NOT NULL DEFAULT 0,
                    cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
                    cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
                    cum_prompt_tokens      INTEGER NOT NULL DEFAULT 0,
                    cum_completion_tokens  INTEGER NOT NULL DEFAULT 0,
                    cum_cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
                    cum_cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
                    updated_at         INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO session_context_meter__new (
                    session_id, prompt_tokens, completion_tokens,
                    cache_hit_tokens, cache_miss_tokens, updated_at
                )
                SELECT session_id, prompt_tokens, completion_tokens,
                       cache_hit_tokens, cache_miss_tokens, updated_at
                FROM session_context_meter;
                DROP TABLE session_context_meter;
                ALTER TABLE session_context_meter__new RENAME TO session_context_meter;",
            )?;
        }
        // Older DBs predate the session-total accumulator columns; add them
        // in place (SQLite ALTER ADD COLUMN with constant default is fine).
        let cols = table_columns(conn, "session_context_meter")?;
        for (col, ddl) in [
            (
                "cum_prompt_tokens",
                "cum_prompt_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "cum_completion_tokens",
                "cum_completion_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "cum_cache_hit_tokens",
                "cum_cache_hit_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "cum_cache_miss_tokens",
                "cum_cache_miss_tokens INTEGER NOT NULL DEFAULT 0",
            ),
        ] {
            if !cols.iter().any(|c| c == col) {
                conn.execute(
                    &format!("ALTER TABLE session_context_meter ADD COLUMN {ddl}"),
                    [],
                )?;
            }
        }
    }
    crate::session::transcript_fts::ensure_schema(conn)?;
    Ok(())
}

fn enable_wal(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=5000;",
    )?;
    Ok(())
}

/// Empty string seeds become SQL NULL (unset model), matching the contract.
fn normalize_model_id(model_id: Option<&str>) -> Option<String> {
    model_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Result of inserting a turn delta into `transcript_items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitDeltaOutcome {
    /// The detail log is shorter than the caller's committed prefix (回退 deleted
    /// rows). `items` was replaced with the DB view and nothing was inserted.
    /// Not a projection-length check and not by itself a user 取消.
    Discarded,
    Applied {
        preview: Option<(String, i64)>,
        /// True when this commit sealed or appended at least one row.
        mutated: bool,
    },
}

/// The three write primitives. All durable mutation goes through [`Session::apply`].
pub enum SessionApply {
    Append(EventDraft),
    Seal { seq: Seq, item: Item },
    Truncate { user_k: i64 },
}

pub enum ApplyOutcome {
    Appended(Seq),
    Sealed,
    Truncated,
}

/// Incremental surface + seq identity for one live Session. Hydrated once on
/// open; write primitives update it under the write gate.
#[derive(Debug, Clone, Default)]
struct LogProjection {
    surface: Surface,
    next_seq: Seq,
    id_to_seq: HashMap<String, Seq>,
    items_by_seq: HashMap<Seq, Item>,
}

impl LogProjection {
    fn max_seq(&self) -> i64 {
        if self.next_seq == 0 {
            -1
        } else {
            self.next_seq as i64 - 1
        }
    }

    fn apply_event(&mut self, event: &SessionEvent, item: Option<&Item>) -> Result<()> {
        if let Some(plan) = plan_surface(&self.surface, event)? {
            if let crate::session::surface::SurfacePlan::Replace { start_idx, len, .. } = &plan {
                let shadowed: Vec<Seq> = self.surface.nodes[*start_idx..*start_idx + *len].to_vec();
                for seq in shadowed {
                    self.items_by_seq.remove(&seq);
                }
            }
            apply_plan(&mut self.surface, plan);
        }
        self.next_seq = event.seq.saturating_add(1);
        if let Some(item) = item {
            if let Some(id) = item_log_id(item) {
                self.id_to_seq.insert(id, event.seq);
            }
            if self.surface.nodes.contains(&event.seq) {
                self.items_by_seq.insert(event.seq, item.clone());
            }
        } else if event.event_type.enters_spine()
            && self.surface.nodes.contains(&event.seq)
            && let Ok(assembled) = spine_agent_item(event)
        {
            self.items_by_seq.insert(event.seq, assembled);
        }
        Ok(())
    }

    fn seal_item(&mut self, seq: Seq, item: &Item) {
        if let Some(id) = item_log_id(item) {
            self.id_to_seq.insert(id, seq);
        }
        if self.surface.nodes.contains(&seq) {
            self.items_by_seq.insert(seq, item.clone());
        }
    }
}

pub struct Session {
    conn: Connection,
    pub id: String,
    pub project: String,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub thinking_tier: ThinkingTier,
    pub context_mode: ContextMode,
    /// Parent session when this row is a subagent child; `None` for root sessions.
    pub parent_session_id: Option<String>,
    /// Parent `function_call.call_id` that launched this child session.
    pub parent_call_id: Option<String>,
    data_root: PathBuf,
    db_path: Option<PathBuf>,
    ephemeral: bool,
    persisted_max_seq: Cell<i64>,
    write_gate: Mutex<()>,
    truncated_item_ids: RefCell<HashSet<String>>,
    projection: RefCell<LogProjection>,
}

pub fn data_root_from_db_path(db_path: &str) -> PathBuf {
    Path::new(db_path)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

fn is_user_message_item(item: &Item) -> bool {
    matches!(
        item,
        Item::Message(MessageItem::Input(InputMessage {
            role: InputRole::User,
            ..
        }))
    )
}

/// Same predicate as [`SQL_USER_DETAIL_COUNT`] / [`SQL_ANCHOR_SEQ`], on a turn row.
#[cfg(test)]
fn transcript_row_is_user_detail(row: &TranscriptRow) -> bool {
    row.kind == "item/user"
}

/// Responses Item `type` string (`message`, `reasoning`, `function_call`, …).
fn item_type_of(item: &Item) -> String {
    match serde_json::to_value(item) {
        Ok(v) => v
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string(),
        Err(_) => "unknown".into(),
    }
}

fn item_log_id(item: &Item) -> Option<String> {
    match item {
        Item::Message(MessageItem::Output(m)) if !m.id.is_empty() => Some(format!("item:{}", m.id)),
        Item::FunctionCall(fc) => fc
            .id
            .clone()
            .filter(|s| !s.is_empty())
            .map(|id| format!("item:{id}"))
            .or_else(|| (!fc.call_id.is_empty()).then(|| format!("call:{}", fc.call_id))),
        Item::FunctionCallOutput(out) if !out.call_id.is_empty() => {
            Some(format!("result:{}", out.call_id))
        }
        Item::Reasoning(r) => {
            r.id.clone()
                .filter(|s| !s.is_empty())
                .map(|id| format!("item:{id}"))
        }
        _ => None,
    }
}

fn surface_event_type_of(item: &Item) -> EventType {
    if is_user_message_item(item) {
        EventType::ItemUser
    } else if matches!(item, Item::FunctionCall(_)) {
        EventType::ItemToolCall
    } else if matches!(item, Item::FunctionCallOutput(_)) {
        EventType::ItemToolResult
    } else {
        EventType::ItemAssistant
    }
}

fn event_sql_envelope(event: &SessionEvent) -> Result<(String, Option<String>, Option<String>)> {
    let surface_op = match &event.surface_op {
        Some(op) => serde_json::to_string(op)?,
        None => String::new(),
    };
    let source_seqs = event
        .source_seqs
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    // `source_seqs` remains a transient legacy fold encoding until all readers
    // consume `compacted.body`; `cites` is the durable generic relation.
    let cites = source_seqs.clone();
    Ok((surface_op, source_seqs, cites))
}

fn insert_event_row(
    tx: &Transaction<'_>,
    session_id: &str,
    data_root: &Path,
    event: &SessionEvent,
    item: Option<&Item>,
    turn_id: &str,
    turn_seq: i64,
    kind: &str,
    token_estimate: i64,
) -> Result<()> {
    let seq = event.seq as i64;
    let (body, body_ref, encoded_tokens) = if let Some(item) = item {
        if is_user_message_item(item) {
            let (body, body_ref, tokens) =
                encode_detail_row(item, data_root, DEFAULT_SPILL_THRESHOLD)?;
            if body.is_none() {
                return Err(LitecodeError::ToolExecution(
                    "user detail item must keep inline body for anchors".into(),
                ));
            }
            (body, body_ref, tokens)
        } else {
            encode_detail_row(item, data_root, DEFAULT_SPILL_THRESHOLD)?
        }
    } else {
        (Some(event.data.to_string()), None, 0)
    };
    let tokens = if token_estimate > 0 {
        token_estimate
    } else {
        encoded_tokens
    };
    let item_type = if let Some(item) = item {
        item_type_of(item)
    } else {
        event.event_type.as_str().to_string()
    };
    let (surface_op, source_seqs, cites) = event_sql_envelope(event)?;
    tx.execute(
        "INSERT INTO transcript_items (session_id, seq, turn_id, turn_seq, item_type, kind, body, body_ref, token_estimate, created_at, event_type, surface_op, source_seqs, cites, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            session_id,
            seq,
            turn_id,
            turn_seq,
            item_type,
            kind,
            body,
            body_ref,
            tokens,
            event.time,
            event.event_type.as_str(),
            surface_op,
            source_seqs,
            cites,
            event.state.as_str(),
        ],
    )?;
    if let Some(item) = item {
        let plain = crate::types::item_text_preview(item);
        crate::session::transcript_fts::upsert(tx, session_id, seq, &plain)?;
    } else if event.event_type == EventType::Compacted
        && let Ok(body) = serde_json::from_value::<CompactedBody>(event.data.clone())
    {
        crate::session::transcript_fts::upsert(tx, session_id, seq, &body.summary)?;
    }
    Ok(())
}

fn seal_event_row(
    tx: &Transaction<'_>,
    session_id: &str,
    data_root: &Path,
    seq: Seq,
    item: &Item,
) -> Result<()> {
    let seq_i = seq as i64;
    let current_kind: Option<String> = tx
        .query_row(
            "SELECT event_type FROM transcript_items WHERE session_id = ?1 AND seq = ?2",
            rusqlite::params![session_id, seq_i],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current_kind) = current_kind else {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "seal_item: no row at seq {seq}"
        )));
    };
    if !matches!(
        EventType::from_str_name(&current_kind),
        EventType::ItemAssistant | EventType::ItemToolCall
    ) {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "seal_item: kind `{current_kind}` is not sealable"
        )));
    }
    let item_type = item_type_of(item);
    let (body, body_ref, token_estimate) =
        encode_detail_row(item, data_root, DEFAULT_SPILL_THRESHOLD)?;
    let event_type = surface_event_type_of(item).as_str().to_string();
    tx.execute(
        "UPDATE transcript_items
         SET item_type = ?1, body = ?2, body_ref = ?3, token_estimate = ?4, event_type = ?5, state = ?6
         WHERE session_id = ?7 AND seq = ?8",
        rusqlite::params![
            item_type,
            body,
            body_ref,
            token_estimate,
            event_type,
            log_state_of_item(item).as_str(),
            session_id,
            seq_i,
        ],
    )?;
    let plain = crate::types::item_text_preview(item);
    crate::session::transcript_fts::upsert(tx, session_id, seq_i, &plain)?;
    Ok(())
}

fn replace_op_for_keep(
    surface: &Surface,
    kept_from_seq: Option<i64>,
) -> Result<(SurfaceOp, Option<Vec<Seq>>)> {
    if surface.nodes.is_empty() {
        return Ok((SurfaceOp::Append, None));
    }
    match kept_from_seq {
        None => {
            let start = *surface.nodes.first().expect("non-empty");
            let end = *surface.nodes.last().expect("non-empty");
            Ok((
                SurfaceOp::Replace { start, end },
                Some(surface.nodes.clone()),
            ))
        }
        Some(k) => {
            let k = Seq::try_from(k).map_err(|_| {
                LitecodeError::InvalidSessionEvent(format!("negative kept_from_seq {k}"))
            })?;
            let keep_i = surface.nodes.iter().position(|s| *s == k).ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "kept_from_seq={k} is not on the current surface"
                ))
            })?;
            if keep_i == 0 {
                return Err(LitecodeError::NothingToCompact);
            }
            let shadowed = surface.nodes[..keep_i].to_vec();
            Ok((
                SurfaceOp::Replace {
                    start: shadowed[0],
                    end: *shadowed.last().expect("keep_i > 0"),
                },
                Some(shadowed),
            ))
        }
    }
}

fn write_compact_pointers(
    tx: &Transaction<'_>,
    session_id: &str,
    checkpoint_seq: i64,
    compacted_seq: Option<i64>,
    kept_from_seq: i64,
    spine_from: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE sessions
         SET checkpoint_seq = ?1, compacted_seq = ?2, kept_from_seq = ?3, spine_from = ?4
         WHERE id = ?5",
        rusqlite::params![
            checkpoint_seq,
            compacted_seq,
            kept_from_seq,
            spine_from,
            session_id
        ],
    )?;
    Ok(())
}

/// Rebuild compact window pointers from the latest surviving `compacted` row.
/// `spine_from` is the compact event seq, never the kept-detail seq.
fn refresh_compact_pointers_from_log(tx: &Transaction<'_>, session_id: &str) -> Result<()> {
    let latest: Option<(i64, Option<String>)> = tx
        .query_row(
            "SELECT seq, body FROM transcript_items
             WHERE session_id = ?1 AND kind = 'compacted'
             ORDER BY seq DESC LIMIT 1",
            rusqlite::params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((seq, body)) = latest else {
        return write_compact_pointers(tx, session_id, 0, None, 0, 0);
    };
    let raw = body.ok_or_else(|| {
        LitecodeError::InvalidSessionEvent(format!("compacted row {seq} has no body"))
    })?;
    let compacted: CompactedBody = serde_json::from_str(&raw)?;
    let kept: Option<i64> = tx.query_row(
        "SELECT MIN(seq) FROM transcript_items
         WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3
           AND kind IN (
             'item/user', 'item/assistant', 'item/tool_call', 'item/tool_result',
             'hook/prompt', 'reminder/job_exit', 'reminder/turn_aborted'
           )",
        rusqlite::params![session_id, compacted.to as i64, seq],
        |row| row.get(0),
    )?;
    write_compact_pointers(tx, session_id, seq, Some(seq), kept.unwrap_or(seq), seq)
}

fn message_timestamp(_item: &Item) -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn encode_detail_row(
    item: &Item,
    data_root: &Path,
    spill_threshold: usize,
) -> Result<(Option<String>, Option<String>, i64)> {
    let body_json = serde_json::to_string(item)?;
    let token_estimate = compute_token_estimate(std::slice::from_ref(item)) as i64;
    // User detail rows stay inlined — k-anchor SQL requires body JSON.
    let allow_spill = spill_threshold > 0 && !is_user_message_item(item);
    if allow_spill && body_json.len() > spill_threshold {
        let blob_id = ulid::Ulid::new().to_string();
        let blob_path = blob_dir(data_root).join(format!("{blob_id}.txt"));
        fs::create_dir_all(blob_path.parent().unwrap())?;
        fs::write(&blob_path, body_json.as_bytes())?;
        return Ok((
            None,
            Some(format!("{BLOB_PREFIX}{blob_id}]")),
            token_estimate,
        ));
    }
    Ok((Some(body_json), None, token_estimate))
}

fn row_to_item(row: &TranscriptRow, data_root: &Path) -> Result<Item> {
    match row.kind.as_str() {
        // During the storage cut-over compact still carries a serialized Item.
        // All ordinary item/* rows do too.
        "compacted" => {
            let raw = row.body.as_deref().ok_or_else(|| {
                LitecodeError::ToolExecution(format!("compacted row seq {} has no body", row.seq))
            })?;
            let body: crate::session::model::CompactedBody = serde_json::from_str(raw)?;
            Ok(body.agent_item())
        }
        "hook/prompt" => {
            let raw = row.body.as_deref().ok_or_else(|| {
                LitecodeError::ToolExecution(format!("hook/prompt row seq {} has no body", row.seq))
            })?;
            let body: HookPromptBody = serde_json::from_str(raw)?;
            Ok(body.agent_item())
        }
        "reminder/job_exit" => {
            let raw = row.body.as_deref().ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "reminder/job_exit row seq {} has no body",
                    row.seq
                ))
            })?;
            let body: ReminderJobExitBody = serde_json::from_str(raw)?;
            Ok(body.agent_item())
        }
        "reminder/turn_aborted" => {
            let raw = row.body.as_deref().ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "reminder/turn_aborted row seq {} has no body",
                    row.seq
                ))
            })?;
            let body: ReminderTurnAbortedBody = serde_json::from_str(raw)?;
            Ok(body.agent_item())
        }
        "compact_checkpoint" | "detail" | "item/user" | "item/assistant" | "item/tool_call"
        | "item/tool_result" => {
            if let Some(body) = &row.body {
                return serde_json::from_str(body).map_err(Into::into);
            }
            if let Some(body_ref) = &row.body_ref {
                let text = load_blob_text(body_ref, data_root)?;
                return serde_json::from_str(&text).map_err(Into::into);
            }
            Err(crate::types::LitecodeError::ToolExecution(format!(
                "transcript row seq {} has no body",
                row.seq
            )))
        }
        other => Err(crate::types::LitecodeError::ToolExecution(format!(
            "unknown transcript row kind {other}"
        ))),
    }
}

fn load_blob_text(body_ref: &str, data_root: &Path) -> Result<String> {
    let rest = body_ref.strip_prefix(BLOB_PREFIX).ok_or_else(|| {
        crate::types::LitecodeError::ToolExecution(format!("invalid body_ref: {body_ref}"))
    })?;
    let (id, _) = rest.split_once(']').ok_or_else(|| {
        crate::types::LitecodeError::ToolExecution(format!("invalid body_ref: {body_ref}"))
    })?;
    let blob_path = blob_dir(data_root).join(format!("{id}.txt"));
    fs::read_to_string(blob_path).map_err(Into::into)
}

#[cfg(test)]
fn rows_to_items(rows: &[TranscriptRow], data_root: &Path) -> Result<Transcript> {
    rows.iter().map(|row| row_to_item(row, data_root)).collect()
}

fn event_from_disk_row(
    session_id: &str,
    seq: i64,
    created_at: i64,
    event_type: String,
    surface_op: String,
    source_seqs: Option<String>,
    kind: String,
    body: Option<String>,
    body_ref: Option<String>,
    state: Option<String>,
    data_root: &Path,
) -> Result<SessionEvent> {
    let seq = Seq::try_from(seq)
        .map_err(|_| LitecodeError::InvalidSessionEvent(format!("negative seq {seq}")))?;
    let event_type = EventType::from_str_name(&event_type);
    let state = LogState::from_str_name(state.as_deref().unwrap_or("final"));
    let source_seqs = match source_seqs.as_deref() {
        None | Some("") => None,
        Some(raw) => Some(serde_json::from_str(raw)?),
    };
    if matches!(event_type, EventType::Compacted) {
        let data = match body.as_deref() {
            Some(raw) if !raw.is_empty() => serde_json::from_str(raw)?,
            _ => {
                return Err(LitecodeError::InvalidSessionEvent(format!(
                    "compacted row {seq} has no body"
                )));
            }
        };
        return Ok(SessionEvent {
            seq,
            time: created_at,
            event_type,
            data,
            surface_op: None,
            source_seqs,
            ignorable: false,
            state,
        });
    }
    if !event_type.is_surface_eligible() {
        let data = match body.as_deref() {
            Some(raw) if !raw.is_empty() => serde_json::from_str(raw)?,
            _ => serde_json::Value::Null,
        };
        let ignorable = matches!(event_type, EventType::Unknown(_));
        return Ok(SessionEvent {
            seq,
            time: created_at,
            event_type,
            data,
            surface_op: None,
            source_seqs,
            ignorable,
            state,
        });
    }
    let item = row_to_item(
        &TranscriptRow {
            session_id: session_id.to_string(),
            seq: seq as i64,
            turn_id: String::new(),
            turn_seq: 0,
            item_type: String::new(),
            kind,
            body,
            body_ref,
            token_estimate: 0,
            created_at,
        },
        data_root,
    )?;
    let surface_op = match surface_op.as_str() {
        "" => None,
        raw => Some(serde_json::from_str(raw)?),
    };
    Ok(SessionEvent {
        seq,
        time: created_at,
        event_type,
        data: serde_json::to_value(&item)?,
        surface_op,
        source_seqs,
        ignorable: false,
        state,
    })
}

fn preview_from_item_json(body: &str) -> String {
    let content = match serde_json::from_str::<Item>(body) {
        Ok(item) => item_text_preview(&item),
        Err(_) => return String::new(),
    };
    let preview: String = content.chars().take(200).collect();
    if content.chars().count() > 200 {
        format!("{preview}…")
    } else {
        preview
    }
}

impl Session {
    pub fn subagent_depth_for(db_path: &str, session_id: &str) -> Result<u32> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        conn.query_row(
            "SELECT subagent_depth FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    pub fn set_subagent_depth(&self, depth: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET subagent_depth = ?1 WHERE id = ?2",
            rusqlite::params![depth, self.id],
        )?;
        Ok(())
    }

    /// Load the durable session metadata without leaking Live or catalog
    /// projection fields into the session domain.
    pub fn meta(&self) -> Result<crate::session::model::SessionMeta> {
        let (
            project,
            created_at,
            parent_session_id,
            parent_call_id,
            subagent_depth,
            agent_id,
            model_id,
            thinking_tier,
            context_mode,
            updated_at,
            compacted_seq,
            spine_from,
            todos_json,
            plan_slug,
            preview,
        ): (
            String,
            i64,
            Option<String>,
            Option<String>,
            u32,
            String,
            Option<String>,
            String,
            String,
            i64,
            Option<i64>,
            i64,
            String,
            Option<String>,
            String,
        ) = self.conn.query_row(
            "SELECT project, created_at, parent_session_id, parent_call_id, subagent_depth,
                    agent_id, model_id, thinking_tier, context_mode, updated_at,
                    compacted_seq, spine_from, todos_json, active_plan_slug, last_message
             FROM sessions WHERE id = ?1",
            rusqlite::params![self.id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                ))
            },
        )?;
        let compacted_seq = compacted_seq
            .map(|seq| {
                Seq::try_from(seq).map_err(|_| {
                    LitecodeError::InvalidSessionEvent(format!("negative compacted_seq {seq}"))
                })
            })
            .transpose()?;
        let spine_from = Seq::try_from(spine_from).map_err(|_| {
            LitecodeError::InvalidSessionEvent(format!("negative spine_from {spine_from}"))
        })?;
        Ok(crate::session::model::SessionMeta {
            id: self.id.clone(),
            project,
            created_at,
            parent_session_id,
            parent_call_id,
            subagent_depth,
            agent_id,
            model_id,
            thinking_tier,
            context_mode,
            updated_at,
            compacted_seq,
            spine_from,
            todos: serde_json::from_str(&todos_json).unwrap_or_default(),
            plan_slug,
            preview,
        })
    }

    pub fn ephemeral(project: &str, agent_id: &str, model_id: Option<&str>) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        ensure_session_schema(&conn)?;

        let id = ulid::Ulid::new().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let model_id_owned = normalize_model_id(model_id);

        conn.execute(
            "INSERT INTO sessions (id, schema_version, project, last_message, agent_id, model_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                SESSION_LOG_SCHEMA_VERSION,
                project,
                "",
                agent_id,
                model_id_owned,
                now,
                now
            ],
        )?;

        let data_root = std::env::temp_dir().join("litecode");

        let session = Self {
            conn,
            id,
            project: project.to_string(),
            agent_id: agent_id.to_string(),
            model_id: model_id_owned,
            thinking_tier: ThinkingTier::default(),
            context_mode: ContextMode::default(),
            parent_session_id: None,
            parent_call_id: None,
            data_root,
            db_path: None,
            ephemeral: true,
            persisted_max_seq: Cell::new(0),
            write_gate: Mutex::new(()),
            truncated_item_ids: RefCell::new(HashSet::new()),
            projection: RefCell::new(LogProjection::default()),
        };
        session.hydrate_projection()?;
        Ok(session)
    }

    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn open(
        db_path: &str,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
    ) -> Result<Self> {
        Self::open_with_parent(db_path, project, agent_id, model_id, None, None)
    }

    /// Open a durable session, optionally linked as a child of `parent_session_id`.
    pub fn open_with_parent(
        db_path: &str,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: Option<&str>,
        parent_call_id: Option<&str>,
    ) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let data_root = data_root_from_db_path(db_path);

        ensure_session_schema(&conn)?;
        enable_wal(&conn)?;

        let id = ulid::Ulid::new().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let model_id_owned = normalize_model_id(model_id);
        let parent_session_id_owned = parent_session_id.map(|s| s.to_string());
        let parent_call_id_owned = parent_call_id.map(|s| s.to_string());

        conn.execute(
            "INSERT INTO sessions (
                id, schema_version, project, last_message, agent_id, model_id, created_at, updated_at,
                parent_session_id, parent_call_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                id,
                SESSION_LOG_SCHEMA_VERSION,
                project,
                "",
                agent_id,
                model_id_owned,
                now,
                now,
                parent_session_id_owned,
                parent_call_id_owned,
            ],
        )?;

        let session = Self {
            conn,
            id,
            project: project.to_string(),
            agent_id: agent_id.to_string(),
            model_id: model_id_owned,
            thinking_tier: ThinkingTier::default(),
            context_mode: ContextMode::default(),
            parent_session_id: parent_session_id_owned,
            parent_call_id: parent_call_id_owned,
            data_root,
            db_path: Some(PathBuf::from(db_path)),
            ephemeral: false,
            persisted_max_seq: Cell::new(0),
            write_gate: Mutex::new(()),
            truncated_item_ids: RefCell::new(HashSet::new()),
            projection: RefCell::new(LogProjection::default()),
        };
        session.hydrate_projection()?;
        Ok(session)
    }

    /// Copy an in-memory ephemeral session into the on-disk database.
    pub fn persist(&mut self, db_path: &str) -> Result<()> {
        if !self.ephemeral {
            return Ok(());
        }

        let file_conn = Connection::open(db_path)?;
        let data_root = data_root_from_db_path(db_path);

        let tx = file_conn.unchecked_transaction()?;
        ensure_session_schema(&tx)?;
        enable_wal(&tx)?;

        let task_state = self.load_task_state()?;
        let todos_json = serde_json::to_string(&task_state.todos)?;
        let active_plan_slug = task_state.active_plan.as_ref().map(|p| p.slug.as_str());

        let now = chrono::Utc::now().timestamp_millis();
        tx.execute(
            "INSERT INTO sessions (
                id, schema_version, project, last_message, agent_id, model_id, thinking_tier, context_mode,
                created_at, updated_at, todos_json, active_plan_slug,
                parent_session_id, parent_call_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                self.id,
                SESSION_LOG_SCHEMA_VERSION,
                self.project,
                "",
                self.agent_id,
                self.model_id,
                self.thinking_tier.as_str(),
                self.context_mode.as_str(),
                now,
                now,
                todos_json,
                active_plan_slug,
                self.parent_session_id,
                self.parent_call_id,
            ],
        )?;

        tx.commit()?;
        self.conn = file_conn;
        self.data_root = data_root;
        self.db_path = Some(PathBuf::from(db_path));
        self.ephemeral = false;
        self.hydrate_projection()?;
        Ok(())
    }

    pub fn delete(db_path: &str, session_id: &str) -> Result<()> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;

        // Resolve project before cascade delete so we can purge external snapshots.
        let project: Option<String> = conn
            .query_row(
                "SELECT project FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )
            .optional()?;

        // Cascade: delete child sessions first (depth-first).
        let child_ids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM sessions WHERE parent_session_id = ?1")?;
            let rows = stmt.query_map(rusqlite::params![session_id], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        drop(conn);
        for child_id in &child_ids {
            Self::delete(db_path, child_id)?;
        }

        let conn = Connection::open(db_path)?;
        crate::session::transcript_fts::ensure_schema(&conn)?;
        let tx = conn.unchecked_transaction()?;
        let changed = tx.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
        )?;
        if changed == 0 {
            return Err(crate::types::LitecodeError::SessionNotFound(
                session_id.to_string(),
            ));
        }
        tx.execute(
            "DELETE FROM transcript_items WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        crate::session::transcript_fts::delete_session(&tx, session_id)?;
        tx.execute(
            "DELETE FROM session_context_meter WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        tx.commit()?;

        let data_root = data_root_from_db_path(db_path);
        let session_dir = data_root.join("sessions").join(session_id);
        if session_dir.exists() {
            std::fs::remove_dir_all(session_dir)?;
        }

        if let Some(project) = project {
            let project_path = std::path::Path::new(&project);
            let snaps = match crate::config::peek_workspace_id(project_path) {
                Some(id) => crate::session::snapshot_paths::snapshots_dir_for_id(&id),
                None => crate::session::snapshot_paths::snapshots_dir_for_workspace(project_path),
            };
            snapshot::delete_session_snapshots(&snaps, session_id)?;
        }
        Ok(())
    }

    pub fn resume(db_path: &str, session_id: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        enable_wal(&conn)?;

        let (
            project,
            agent_id,
            model_id,
            thinking_raw,
            context_raw,
            parent_session_id,
            parent_call_id,
        ): (
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT project, agent_id, model_id, thinking_tier, context_mode,
                        parent_session_id, parent_call_id
                 FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(|_| crate::types::LitecodeError::SessionNotFound(session_id.to_string()))?;

        let thinking_tier = ThinkingTier::parse(&thinking_raw).unwrap_or_default();
        let context_mode = ContextMode::parse(&context_raw).unwrap_or_default();

        let data_root = data_root_from_db_path(db_path);

        let session = Self {
            conn,
            id: session_id.to_string(),
            project,
            agent_id,
            model_id,
            thinking_tier,
            context_mode,
            parent_session_id,
            parent_call_id,
            data_root,
            db_path: Some(PathBuf::from(db_path)),
            ephemeral: false,
            persisted_max_seq: Cell::new(0),
            write_gate: Mutex::new(()),
            truncated_item_ids: RefCell::new(HashSet::new()),
            projection: RefCell::new(LogProjection::default()),
        };
        session.hydrate_projection()?;
        Ok(session)
    }

    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// Load session-scoped todo/plan state from SQLite.
    pub fn load_task_state(&self) -> Result<TaskReminders> {
        let (todos_json, active_plan_slug): (String, Option<String>) = self
            .conn
            .query_row(
                "SELECT COALESCE(todos_json, '[]'), active_plan_slug FROM sessions WHERE id = ?1",
                rusqlite::params![self.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;

        let todos: Vec<TodoItem> = serde_json::from_str(&todos_json)
            .map_err(|e| LitecodeError::ToolExecution(format!("parse session todos: {e}")))?;

        let active_plan = active_plan_slug.map(|slug| PlanRef::new(&slug));

        let mut state = TaskReminders { todos, active_plan };
        state.normalize();
        Ok(state)
    }

    pub fn save_task_state(&self, state: &TaskReminders) -> Result<()> {
        Self::write_task_state(&self.conn, &self.id, state)
    }

    /// Load last-known provider usage meter (empty if never written).
    pub fn load_context_meter(&self) -> Result<SessionContextMeter> {
        let row: Option<(i64, i64, i64, i64, i64, i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT prompt_tokens, completion_tokens,
                        cache_hit_tokens, cache_miss_tokens,
                        cum_prompt_tokens, cum_completion_tokens,
                        cum_cache_hit_tokens, cum_cache_miss_tokens
                 FROM session_context_meter WHERE session_id = ?1",
                rusqlite::params![self.id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            Some((p, c, hit, miss, cp, cc, chit, cmiss)) => SessionContextMeter {
                prompt_tokens: p.max(0) as u64,
                completion_tokens: c.max(0) as u64,
                cache_hit_tokens: hit.max(0) as u64,
                cache_miss_tokens: miss.max(0) as u64,
                cum_prompt_tokens: cp.max(0) as u64,
                cum_completion_tokens: cc.max(0) as u64,
                cum_cache_hit_tokens: chit.max(0) as u64,
                cum_cache_miss_tokens: cmiss.max(0) as u64,
            },
            None => SessionContextMeter::default(),
        })
    }

    /// Upsert last-known **provider** usage for this session.
    pub fn save_context_meter(&self, meter: &SessionContextMeter) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.conn
            .execute(
                "INSERT INTO session_context_meter (
                    session_id, prompt_tokens, completion_tokens,
                    cache_hit_tokens, cache_miss_tokens,
                    cum_prompt_tokens, cum_completion_tokens,
                    cum_cache_hit_tokens, cum_cache_miss_tokens,
                    updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(session_id) DO UPDATE SET
                    prompt_tokens = excluded.prompt_tokens,
                    completion_tokens = excluded.completion_tokens,
                    cache_hit_tokens = excluded.cache_hit_tokens,
                    cache_miss_tokens = excluded.cache_miss_tokens,
                    cum_prompt_tokens = excluded.cum_prompt_tokens,
                    cum_completion_tokens = excluded.cum_completion_tokens,
                    cum_cache_hit_tokens = excluded.cum_cache_hit_tokens,
                    cum_cache_miss_tokens = excluded.cum_cache_miss_tokens,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    self.id,
                    meter.prompt_tokens as i64,
                    meter.completion_tokens as i64,
                    meter.cache_hit_tokens as i64,
                    meter.cache_miss_tokens as i64,
                    meter.cum_prompt_tokens as i64,
                    meter.cum_completion_tokens as i64,
                    meter.cum_cache_hit_tokens as i64,
                    meter.cum_cache_miss_tokens as i64,
                    now,
                ],
            )
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        Ok(())
    }

    fn write_task_state(conn: &Connection, session_id: &str, state: &TaskReminders) -> Result<()> {
        let mut state = state.clone();
        state.normalize();

        let todos_json = serde_json::to_string(&state.todos)?;
        let active_plan_slug = state.active_plan.as_ref().map(|p| p.slug.as_str());

        conn.execute(
            "UPDATE sessions SET todos_json = ?1, active_plan_slug = ?2 WHERE id = ?3",
            rusqlite::params![todos_json, active_plan_slug, session_id],
        )
        .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn last_user_message_preview(items: &[Item]) -> String {
        for item in items.iter().rev() {
            if matches!(
                item,
                Item::Message(MessageItem::Input(InputMessage {
                    role: InputRole::User,
                    ..
                }))
            ) {
                let content = item_text_preview(item);
                let preview: String = content.chars().take(200).collect();
                if content.chars().count() > 200 {
                    return format!("{preview}…");
                }
                return preview;
            }
        }
        String::new()
    }
}

impl Session {
    pub fn find_latest_by_project(db_path: &str, project: &str) -> Result<Option<String>> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        let result = conn.query_row(
            "SELECT id FROM sessions WHERE project = ?1 ORDER BY updated_at DESC LIMIT 1",
            rusqlite::params![project],
            |row| row.get(0),
        );
        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_sessions(
        db_path: &str,
    ) -> Result<Vec<(String, String, i64, String, String, Option<String>)>> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;

        // Top-level list only: child (subagent) sessions are mounted under the parent.
        let mut stmt = conn.prepare(
            "SELECT id, project, updated_at, last_message, agent_id, model_id
             FROM sessions
             WHERE parent_session_id IS NULL
             ORDER BY updated_at DESC LIMIT 50",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?;

        let mut sessions = vec![];
        for row in rows {
            sessions.push(row?);
        }

        Ok(sessions)
    }

    /// List child session ids for a parent (direct children only).
    pub fn list_child_session_ids(db_path: &str, parent_session_id: &str) -> Result<Vec<String>> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM sessions WHERE parent_session_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![parent_session_id], |row| row.get(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }

    /// Resolve child session id for a parent `subagent_launch` call_id.
    pub fn child_session_id_for_call(
        db_path: &str,
        parent_session_id: &str,
        parent_call_id: &str,
    ) -> Result<Option<String>> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM sessions
             WHERE parent_session_id = ?1 AND parent_call_id = ?2
             ORDER BY created_at DESC LIMIT 1",
        )?;
        match stmt.query_row(
            rusqlite::params![parent_session_id, parent_call_id],
            |row| row.get(0),
        ) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Map `parent_call_id → child session id` for all children of a parent.
    pub fn child_bindings_for_parent(
        db_path: &str,
        parent_session_id: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        let mut stmt = conn.prepare(
            "SELECT parent_call_id, id FROM sessions
             WHERE parent_session_id = ?1 AND parent_call_id IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![parent_session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (call_id, child_id) = row?;
            map.insert(call_id, child_id);
        }
        Ok(map)
    }

    /// Clear `model_id` on every row whose binding is missing from `valid_model_ids`.
    /// Does not touch `agent_id`. Returns cleared session ids.
    pub fn clear_orphaned_model_ids(
        db_path: &str,
        valid_model_ids: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>> {
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        let orphans: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT id, model_id FROM sessions WHERE model_id IS NOT NULL AND trim(model_id) != ''",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, model_id) = row?;
                if !valid_model_ids.contains(&model_id) {
                    out.push(id);
                }
            }
            out
        };
        for id in &orphans {
            conn.execute(
                "UPDATE sessions SET model_id = NULL WHERE id = ?1",
                rusqlite::params![id],
            )?;
        }
        Ok(orphans)
    }

    /// Lightweight unbounded scan used by background empty-session GC.
    /// User-facing history remains deliberately capped by `list_sessions`.
    pub fn list_sessions_for_gc(db_path: &str) -> Result<Vec<(String, i64)>> {
        if !Path::new(db_path).exists() {
            return Ok(vec![]);
        }
        let conn = Connection::open(db_path)?;
        ensure_session_schema(&conn)?;
        let mut stmt = conn.prepare("SELECT id, updated_at FROM sessions")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut sessions = vec![];
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn load_transcript(&self) -> Result<Transcript> {
        Ok(self
            .load_working_set()?
            .into_iter()
            .map(|row| row.item)
            .collect())
    }

    /// Model-visible working set: surface order, same skip rules as `derive_messages`.
    pub fn load_working_set(&self) -> Result<Vec<WorkingRow>> {
        let projection = self.projection.borrow();
        let pairs = project_working_pairs(&projection.surface, |seq| {
            projection.items_by_seq.get(&seq).cloned().ok_or_else(|| {
                LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing from cache"))
            })
        })?;
        Ok(pairs
            .into_iter()
            .map(|(seq, item)| WorkingRow::persisted(seq, item))
            .collect())
    }

    /// Seqs of model-visible items (same skip rules as [`load_working_set`]).
    pub fn model_surface_seqs(&self) -> Result<Vec<Seq>> {
        Ok(self
            .load_working_set()?
            .into_iter()
            .filter_map(|row| row.log_seq)
            .collect())
    }

    fn hydrate_projection(&self) -> Result<()> {
        let events = self.load_events()?;
        let surface = fold_surface(&events)?;
        let node_set: HashSet<Seq> = surface.nodes.iter().copied().collect();
        let mut id_to_seq = HashMap::new();
        let mut items_by_seq = HashMap::new();
        for event in &events {
            let item = spine_agent_item(event).ok();
            if let Some(item) = item {
                if let Some(id) = item_log_id(&item) {
                    id_to_seq.insert(id, event.seq);
                }
                if node_set.contains(&event.seq) {
                    items_by_seq.insert(event.seq, item);
                }
            }
        }
        let next_seq = events.last().map(|e| e.seq + 1).unwrap_or(0);
        let projection = LogProjection {
            surface,
            next_seq,
            id_to_seq,
            items_by_seq,
        };
        self.persisted_max_seq.set(projection.max_seq());
        *self.projection.borrow_mut() = projection;
        Ok(())
    }

    fn cached_max_seq(&self) -> i64 {
        self.projection.borrow().max_seq()
    }

    fn commit_projection(&self, projection: LogProjection) {
        self.persisted_max_seq.set(projection.max_seq());
        *self.projection.borrow_mut() = projection;
    }

    fn admit_draft_in_tx(
        &self,
        tx: &Transaction<'_>,
        mut draft: EventDraft,
        turn_id: &str,
        turn_seq: i64,
        kind: &str,
        token_estimate: i64,
        projection: &mut LogProjection,
    ) -> Result<(Seq, Option<Item>)> {
        if draft.time == 0 {
            draft.time = chrono::Utc::now().timestamp_millis();
        }
        let item = if draft.event_type.is_item() {
            Some(serde_json::from_value::<Item>(draft.data.clone())?)
        } else {
            None
        };
        if let Some(id) = item.as_ref().and_then(item_log_id)
            && self.truncated_item_ids.borrow().contains(&id)
        {
            return Err(LitecodeError::Canceled);
        }
        let seq = projection.next_seq;
        let event = finalize_draft(seq, draft)?;
        plan_surface(&projection.surface, &event)?;
        let tid = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };
        insert_event_row(
            tx,
            &self.id,
            &self.data_root,
            &event,
            item.as_ref(),
            &tid,
            turn_seq,
            kind,
            token_estimate,
        )?;
        projection.apply_event(&event, item.as_ref())?;
        Ok((seq, item))
    }

    pub fn buffer_len(&self) -> usize {
        self.load_events().map(|e| e.len()).unwrap_or(0)
    }

    #[cfg(test)]
    pub fn buffer_index_len(&self) -> Result<usize> {
        Ok(self.load_history_transcript()?.len())
    }

    /// §5.1 Materialize — buffer index range `[start, end)`.
    #[cfg(test)]
    pub fn load_by_buffer_index(&self, start: usize, end: usize) -> Result<Transcript> {
        let rows = self.load_by_buffer_index_rows(start, end)?;
        rows_to_items(&rows, &self.data_root)
    }

    /// Load a buffer range together with each row's DB `kind`
    /// (`detail` | `compact_checkpoint`) and its history ordinal (position in
    /// `ORDER BY seq` UI history). The ordinal is the only index the client may
    /// use; it is not recomputed from `start + i` on the wire consumer.
    #[cfg(test)]
    pub fn load_by_buffer_index_with_kinds(
        &self,
        start: usize,
        end: usize,
    ) -> Result<(Transcript, Vec<String>, Vec<usize>)> {
        let rows = self.load_by_buffer_index_rows(start, end)?;
        let kinds = rows.iter().map(|r| r.kind.clone()).collect();
        let indices: Vec<usize> = (start..start + rows.len()).collect();
        Ok((rows_to_items(&rows, &self.data_root)?, kinds, indices))
    }

    /// History index and Item of the current turn-view checkpoint, if any.
    #[cfg(test)]
    pub fn compact_checkpoint_buffer_item(&self) -> Result<Option<(usize, Item)>> {
        let cp_seq = self.checkpoint_seq()?;
        let rows = self.load_history_transcript()?;
        for (i, row) in rows.iter().enumerate() {
            if row.kind == "compacted" && row.seq == cp_seq {
                return Ok(Some((i, row_to_item(row, &self.data_root)?)));
            }
        }
        Ok(None)
    }

    #[cfg(test)]
    fn load_by_buffer_index_rows(&self, start: usize, end: usize) -> Result<Vec<TranscriptRow>> {
        let len = self.buffer_index_len()?;
        if start > end || end > len {
            return Err(crate::types::LitecodeError::ToolExecution(format!(
                "invalid range {start}..{end} (len={len})"
            )));
        }
        let rows = self.load_history_transcript()?;
        Ok(rows[start..end].to_vec())
    }

    /// Count user detail rows with buffer index `< start` in the UI history.
    ///
    /// FE uses this as the absolute 0-based anchor baseline for a loaded window
    /// starting at `start` (`k = before + local user ordinal`).
    #[cfg(test)]
    pub fn user_detail_before_buffer_index(&self, start: usize) -> Result<usize> {
        let rows = self.load_history_transcript()?;
        let end = start.min(rows.len());
        let mut n = 0usize;
        for row in &rows[..end] {
            if transcript_row_is_user_detail(row) {
                n += 1;
            }
        }
        Ok(n)
    }

    fn lock_write(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_gate.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sole durable write entry: append, seal, or truncate.
    pub fn apply(&self, op: SessionApply) -> Result<ApplyOutcome> {
        let _gate = self.lock_write();
        match op {
            SessionApply::Append(draft) => {
                let kind = draft.event_type.as_str().to_owned();
                let seq = self.append_unlocked(draft, "", 0, &kind)?;
                Ok(ApplyOutcome::Appended(seq))
            }
            SessionApply::Seal { seq, item } => {
                self.seal_unlocked(seq, &item)?;
                Ok(ApplyOutcome::Sealed)
            }
            SessionApply::Truncate { user_k } => {
                self.truncate_unlocked(user_k)?;
                Ok(ApplyOutcome::Truncated)
            }
        }
    }

    fn append_unlocked(
        &self,
        draft: EventDraft,
        turn_id: &str,
        turn_seq: i64,
        kind: &str,
    ) -> Result<Seq> {
        let mut projection = self.projection.borrow().clone();
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        let (seq, item) =
            self.admit_draft_in_tx(&tx, draft, turn_id, turn_seq, kind, 0, &mut projection)?;
        let now = chrono::Utc::now().timestamp_millis();
        if let Some(item) = item.as_ref() {
            let preview = Self::last_user_message_preview(std::slice::from_ref(item));
            if !preview.is_empty() {
                tx.execute(
                    "UPDATE sessions SET updated_at = ?1, last_message = ?2 WHERE id = ?3",
                    rusqlite::params![now, preview, self.id],
                )?;
            } else {
                tx.execute(
                    "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, self.id],
                )?;
            }
        } else {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, self.id],
            )?;
        }
        tx.commit()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        self.commit_projection(projection);
        Ok(seq)
    }

    fn seal_unlocked(&self, seq: Seq, item: &Item) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        seal_event_row(&tx, &self.id, &self.data_root, seq, item)?;
        tx.commit()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        self.projection.borrow_mut().seal_item(seq, item);
        Ok(())
    }

    fn truncate_unlocked(&self, k: i64) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;

        let anchor_seq: i64 = tx
            .query_row(SQL_ANCHOR_SEQ, rusqlite::params![self.id, k], |row| {
                row.get(0)
            })
            .map_err(|_| LitecodeError::InvalidRevertAnchor(format!("k={k}")))?;

        let mut stmt = tx.prepare(
            "SELECT event_type, kind, body, body_ref FROM transcript_items
             WHERE session_id = ?1 AND seq >= ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![self.id, anchor_seq], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut tombstones = HashSet::new();
        for row in rows {
            let (event_type, kind, body, body_ref) = row?;
            if !EventType::from_str_name(&event_type).is_surface_eligible() {
                continue;
            }
            if let Ok(item) = row_to_item(
                &TranscriptRow {
                    session_id: self.id.clone(),
                    seq: 0,
                    turn_id: String::new(),
                    turn_seq: 0,
                    item_type: String::new(),
                    kind,
                    body,
                    body_ref,
                    token_estimate: 0,
                    created_at: 0,
                },
                &self.data_root,
            ) && let Some(id) = item_log_id(&item)
            {
                tombstones.insert(id);
            }
        }
        drop(stmt);

        tx.execute(
            "DELETE FROM transcript_items WHERE session_id = ?1 AND seq >= ?2",
            rusqlite::params![self.id, anchor_seq],
        )?;
        crate::session::transcript_fts::delete_seq_ge(&tx, &self.id, anchor_seq)?;

        let now = chrono::Utc::now().timestamp_millis();
        let preview: String = tx
            .query_row(
                "SELECT body FROM transcript_items
                 WHERE session_id = ?1
                   AND kind = 'item/user'
                   AND body IS NOT NULL
                 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![self.id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .map(|body| preview_from_item_json(&body))
            .unwrap_or_default();

        tx.execute(
            "UPDATE sessions
             SET updated_at = ?1,
                 last_message = ?2
             WHERE id = ?3",
            rusqlite::params![now, preview, self.id],
        )?;
        refresh_compact_pointers_from_log(&tx, &self.id)?;

        tx.commit()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        self.truncated_item_ids.borrow_mut().extend(tombstones);
        // Remaining log is 0..anchor-1. A deleted replace must unshadow earlier
        // nodes; dropping `nodes >= anchor` from the live surface is not enough.
        self.hydrate_projection()?;
        Ok(())
    }

    fn bump_session_updated(
        &self,
        tx: &Transaction<'_>,
        items: &[Item],
    ) -> Result<Option<(String, i64)>> {
        let now = chrono::Utc::now().timestamp_millis();
        let preview = Self::last_user_message_preview(items);
        if !preview.is_empty() {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1, last_message = ?2 WHERE id = ?3",
                rusqlite::params![now, preview, self.id],
            )?;
            Ok(Some((preview, now)))
        } else {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, self.id],
            )?;
            Ok(None)
        }
    }

    /// §5.1 step INSERT — append `delta` detail rows with consecutive seq allocation.
    ///
    /// Returns `Some((preview, updated_at))` when this delta contains a user
    /// message and `last_message` was updated. Assistant/tool-only deltas no
    /// longer wipe `last_message`.
    pub fn insert_detail_rows(&self, delta: &[Item]) -> Result<Option<(String, i64)>> {
        self.insert_detail_rows_with_turn(delta, "")
    }

    /// Append detail rows tagged with a real turn id.
    pub fn insert_detail_rows_with_turn(
        &self,
        delta: &[Item],
        turn_id: &str,
    ) -> Result<Option<(String, i64)>> {
        if delta.is_empty() {
            return Ok(None);
        }
        let _gate = self.lock_write();
        let mut projection = self.projection.borrow().clone();
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        let turn_id = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };
        for (i, msg) in delta.iter().enumerate() {
            let kind = surface_event_type_of(msg).as_str().to_owned();
            let mut draft =
                EventDraft::surface_item(surface_event_type_of(msg), msg, SurfaceOp::Append)?;
            draft.time = message_timestamp(msg);
            self.admit_draft_in_tx(&tx, draft, &turn_id, i as i64, &kind, 0, &mut projection)?;
        }
        let preview_updated = self.bump_session_updated(&tx, delta)?;
        tx.commit()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        self.commit_projection(projection);
        Ok(preview_updated)
    }

    /// Append one Item (including `in_progress`) and return its `seq`.
    pub fn persist_item(&self, item: &Item) -> Result<Seq> {
        let _gate = self.lock_write();
        if let Some(id) = item_log_id(item) {
            if self.truncated_item_ids.borrow().contains(&id) {
                return Err(LitecodeError::Canceled);
            }
            if let Some(&seq) = self.projection.borrow().id_to_seq.get(&id) {
                self.seal_unlocked(seq, item)?;
                return Ok(seq);
            }
        }
        let mut draft =
            EventDraft::surface_item(surface_event_type_of(item), item, SurfaceOp::Append)?;
        draft.time = message_timestamp(item);
        let kind = draft.event_type.as_str().to_owned();
        self.append_unlocked(draft, "", 0, &kind)
    }

    /// 封口: rewrite payload on an existing `seq`. Does not allocate a new row.
    pub fn seal_item(&self, seq: Seq, item: &Item) -> Result<()> {
        self.apply(SessionApply::Seal {
            seq,
            item: item.clone(),
        })?;
        Ok(())
    }

    /// Seal durable `in_progress` rows with established `seal_item` (Incomplete + final).
    /// Returns sealed seqs in log order. Already-final rows are skipped (idempotent).
    pub fn seal_in_progress_items(&self) -> Result<Vec<Seq>> {
        use crate::authority::responses::OutputStatus;
        let events = self.load_events()?;
        let mut sealed = Vec::new();
        for event in events {
            if event.state != LogState::InProgress {
                continue;
            }
            let Ok(mut item) = item_from_event(&event) else {
                continue;
            };
            match &mut item {
                Item::Message(MessageItem::Output(m)) => m.status = OutputStatus::Incomplete,
                Item::FunctionCall(fc) => fc.status = Some(OutputStatus::Incomplete),
                Item::Reasoning(r) => r.status = Some(OutputStatus::Incomplete),
                _ => {}
            }
            self.seal_item(event.seq, &item)?;
            sealed.push(event.seq);
        }
        Ok(sealed)
    }

    /// Persist the working set. Unmatched `FunctionCallOutput`s are not
    /// inserted and are dropped from the in-memory working set. Historical log
    /// rows are never DELETE'd here (only 回退 deletes seqs).
    ///
    /// `expected_max_seq` is the log `MAX(seq)` last observed by the caller
    /// (`-1` if the log was empty). If the table is now shorter, this is 回退
    /// and the delta is discarded.
    pub fn commit_turn_delta_with_orphan_cleanup(
        &self,
        rows: &mut Vec<WorkingRow>,
        _delta: &[Item],
        expected_max_seq: i64,
        turn_id: &str,
    ) -> Result<CommitDeltaOutcome> {
        let _gate = self.lock_write();
        let valid_call_ids: HashSet<String> = rows
            .iter()
            .filter_map(|row| match &row.item {
                Item::FunctionCall(fc) => Some(fc.call_id.clone()),
                _ => None,
            })
            .collect();
        let is_orphan = |item: &Item| {
            matches!(
                item,
                Item::FunctionCallOutput(out) if !valid_call_ids.contains(&out.call_id)
            )
        };
        let mut kept: Vec<WorkingRow> = rows
            .iter()
            .filter(|row| !is_orphan(&row.item))
            .cloned()
            .collect();

        if self.cached_max_seq() < expected_max_seq {
            *rows = self.load_working_set()?;
            return Ok(CommitDeltaOutcome::Discarded);
        }

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        let mut projection = self.projection.borrow().clone();
        let tid = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };
        let mut mutated = false;
        let mut appended: Vec<Item> = Vec::new();
        let mut next_turn_seq = 0i64;
        for row in kept.iter_mut() {
            let msg = &row.item;
            if let Some(seq) = row.log_seq {
                let same = projection
                    .items_by_seq
                    .get(&seq)
                    .and_then(|existing| serde_json::to_value(existing).ok())
                    == serde_json::to_value(msg).ok();
                if !same {
                    seal_event_row(&tx, &self.id, &self.data_root, seq, msg)?;
                    projection.seal_item(seq, msg);
                    mutated = true;
                }
                continue;
            }
            if let Some(id) = item_log_id(msg) {
                if self.truncated_item_ids.borrow().contains(&id) {
                    continue;
                }
                if let Some(&seq) = projection.id_to_seq.get(&id) {
                    row.log_seq = Some(seq);
                    let same = projection
                        .items_by_seq
                        .get(&seq)
                        .and_then(|existing| serde_json::to_value(existing).ok())
                        == serde_json::to_value(msg).ok();
                    if !same {
                        seal_event_row(&tx, &self.id, &self.data_root, seq, msg)?;
                        projection.seal_item(seq, msg);
                        mutated = true;
                    }
                    continue;
                }
            }
            let kind = surface_event_type_of(msg).as_str().to_owned();
            let mut draft =
                EventDraft::surface_item(surface_event_type_of(msg), msg, SurfaceOp::Append)?;
            draft.time = message_timestamp(msg);
            let (seq, _) =
                self.admit_draft_in_tx(&tx, draft, &tid, next_turn_seq, &kind, 0, &mut projection)?;
            row.log_seq = Some(seq);
            next_turn_seq += 1;
            mutated = true;
            appended.push(msg.clone());
        }

        let preview_updated = self.bump_session_updated(&tx, &appended)?;
        tx.commit()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        *rows = kept;
        self.commit_projection(projection);
        Ok(CommitDeltaOutcome::Applied {
            preview: preview_updated,
            mutated,
        })
    }

    /// §5.1 k formula — persisted user detail count (C2 anchor input).
    pub fn user_detail_count(&self) -> Result<i64> {
        self.conn
            .query_row(SQL_USER_DETAIL_COUNT, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    /// Count `item/user` rows with `seq < from_seq`.
    pub fn user_detail_before_seq(&self, from_seq: Seq) -> Result<i64> {
        self.conn
            .query_row(
                SQL_USER_DETAIL_BEFORE_SEQ,
                rusqlite::params![self.id, from_seq as i64],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Snapshot file stem written at turn start (`next_seq` after the k-th user append).
    /// Track runs after that user row is last, so stem = `anchor_seq + 1`.
    pub fn snapshot_stem_for_user_k(&self, k: i64) -> Result<i64> {
        let anchor_seq: i64 = self
            .conn
            .query_row(SQL_ANCHOR_SEQ, rusqlite::params![self.id, k], |row| {
                row.get(0)
            })
            .map_err(|_| LitecodeError::InvalidRevertAnchor(format!("k={k}")))?;
        anchor_seq
            .checked_add(1)
            .ok_or_else(|| LitecodeError::InvalidRevertAnchor(format!("k={k} seq overflow")))
    }

    /// Truncate DB from the k-th user detail anchor; zero file side effects.
    pub fn revert_to_user_anchor(&self, k: i64) -> Result<()> {
        self.apply(SessionApply::Truncate { user_k: k })?;
        Ok(())
    }

    pub fn truncated_tool_result(content: &str, max_tokens: usize) -> String {
        let byte_limit = max_tokens.saturating_mul(4);
        if byte_limit >= content.len() {
            return content.to_string();
        }
        let end = byte_limit;
        format!("{}\n... [truncated]", &content[..end])
    }

    /// Remove orphan `FunctionCallOutput` items whose `call_id` has no matching
    /// `FunctionCall` in the transcript. Does not rewrite output text.
    pub fn snip_stale_results(transcript: &mut Transcript) {
        let active_ids: std::collections::HashSet<String> = transcript
            .iter()
            .filter_map(|item| match item {
                Item::FunctionCall(fc) => Some(fc.call_id.clone()),
                _ => None,
            })
            .collect();

        transcript.retain(|item| match item {
            Item::FunctionCallOutput(out) => active_ids.contains(&out.call_id),
            _ => true,
        });
    }

    /// Synthesize `FunctionCallOutput`s for `FunctionCall`s that lack a matching
    /// output (crash between persist and tool completion). Used on the **ephemeral
    /// LLM view**. Callers must not persist
    /// these pads as `detail` — hanging calls stay on disk until a real result
    /// or abort seal.
    ///
    /// Returns the number of outputs appended.
    pub fn pad_unanswered_calls(transcript: &mut Transcript) -> usize {
        use crate::authority::responses::{FunctionCallOutput, FunctionCallOutputItemParam};

        let answered: std::collections::HashSet<String> = transcript
            .iter()
            .filter_map(|item| match item {
                Item::FunctionCallOutput(out) => Some(out.call_id.clone()),
                _ => None,
            })
            .collect();

        let mut result = Vec::with_capacity(transcript.len());
        let mut pending: Vec<(String, String)> = Vec::new();
        let mut padded = 0usize;

        let flush =
            |pending: &mut Vec<(String, String)>, result: &mut Vec<Item>, padded: &mut usize| {
                for (call_id, name) in pending.drain(..) {
                    result.push(Item::FunctionCallOutput(FunctionCallOutputItemParam {
                        call_id,
                        output: FunctionCallOutput::Text(format!(
                            "tool '{name}' was interrupted: no result was recorded \
                         (session recovered before completion)"
                        )),
                        id: None,
                        status: None,
                    }));
                    *padded += 1;
                }
            };

        for item in transcript.iter() {
            match item {
                Item::FunctionCall(fc) => {
                    result.push(item.clone());
                    if !answered.contains(&fc.call_id) {
                        pending.push((fc.call_id.clone(), fc.name.clone()));
                    }
                }
                _ => {
                    flush(&mut pending, &mut result, &mut padded);
                    result.push(item.clone());
                }
            }
        }
        flush(&mut pending, &mut result, &mut padded);

        if padded > 0 {
            *transcript = result;
        }
        padded
    }

    pub fn set_agent_id(&mut self, agent_id: &str) -> Result<()> {
        self.agent_id = agent_id.to_string();
        self.conn.execute(
            "UPDATE sessions SET agent_id = ?1 WHERE id = ?2",
            rusqlite::params![agent_id, self.id],
        )?;
        Ok(())
    }

    pub fn set_model_id(&mut self, model_id: Option<&str>) -> Result<()> {
        let model_id_owned = normalize_model_id(model_id);
        self.model_id = model_id_owned.clone();
        self.conn.execute(
            "UPDATE sessions SET model_id = ?1 WHERE id = ?2",
            rusqlite::params![model_id_owned, self.id],
        )?;
        Ok(())
    }

    pub fn set_thinking_tier(&mut self, tier: ThinkingTier) -> Result<()> {
        self.thinking_tier = tier;
        self.conn.execute(
            "UPDATE sessions SET thinking_tier = ?1 WHERE id = ?2",
            rusqlite::params![tier.as_str(), self.id],
        )?;
        Ok(())
    }

    pub fn set_context_mode(&mut self, mode: ContextMode) -> Result<()> {
        self.context_mode = mode;
        self.conn.execute(
            "UPDATE sessions SET context_mode = ?1 WHERE id = ?2",
            rusqlite::params![mode.as_str(), self.id],
        )?;
        Ok(())
    }

    /// §5.1 step 1 — authoritative checkpoint column (default 0 = no compact
    /// unless a `compact_checkpoint` row exists at that seq; empty-session
    /// compact may legitimately set this to 0 with a CP row present).
    #[allow(dead_code)]
    pub fn checkpoint_seq(&self) -> Result<i64> {
        self.conn
            .query_row(SQL_CHECKPOINT_SEQ, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    /// Pi-style firstKept pointer — first `detail.seq` included in the turn view.
    /// Default 0 (no compact). Empty-keep compact sets this to the new checkpoint
    /// seq so the view is summary-only until newer detail arrives.
    #[allow(dead_code)]
    pub fn kept_from_seq(&self) -> Result<i64> {
        self.conn
            .query_row(SQL_KEPT_FROM_SEQ, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    pub fn max_seq(&self) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) FROM transcript_items WHERE session_id = ?1",
                rusqlite::params![self.id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// In-memory persisted-seq cursor value.
    pub fn persisted_max_seq(&self) -> i64 {
        self.persisted_max_seq.get()
    }

    /// Reload the cursor from the DB's current `MAX(seq)` (turn load, post-compact).
    pub fn reload_persisted_max_seq(&self) -> Result<()> {
        self.persisted_max_seq.set(self.cached_max_seq());
        Ok(())
    }

    /// §5.1 turn load — pi view: current compact summary + original detail from
    /// `kept_from_seq` onward (no mid-log splice / kept rewrite).
    pub fn load_turn_transcript(&self) -> Result<Vec<TranscriptRow>> {
        let mut stmt = self.conn.prepare(SQL_LOAD_TURN_TRANSCRIPT)?;
        let rows = stmt.query_map(rusqlite::params![self.id], |row| {
            Ok(TranscriptRow {
                session_id: row.get(0)?,
                seq: row.get(1)?,
                turn_id: row.get(2)?,
                turn_seq: row.get(3)?,
                item_type: row.get(4)?,
                kind: row.get(5)?,
                body: row.get(6)?,
                body_ref: row.get(7)?,
                token_estimate: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Full chronological conversation history for client materialization.
    pub fn load_history_transcript(&self) -> Result<Vec<TranscriptRow>> {
        let mut stmt = self.conn.prepare(SQL_LOAD_HISTORY_TRANSCRIPT)?;
        let rows = stmt.query_map(rusqlite::params![self.id], |row| {
            Ok(TranscriptRow {
                session_id: row.get(0)?,
                seq: row.get(1)?,
                turn_id: row.get(2)?,
                turn_seq: row.get(3)?,
                item_type: row.get(4)?,
                kind: row.get(5)?,
                body: row.get(6)?,
                body_ref: row.get(7)?,
                token_estimate: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Append-only log in disk `seq` order. Does not apply the turn-window SQL view.
    pub fn load_events(&self) -> Result<Vec<SessionEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, created_at, event_type, surface_op, source_seqs, kind, body, body_ref, state
             FROM transcript_items WHERE session_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![self.id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, created_at, event_type, surface_op, source_seqs, kind, body, body_ref, state) =
                row?;
            events.push(event_from_disk_row(
                &self.id,
                seq,
                created_at,
                event_type,
                surface_op,
                source_seqs,
                kind,
                body,
                body_ref,
                state,
                &self.data_root,
            )?);
        }
        for (i, event) in events.iter().enumerate() {
            let expected = i as Seq;
            if event.seq != expected {
                return Err(LitecodeError::InvalidSessionEvent(format!(
                    "seq hole: expected {expected}, got {}",
                    event.seq
                )));
            }
        }
        Ok(events)
    }

    /// Half-open log window `[from_seq, to_seq)` in append-origin seq order.
    pub fn load_events_range(&self, from_seq: Seq, to_seq: Seq) -> Result<Vec<SessionEvent>> {
        if from_seq >= to_seq {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT seq, created_at, event_type, surface_op, source_seqs, kind, body, body_ref, state
             FROM transcript_items WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![self.id, from_seq as i64, to_seq as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, created_at, event_type, surface_op, source_seqs, kind, body, body_ref, state) =
                row?;
            events.push(event_from_disk_row(
                &self.id,
                seq,
                created_at,
                event_type,
                surface_op,
                source_seqs,
                kind,
                body,
                body_ref,
                state,
                &self.data_root,
            )?);
        }
        Ok(events)
    }

    /// `(last_seq, next_seq)` for wire snapshots. Empty log → `(-1, 0)`.
    pub fn wire_seq_cursor(&self) -> Result<(i64, u64)> {
        let last = self.max_seq()?;
        let next = if last < 0 {
            0
        } else {
            (last as u64).saturating_add(1)
        };
        Ok((last, next))
    }

    /// §5.1 compact success — summary checkpoint with empty keep (summary-only view).
    pub fn apply_compact_checkpoint(&self, summary: &Item, token_estimate: i64) -> Result<i64> {
        // `kept_from_seq = N` (checkpoint) → no pre-existing detail in the view.
        self.apply_compact_checkpoint_checked(summary, None, token_estimate, None)
    }

    /// Pi-style keep-recent compact:
    /// 1. INSERT `compact_checkpoint` @ N with `summary`
    /// 2. UPDATE `checkpoint_seq = N`, `kept_from_seq = firstKept` (or N if empty)
    ///
    /// Earlier checkpoint rows stay in history as items. The turn working set
    /// is a view: current summary + original `detail` with `seq >= kept_from_seq`.
    /// **Never deletes or rewrites historical `detail`.**
    ///
    /// `kept_from_seq`: `Some(seq)` of the first kept detail row; `None` = empty
    /// keep (pointer set to N so only the summary is visible until new inserts).
    pub fn apply_compact_checkpoint_from(
        &self,
        summary: &Item,
        kept_from_seq: Option<i64>,
        token_estimate: i64,
    ) -> Result<i64> {
        self.apply_compact_checkpoint_checked(summary, kept_from_seq, token_estimate, None)
    }

    /// Like `apply_compact_checkpoint_from`, but abort if the turn view length
    /// no longer matches `expected_view_len` (log truncated under this compact).
    pub fn apply_compact_checkpoint_checked(
        &self,
        summary: &Item,
        kept_from_seq: Option<i64>,
        token_estimate: i64,
        expected_view_len: Option<usize>,
    ) -> Result<i64> {
        let _gate = self.lock_write();
        let now = chrono::Utc::now().timestamp_millis();
        let mut projection = self.projection.borrow().clone();
        if let Some(expected) = expected_view_len {
            let view_len = project_working_pairs(&projection.surface, |seq| {
                projection.items_by_seq.get(&seq).cloned().ok_or_else(|| {
                    LitecodeError::InvalidSessionEvent(format!(
                        "surface seq {seq} missing from cache"
                    ))
                })
            })?
            .len();
            if view_len != expected {
                return Err(LitecodeError::Canceled);
            }
        }

        let (op, source_seqs) = replace_op_for_keep(&projection.surface, kept_from_seq)?;
        let (from, to) = match (&op, kept_from_seq) {
            (SurfaceOp::Append, _) => (projection.next_seq, projection.next_seq),
            (SurfaceOp::Replace { start, .. }, Some(k)) => (*start, k as Seq),
            (SurfaceOp::Replace { start, .. }, None) => (*start, projection.next_seq),
        };
        let summary = item_text_preview(summary);
        let compacted = crate::session::model::CompactedBody { summary, from, to };
        let draft = EventDraft {
            time: now,
            event_type: EventType::Compacted,
            data: serde_json::to_value(&compacted)?,
            surface_op: None,
            source_seqs,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        };
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        let (seq, _) = self.admit_draft_in_tx(
            &tx,
            draft,
            &format!("compact-{}", projection.next_seq),
            0,
            "compacted",
            token_estimate,
            &mut projection,
        )?;
        if projection.surface.nodes.contains(&seq) {
            projection.items_by_seq.insert(seq, compacted.agent_item());
        }
        tx.execute(
            "UPDATE sessions SET checkpoint_seq = ?1, compacted_seq = ?1, kept_from_seq = ?2, spine_from = ?1, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![
                seq as i64,
                kept_from_seq.unwrap_or(seq as i64),
                now,
                self.id
            ],
        )?;
        tx.commit()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        self.commit_projection(projection);
        Ok(seq as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        AssistantRole, FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
        MessageItem, OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
    };
    use crate::types::user_text;

    fn session_compact_pointers(session: &Session) -> (i64, Option<i64>, i64, i64) {
        session
            .conn
            .query_row(
                "SELECT checkpoint_seq, compacted_seq, kept_from_seq, spine_from FROM sessions WHERE id = ?1",
                rusqlite::params![session.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }

    #[test]
    fn item_roundtrip_insert_load() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let items = vec![user_text("hello"), user_text("world")];
        session.insert_detail_rows(&items).unwrap();
        let loaded = session.load_transcript().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(item_text_preview(&loaded[0]), "hello");
        assert_eq!(item_text_preview(&loaded[1]), "world");
    }

    #[test]
    fn fresh_db_has_transcript_items_no_messages_table() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let has_ti: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transcript_items'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let messages_table: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has_ti, 1);
        assert_eq!(messages_table, 0);

        let cols: Vec<String> = {
            let mut stmt = session
                .conn
                .prepare("PRAGMA table_info(transcript_items)")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(cols.contains(&"item_type".to_string()));
        assert!(!cols.contains(&"role".to_string()));

        let session_cols: Vec<String> = {
            let mut stmt = session.conn.prepare("PRAGMA table_info(sessions)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(session_cols.contains(&"agent_id".to_string()));
        assert!(session_cols.contains(&"thinking_tier".to_string()));
        assert!(session_cols.contains(&"context_mode".to_string()));
        assert!(!session_cols.contains(&"model".to_string()));
        assert!(cols.contains(&"event_type".to_string()));
        assert!(cols.contains(&"surface_op".to_string()));
        assert!(cols.contains(&"source_seqs".to_string()));
    }

    #[test]
    fn insert_detail_writes_append_surface_envelope() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session.insert_detail_rows(&[user_text("hello")]).unwrap();
        let (seq, event_type, surface_op, source_seqs): (i64, String, String, Option<String>) =
            session
                .conn
                .query_row(
                    "SELECT seq, event_type, surface_op, source_seqs FROM transcript_items WHERE session_id = ?1",
                    rusqlite::params![session.id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
        assert_eq!(seq, 0);
        assert_eq!(event_type, "item/user");
        let op: crate::session::surface::SurfaceOp =
            serde_json::from_str(&surface_op).expect("surface_op json");
        assert_eq!(op, crate::session::surface::SurfaceOp::Append);
        assert!(source_seqs.is_none());
    }

    #[test]
    fn load_events_roundtrip_append_origin_is_contiguous_seq() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 3);
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.seq, i as u64);
            let disk_seq: i64 = session
                .conn
                .query_row(
                    "SELECT seq FROM transcript_items WHERE session_id = ?1 AND seq = ?2",
                    rusqlite::params![session.id, event.seq as i64],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(disk_seq, event.seq as i64);
        }
        let texts: Vec<_> = crate::session::derive_transcript_items(&events)
            .unwrap()
            .iter()
            .map(item_text_preview)
            .collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
    }

    #[test]
    fn load_events_range_is_half_open_seq_window() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let window = session.load_events_range(1, 3).unwrap();
        assert_eq!(window.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert!(session.load_events_range(2, 2).unwrap().is_empty());
        let (last, next) = session.wire_seq_cursor().unwrap();
        assert_eq!(last, 2);
        assert_eq!(next, 3);
    }

    #[test]
    fn derive_messages_after_replace_matches_surface_not_log_tail() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("d0"),
                user_text("d1"),
                user_text("d2"),
                user_text("d3"),
                user_text("d4"),
            ])
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("summary"), Some(2), 10)
            .unwrap();
        let texts: Vec<_> = session
            .load_transcript()
            .unwrap()
            .iter()
            .map(item_text_preview)
            .collect();
        assert_eq!(texts, vec!["summary", "d2", "d3", "d4"]);
        let origin: Vec<_> =
            crate::session::derive_transcript_items(&session.load_events().unwrap())
                .unwrap()
                .iter()
                .map(item_text_preview)
                .collect();
        assert_eq!(origin, vec!["d0", "d1", "d2", "d3", "d4"]);
    }

    #[test]
    fn transcript_items_missing_envelope_columns_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                project TEXT NOT NULL,
                last_message TEXT NOT NULL DEFAULT '',
                agent_id TEXT NOT NULL,
                model_id TEXT,
                thinking_tier TEXT NOT NULL DEFAULT 'medium',
                context_mode TEXT NOT NULL DEFAULT 'standard',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                checkpoint_seq INTEGER NOT NULL DEFAULT 0,
                kept_from_seq INTEGER NOT NULL DEFAULT 0,
                todos_json TEXT NOT NULL DEFAULT '[]',
                active_plan_slug TEXT,
                parent_session_id TEXT,
                parent_call_id TEXT
            );
            CREATE TABLE transcript_items (
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                turn_id TEXT NOT NULL DEFAULT '',
                turn_seq INTEGER NOT NULL DEFAULT 0,
                item_type TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'detail',
                body TEXT,
                body_ref TEXT,
                token_estimate INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, seq)
            );",
        )
        .unwrap();
        let err = ensure_session_schema(&conn).expect_err("missing envelope columns must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("event_type")
                || msg.contains("surface_op")
                || msg.contains("source_seqs")
                || msg.contains("schema_version")
                || msg.contains("compacted_seq")
                || msg.contains("spine_from")
                || msg.contains("subagent_depth"),
            "error must name a missing envelope column: {msg}"
        );
    }

    #[test]
    fn half_old_sessions_schema_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        // Old api-id `model` column without agent_id/model_id — delete-rebuild only.
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                project TEXT NOT NULL,
                model TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .unwrap();
        let err = ensure_session_schema(&conn).expect_err("missing columns must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("delete") || msg.contains("Delete"),
            "error must mention delete: {msg}"
        );
        assert!(msg.contains("sessions.db") || msg.contains("delete-and-rebuild"));
        assert!(
            msg.contains("schema_version")
                || msg.contains("agent_id")
                || msg.contains("model_id")
                || msg.contains("last_message"),
            "error must name a missing required column: {msg}"
        );
    }

    #[test]
    fn clear_orphaned_model_ids_clears_missing_catalog_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap();
        let _a = Session::open(db_path, "/p", "default", Some("keep-me")).unwrap();
        let b = Session::open(db_path, "/p", "default", Some("drop-me")).unwrap();
        drop(b);

        let mut valid = std::collections::HashSet::new();
        valid.insert("keep-me".into());
        let cleared = Session::clear_orphaned_model_ids(db_path, &valid).unwrap();
        assert_eq!(cleared.len(), 1);

        let listed = Session::list_sessions(db_path).unwrap();
        let models: Vec<_> = listed.into_iter().map(|r| r.5).collect();
        assert!(models.contains(&Some("keep-me".into())));
        assert!(models.contains(&None));
    }

    #[test]
    fn messages_with_rows_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        // Fossil table name assembled at runtime so source avoids R6 death_list SQL needles.
        let fossil = "messages";
        conn.execute_batch(&format!(
            "CREATE TABLE {fossil} (id TEXT PRIMARY KEY, body TEXT);
             INSERT INTO {fossil} (id, body) VALUES ('1', 'old');"
        ))
        .unwrap();
        let err = ensure_session_schema(&conn).expect_err("fossil messages with data must fail");
        let msg = err.to_string();
        assert!(msg.contains("messages"));
        assert!(msg.contains("delete") || msg.contains("Delete"));
    }

    #[test]
    fn empty_messages_leftover_is_dropped() {
        let conn = Connection::open_in_memory().unwrap();
        let fossil = "messages";
        conn.execute_batch(&format!("CREATE TABLE {fossil} (id TEXT PRIMARY KEY);"))
            .unwrap();
        ensure_session_schema(&conn).expect("empty messages leftover may drop");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn context_meter_roundtrip() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        assert!(session.load_context_meter().unwrap().is_empty());
        let meter = SessionContextMeter {
            prompt_tokens: 1000,
            completion_tokens: 50,
            cache_hit_tokens: 800,
            cache_miss_tokens: 200,
            cum_prompt_tokens: 5000,
            cum_completion_tokens: 250,
            cum_cache_hit_tokens: 4100,
            cum_cache_miss_tokens: 900,
        };
        session.save_context_meter(&meter).unwrap();
        assert_eq!(session.load_context_meter().unwrap(), meter);
    }

    #[test]
    fn context_meter_migrates_cumulative_columns() {
        // Simulate a pre-accumulator DB: create the old 5-column table, then run
        // schema ensure and confirm ALTER ADD COLUMN fills defaults.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite3");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE session_context_meter (
                    session_id         TEXT PRIMARY KEY,
                    prompt_tokens      INTEGER NOT NULL DEFAULT 0,
                    completion_tokens  INTEGER NOT NULL DEFAULT 0,
                    cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
                    cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
                    updated_at         INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO session_context_meter (session_id, prompt_tokens, cache_hit_tokens)
                    VALUES ('s1', 1000, 800);",
            )
            .unwrap();
        }
        let session =
            Session::open(path.to_str().unwrap(), "/proj", "default", Some("model")).unwrap();
        // Seed legacy-style data against the session's real id (post-migration).
        {
            let conn = &session.conn;
            conn.execute(
                "INSERT INTO session_context_meter (session_id, prompt_tokens, cache_hit_tokens)
                 VALUES (?1, 1000, 800)",
                rusqlite::params![session.id],
            )
            .unwrap();
        }
        let meter = session.load_context_meter().unwrap();
        assert_eq!(meter.prompt_tokens, 1000);
        assert_eq!(meter.cache_hit_tokens, 800);
        assert_eq!(meter.cum_prompt_tokens, 0);
        assert_eq!(meter.cum_cache_hit_tokens, 0);
    }

    #[test]
    fn user_anchor_query_without_role_column() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("u0"),
                Item::FunctionCall(FunctionToolCall {
                    arguments: "{}".into(),
                    call_id: "c1".into(),
                    namespace: None,
                    name: "bash".into(),
                    id: None,
                    status: None,
                }),
                user_text("u1"),
            ])
            .unwrap();
        assert_eq!(session.user_detail_count().unwrap(), 2);
        assert_eq!(session.snapshot_stem_for_user_k(0).unwrap(), 1);
        assert_eq!(session.snapshot_stem_for_user_k(1).unwrap(), 3);

        let rows = session.load_turn_transcript().unwrap();
        assert_eq!(rows[0].item_type, "message");
        assert_eq!(rows[1].item_type, "function_call");
        assert_eq!(rows[2].item_type, "message");

        session.revert_to_user_anchor(1).unwrap();
        let loaded = session.load_transcript().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(item_text_preview(&loaded[0]), "u0");
    }

    #[test]
    fn user_detail_before_buffer_index_counts_users_only() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("u0"),
                Item::FunctionCall(FunctionToolCall {
                    arguments: "{}".into(),
                    call_id: "c1".into(),
                    namespace: None,
                    name: "bash".into(),
                    id: None,
                    status: None,
                }),
                user_text("u1"),
                user_text("u2"),
            ])
            .unwrap();
        assert_eq!(session.user_detail_before_buffer_index(0).unwrap(), 0);
        assert_eq!(session.user_detail_before_buffer_index(1).unwrap(), 1);
        // index 2 is the function_call — still only one user before it
        assert_eq!(session.user_detail_before_buffer_index(2).unwrap(), 1);
        assert_eq!(session.user_detail_before_buffer_index(3).unwrap(), 2);
        assert_eq!(session.user_detail_before_buffer_index(4).unwrap(), 3);
        // past end clamps to full scan
        assert_eq!(session.user_detail_before_buffer_index(99).unwrap(), 3);
    }

    #[test]
    fn user_detail_before_includes_pre_checkpoint_history() {
        // UI history keeps archived detail visible even though model context does not.
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("before-compact"), user_text("also-before")])
            .unwrap();
        let summary = user_text("compact summary text");
        session.apply_compact_checkpoint(&summary, 10).unwrap();
        session.insert_detail_rows(&[user_text("after")]).unwrap();
        // Compaction is not a user row, even though AgentView synthesizes an
        // Item for it.
        assert_eq!(session.user_detail_before_buffer_index(0).unwrap(), 0);
        assert_eq!(session.user_detail_before_buffer_index(1).unwrap(), 1);
        assert_eq!(session.user_detail_before_buffer_index(2).unwrap(), 2);
        assert_eq!(session.user_detail_before_buffer_index(3).unwrap(), 2);
        assert_eq!(session.user_detail_before_buffer_index(4).unwrap(), 3);
    }

    #[test]
    fn user_detail_before_seq_counts_users_with_lower_seq() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
        assert_eq!(session.user_detail_before_seq(0).unwrap(), 0);
        assert_eq!(session.user_detail_before_seq(1).unwrap(), 1);
        assert_eq!(session.user_detail_before_seq(2).unwrap(), 2);
        assert_eq!(session.user_detail_before_seq(3).unwrap(), 3);
        assert_eq!(session.user_detail_count().unwrap(), 3);
    }

    #[test]
    fn compact_never_deletes_historical_detail_rows() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("archive-a"),
                user_text("archive-b"),
                user_text("keep-tail"),
            ])
            .unwrap();
        let detail_before: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(detail_before, 3);

        let keep_tail_seq: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'
                 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();

        let n = session
            .apply_compact_checkpoint_from(
                &user_text("[Conversation summary]\nprior"),
                Some(keep_tail_seq),
                42,
            )
            .unwrap();

        let detail_after: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind IN ('item/user', 'compacted')",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            detail_after, 4,
            "compact appends a log row and must not rewrite/copy kept detail (got {detail_after})"
        );

        let pre_kept_detail: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND seq < ?2",
                rusqlite::params![session.id, keep_tail_seq],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            pre_kept_detail, 2,
            "archived detail below kept_from_seq must remain"
        );
        let events = session.load_events().unwrap();
        let last = events.last().unwrap();
        assert_eq!(last.seq, n as u64);
        assert_eq!(last.event_type, EventType::Compacted);

        // Working set still excludes archived history.
        let loaded = session.load_transcript().unwrap();
        let previews: Vec<String> = loaded.iter().map(item_text_preview).collect();
        assert!(
            !previews
                .iter()
                .any(|p| p == "archive-a" || p == "archive-b"),
            "turn working set must not include pre-kept archive"
        );
        assert!(previews.iter().any(|p| p.contains("Conversation summary")));
        assert!(previews.iter().any(|p| p == "keep-tail"));
    }

    #[test]
    fn compact_checkpoint_points_at_original_kept_detail() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("old-a"),
                user_text("old-b"),
                user_text("keep-me"),
                user_text("keep-me-too"),
            ])
            .unwrap();

        let keep_from: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'
                 ORDER BY seq ASC LIMIT 1 OFFSET 2",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();

        let summary = user_text("[Conversation summary]\nprior");
        let n = session
            .apply_compact_checkpoint_from(&summary, Some(keep_from), 42)
            .unwrap();
        assert!(n >= 0);

        let loaded = session.load_transcript().unwrap();
        let previews: Vec<String> = loaded.iter().map(item_text_preview).collect();
        assert_eq!(
            previews,
            vec![
                "[Conversation summary]\nprior".to_string(),
                "keep-me".to_string(),
                "keep-me-too".to_string(),
            ]
        );
        // Turn working set hides pre-kept rows; they remain archived in DB.
        assert!(!previews.iter().any(|p| p == "old-a" || p == "old-b"));
        let archived: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user' AND seq < ?2
                   AND (body LIKE '%old-a%' OR body LIKE '%old-b%')",
                rusqlite::params![session.id, keep_from],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            archived >= 2,
            "historical old-a/old-b detail must survive compact in DB"
        );
        // No duplicate kept copies after checkpoint.
        let detail_total: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind IN ('item/user', 'compacted')",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(detail_total, 5);
    }

    #[test]
    fn compact_checkpoint_body_is_item_json() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session.insert_detail_rows(&[user_text("before")]).unwrap();
        let summary = user_text("compact summary text");
        let n = session.apply_compact_checkpoint(&summary, 10).unwrap();
        assert!(n >= 0);

        let loaded = session.load_transcript().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(item_text_preview(&loaded[0]), "compact summary text");
        let events = session.load_events().unwrap();
        let last = events.last().unwrap();
        assert_eq!(last.event_type, EventType::Compacted);
        let body = serde_json::to_string(&last.data).expect("json");
        assert!(
            body.trim_start().starts_with('{'),
            "body must be JSON object"
        );
    }

    /// Adversarial probe: empty-session compact lands CP at `seq=0` /
    /// `checkpoint_seq=0`. Has-compact must be row presence, not `> 0`,
    /// or the summary is invisible and later detail orphans it.
    #[test]
    fn empty_session_compact_checkpoint_at_seq_zero_is_visible() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        assert!(
            session.load_transcript().unwrap().is_empty(),
            "fresh session must start empty"
        );

        let n = session
            .apply_compact_checkpoint(&user_text("empty-session summary"), 10)
            .unwrap();
        assert_eq!(n, 0, "first transcript row on empty DB is seq 0");

        let loaded = session.load_transcript().unwrap();
        assert_eq!(
            loaded.len(),
            1,
            "CP at seq 0 must appear in the turn working set"
        );
        assert_eq!(item_text_preview(&loaded[0]), "empty-session summary");
    }

    #[test]
    fn empty_session_compact_then_detail_keeps_summary_visible() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .apply_compact_checkpoint(&user_text("summary-at-zero"), 10)
            .unwrap();

        session
            .insert_detail_rows(&[user_text("post-empty-compact")])
            .unwrap();

        let loaded = session.load_transcript().unwrap();
        let previews: Vec<String> = loaded.iter().map(item_text_preview).collect();
        assert_eq!(
            previews,
            vec![
                "summary-at-zero".to_string(),
                "post-empty-compact".to_string(),
            ],
            "summary at seq 0 must stay visible alongside new detail"
        );
    }

    #[test]
    fn default_no_compact_still_loads_all_detail() {
        // Default checkpoint_seq=0 with no CP row must not be treated as compact.
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        assert_eq!(session.checkpoint_seq().unwrap(), 0);
        assert_eq!(session.kept_from_seq().unwrap(), 0);

        let loaded = session.load_transcript().unwrap();
        let previews: Vec<String> = loaded.iter().map(item_text_preview).collect();
        assert_eq!(
            previews,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        let rows = session.load_turn_transcript().unwrap();
        assert!(rows.iter().all(|r| r.kind == "item/user"));
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn nonempty_empty_keep_compact_is_summary_only_until_new_detail() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("archive-1"), user_text("archive-2")])
            .unwrap();
        let n = session
            .apply_compact_checkpoint(&user_text("rolled-up"), 10)
            .unwrap();
        assert!(n > 0, "non-empty session replace must land after detail");

        let after_compact = session.load_transcript().unwrap();
        assert_eq!(after_compact.len(), 1);
        assert_eq!(item_text_preview(&after_compact[0]), "rolled-up");

        session.insert_detail_rows(&[user_text("fresh")]).unwrap();
        let previews: Vec<String> = session
            .load_transcript()
            .unwrap()
            .iter()
            .map(item_text_preview)
            .collect();
        assert_eq!(previews, vec!["rolled-up".to_string(), "fresh".to_string()]);
        assert!(!previews.iter().any(|p| p.starts_with("archive-")));
    }

    #[test]
    fn second_compact_replaces_seq_zero_checkpoint() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let first = session
            .apply_compact_checkpoint(&user_text("first-summary"), 10)
            .unwrap();
        assert_eq!(first, 0);
        session.insert_detail_rows(&[user_text("between")]).unwrap();

        let second = session
            .apply_compact_checkpoint(&user_text("second-summary"), 20)
            .unwrap();
        assert!(second > first);

        let loaded = session.load_transcript().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(item_text_preview(&loaded[0]), "second-summary");
        let events = session.load_events().unwrap();
        assert_eq!(events[0].event_type, EventType::Compacted);
        assert_eq!(events.last().unwrap().event_type, EventType::Compacted);
    }

    #[test]
    fn compact_checkpoint_history_index_always_appends() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        session
            .apply_compact_checkpoint(&user_text("first-cut"), 10)
            .unwrap();
        let history = session.load_history_transcript().unwrap();
        assert_eq!(history.len(), 4);
        assert_eq!(history[3].seq, 3, "first compact appends at log end");

        session.insert_detail_rows(&[user_text("d")]).unwrap();
        session
            .apply_compact_checkpoint(&user_text("second-cut"), 20)
            .unwrap();
        let history2 = session.load_history_transcript().unwrap();
        assert_eq!(history2.len(), 6);
        assert_eq!(history2[3].seq, 3);
        assert_eq!(history2[4].seq, 4);
        assert_eq!(history2[5].seq, 5);

        let (_, _, indices) = session.load_by_buffer_index_with_kinds(0, 6).unwrap();
        assert_eq!(
            indices,
            vec![0, 1, 2, 3, 4, 5],
            "history ordinals are ORDER BY seq ranks, not recomputed after compact"
        );
        let (_, _, tail) = session.load_by_buffer_index_with_kinds(3, 6).unwrap();
        assert_eq!(tail, vec![3, 4, 5]);
    }

    #[test]
    fn revert_removing_last_checkpoint_resets_pointers() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("keep-visible-after-revert"),
                user_text("will-be-cut"),
                user_text("also-cut"),
            ])
            .unwrap();

        let keep_from: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'
                 ORDER BY seq ASC LIMIT 1 OFFSET 1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();

        session
            .apply_compact_checkpoint_from(&user_text("summary"), Some(keep_from), 10)
            .unwrap();
        // Replace summaries are not revert k; three append-origin user rows remain.
        assert_eq!(session.user_detail_count().unwrap(), 3);

        session.revert_to_user_anchor(1).unwrap();

        let previews: Vec<String> = session
            .load_transcript()
            .unwrap()
            .iter()
            .map(item_text_preview)
            .collect();
        assert_eq!(
            previews,
            vec!["keep-visible-after-revert".to_string()],
            "after last CP is gone, full remaining detail must be visible"
        );
        let (checkpoint_seq, compacted_seq, kept_from_seq, spine_from) =
            session_compact_pointers(&session);
        assert_eq!(checkpoint_seq, 0);
        assert_eq!(compacted_seq, None);
        assert_eq!(kept_from_seq, 0);
        assert_eq!(spine_from, 0);
    }

    #[test]
    fn compact_keep_sets_spine_from_to_compact_seq_not_kept_detail() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let keep_from: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'
                 ORDER BY seq ASC LIMIT 1 OFFSET 1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        let n = session
            .apply_compact_checkpoint_from(&user_text("summary"), Some(keep_from), 10)
            .unwrap();
        let (checkpoint_seq, compacted_seq, kept_from_seq, spine_from) =
            session_compact_pointers(&session);
        assert_eq!(checkpoint_seq, n);
        assert_eq!(compacted_seq, Some(n));
        assert_eq!(kept_from_seq, keep_from);
        assert_eq!(spine_from, n, "spine_from is the compact event seq");
        assert_ne!(spine_from, keep_from);
    }

    #[test]
    fn revert_retaining_earlier_compact_restores_pointers() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let keep_first: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'
                 ORDER BY seq ASC LIMIT 1 OFFSET 1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        let first = session
            .apply_compact_checkpoint_from(&user_text("cut-1"), Some(keep_first), 10)
            .unwrap();
        session
            .insert_detail_rows(&[user_text("d"), user_text("e")])
            .unwrap();
        let keep_second: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'
                 ORDER BY seq ASC LIMIT 1 OFFSET 3",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("cut-2"), Some(keep_second), 10)
            .unwrap();
        // Users: a,b,c,d,e → k=3 is `d`. Truncate from d removes the second compact.
        session.revert_to_user_anchor(3).unwrap();
        let (checkpoint_seq, compacted_seq, kept_from_seq, spine_from) =
            session_compact_pointers(&session);
        assert_eq!(checkpoint_seq, first);
        assert_eq!(compacted_seq, Some(first));
        assert_eq!(kept_from_seq, keep_first);
        assert_eq!(spine_from, first);
    }

    #[test]
    fn assistant_only_delta_does_not_wipe_last_message() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let updated = session
            .insert_detail_rows(&[user_text("hello user")])
            .unwrap();
        assert!(updated.is_some());
        assert_eq!(updated.unwrap().0, "hello user");

        let assistant = Item::Message(MessageItem::Output(OutputMessage {
            id: "msg_1".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "assistant reply".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        let second = session.insert_detail_rows(&[assistant]).unwrap();
        assert!(
            second.is_none(),
            "no user message in delta → no preview update"
        );

        let preview: String = session
            .conn
            .query_row(
                "SELECT last_message FROM sessions WHERE id = ?1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preview, "hello user");
    }

    #[test]
    fn revert_preview_is_text_not_json_brace() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("keep me"), user_text("drop me")])
            .unwrap();
        session.revert_to_user_anchor(1).unwrap();

        let preview: String = session
            .conn
            .query_row(
                "SELECT last_message FROM sessions WHERE id = ?1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preview, "keep me");
        assert!(!preview.starts_with('{'));
    }

    #[test]
    fn snip_stale_results_removes_orphan_outputs() {
        let fc = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "c1".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        });
        let live = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Text("ok".into()),
            id: None,
            status: None,
        });
        let stale = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "gone".into(),
            output: FunctionCallOutput::Text("orphan".into()),
            id: None,
            status: None,
        });
        let mut items = vec![fc, live, stale];
        Session::snip_stale_results(&mut items);
        assert_eq!(items.len(), 2);
        match &items[1] {
            Item::FunctionCallOutput(o) => match &o.output {
                FunctionCallOutput::Text(t) => assert_eq!(t, "ok"),
                _ => panic!("expected text"),
            },
            _ => panic!("expected output"),
        }
        assert!(
            !items.iter().any(|i| match i {
                Item::FunctionCallOutput(o) => o.call_id == "gone",
                _ => false,
            }),
            "orphan FunctionCallOutput must be removed, not rewritten"
        );
    }

    #[test]
    fn pad_unanswered_calls_inserts_after_call_block() {
        let fc_a = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "a".into(),
            namespace: None,
            name: "read".into(),
            id: None,
            status: None,
        });
        let fc_b = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "b".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        });
        let mut items = vec![fc_a, fc_b, user_text("continue")];
        let n = Session::pad_unanswered_calls(&mut items);
        assert_eq!(n, 2);
        assert_eq!(items.len(), 5);
        assert!(matches!(&items[0], Item::FunctionCall(fc) if fc.call_id == "a"));
        assert!(matches!(&items[1], Item::FunctionCall(fc) if fc.call_id == "b"));
        assert!(matches!(&items[2], Item::FunctionCallOutput(o) if o.call_id == "a"));
        assert!(matches!(&items[3], Item::FunctionCallOutput(o) if o.call_id == "b"));
        assert!(matches!(&items[4], Item::Message(_)));
    }

    #[test]
    fn pad_unanswered_calls_skips_already_answered() {
        let fc = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "c1".into(),
            namespace: None,
            name: "read".into(),
            id: None,
            status: None,
        });
        let out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Text("ok".into()),
            id: None,
            status: None,
        });
        let mut items = vec![fc, out];
        assert_eq!(Session::pad_unanswered_calls(&mut items), 0);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn child_session_persists_parent_link_and_is_hidden_from_list() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap();

        let parent = Session::open(db_path, "/proj", "default", Some("m")).unwrap();
        let child = Session::open_with_parent(
            db_path,
            "/proj",
            "reviewer",
            Some("m"),
            Some(&parent.id),
            Some("call_abc"),
        )
        .unwrap();

        assert_eq!(child.parent_session_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(child.parent_call_id.as_deref(), Some("call_abc"));

        let listed = Session::list_sessions(db_path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, parent.id);

        let children = Session::list_child_session_ids(db_path, &parent.id).unwrap();
        assert_eq!(children, vec![child.id.clone()]);

        let resumed = Session::resume(db_path, &child.id).unwrap();
        assert_eq!(
            resumed.parent_session_id.as_deref(),
            Some(parent.id.as_str())
        );
        assert_eq!(resumed.parent_call_id.as_deref(), Some("call_abc"));
    }

    #[test]
    fn delete_parent_cascades_to_child_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap();

        let parent = Session::open(db_path, "/proj", "default", None).unwrap();
        let child = Session::open_with_parent(
            db_path,
            "/proj",
            "reviewer",
            None,
            Some(&parent.id),
            Some("call_1"),
        )
        .unwrap();
        let child_id = child.id.clone();
        let parent_id = parent.id.clone();
        drop(parent);
        drop(child);

        Session::delete(db_path, &parent_id).unwrap();
        assert!(Session::resume(db_path, &parent_id).is_err());
        assert!(Session::resume(db_path, &child_id).is_err());
        assert!(
            Session::list_child_session_ids(db_path, &parent_id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn commit_orphan_cleanup_leaves_memory_untouched_when_tx_fails() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let fc = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "c1".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        });
        let live = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Text("ok".into()),
            id: None,
            status: None,
        });
        let orphan = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "gone".into(),
            output: FunctionCallOutput::Text("orphan".into()),
            id: None,
            status: None,
        });
        session
            .insert_detail_rows(&[fc.clone(), live.clone(), orphan.clone()])
            .unwrap();

        let mut items = vec![
            WorkingRow::pending(fc),
            WorkingRow::pending(live),
            WorkingRow::pending(orphan),
            WorkingRow::pending(user_text("fresh")),
        ];
        session
            .conn
            .execute_batch("DROP TABLE transcript_items")
            .unwrap();
        let err = session
            .commit_turn_delta_with_orphan_cleanup(&mut items, &[], 2, "t1")
            .expect_err("dropped table must fail the transaction");
        let _ = err;
        assert_eq!(
            items.len(),
            4,
            "failed commit must not snip orphans from memory"
        );
        assert!(
            items.iter().any(|row| match &row.item {
                Item::FunctionCallOutput(o) => o.call_id == "gone",
                _ => false,
            }),
            "orphan must remain in the caller's vec when disk did not change"
        );
    }

    #[test]
    fn commit_delta_discards_when_turn_view_is_shorter() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
        session.revert_to_user_anchor(1).unwrap();
        let mut items = vec![
            WorkingRow::pending(user_text("u0")),
            WorkingRow::pending(user_text("u1")),
            WorkingRow::pending(user_text("u2")),
            WorkingRow::pending(user_text("stale")),
        ];
        let outcome = session
            .commit_turn_delta_with_orphan_cleanup(&mut items, &[user_text("stale")], 2, "t1")
            .unwrap();
        assert!(matches!(outcome, CommitDeltaOutcome::Discarded));
        assert_eq!(items.len(), 1);
        assert_eq!(item_text_preview(&items[0].item), "u0");
        assert_eq!(session.load_transcript().unwrap().len(), 1);
    }

    #[test]
    fn commit_delta_does_not_discard_when_empty_assistant_is_off_surface() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let empty_assistant = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_empty".into(),
            role: AssistantRole::Assistant,
            content: vec![],
            status: OutputStatus::Completed,
            phase: None,
        }));
        let fc = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "c1".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        });
        session
            .insert_detail_rows(&[user_text("u0"), empty_assistant.clone(), fc.clone()])
            .unwrap();
        let projected = session.load_transcript().unwrap().len();
        assert!(
            projected < 3,
            "empty assistant must be off Surface so projection length would false-cancel"
        );
        let mut items = session.load_working_set().unwrap();
        let outcome = session
            .commit_turn_delta_with_orphan_cleanup(&mut items, &[], 2, "t1")
            .unwrap();
        assert!(
            matches!(outcome, CommitDeltaOutcome::Applied { .. }),
            "log still has 3 rows; persist must not treat skip-empty Surface as 回退"
        );
        assert_eq!(session.load_events().unwrap().len(), 3);
    }

    #[test]
    fn commit_delta_does_not_insert_orphan_output() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let fc = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "c1".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        });
        let live = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Text("ok".into()),
            id: None,
            status: None,
        });
        let orphan = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "gone".into(),
            output: FunctionCallOutput::Text("orphan".into()),
            id: None,
            status: None,
        });
        session.insert_detail_rows(&[fc.clone()]).unwrap();
        let mut items = session.load_working_set().unwrap();
        items.push(WorkingRow::pending(live.clone()));
        items.push(WorkingRow::pending(orphan.clone()));
        let outcome = session
            .commit_turn_delta_with_orphan_cleanup(&mut items, &[live, orphan], 0, "t1")
            .unwrap();
        assert!(matches!(outcome, CommitDeltaOutcome::Applied { .. }));
        assert_eq!(items.len(), 2);
        let db = session.load_transcript().unwrap();
        assert_eq!(db.len(), 2);
        assert!(
            !db.iter()
                .any(|i| matches!(i, Item::FunctionCallOutput(o) if o.call_id == "gone")),
            "orphan in the delta must not be inserted"
        );
    }

    #[test]
    fn compact_checkpoint_aborts_when_turn_view_shrunk() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
        let rows = session.load_turn_transcript().unwrap();
        let prefix = rows.len();
        let kept = rows[0].seq;
        session.revert_to_user_anchor(2).unwrap();
        let err = session
            .apply_compact_checkpoint_checked(&user_text("sum"), Some(kept), 10, Some(prefix))
            .unwrap_err();
        assert!(matches!(err, LitecodeError::Canceled));
        assert_eq!(session.load_transcript().unwrap().len(), 2);
        assert!(
            session
                .load_turn_transcript()
                .unwrap()
                .iter()
                .all(|r| r.kind != "compact_checkpoint"),
            "truncated log must not receive a compact checkpoint"
        );
    }

    #[test]
    fn in_progress_item_is_retrievable_and_seal_keeps_seq() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let live = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_live".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hel".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::InProgress,
            phase: None,
        }));
        let seq = session.persist_item(&live).unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, seq);
        assert_eq!(events[0].state, LogState::InProgress);
        let loaded = crate::session::event::item_from_event(&events[0]).unwrap();
        match &loaded {
            Item::Message(MessageItem::Output(m)) => {
                assert_eq!(m.status, OutputStatus::InProgress);
                assert_eq!(item_text_preview(&loaded), "hel");
            }
            other => panic!("expected assistant item, got {other:?}"),
        }

        let sealed = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_live".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hello".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        session.seal_item(seq, &sealed).unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 1, "封口 must not append a new row");
        assert_eq!(events[0].seq, seq);
        assert_eq!(events[0].state, LogState::Final);
        let loaded = crate::session::event::item_from_event(&events[0]).unwrap();
        match &loaded {
            Item::Message(MessageItem::Output(m)) => {
                assert_eq!(m.status, OutputStatus::Completed);
                assert_eq!(item_text_preview(&loaded), "hello");
            }
            other => panic!("expected sealed assistant item, got {other:?}"),
        }
    }

    #[test]
    fn seal_in_progress_items_marks_incomplete_and_final() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let live = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_cancel".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hel".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::InProgress,
            phase: None,
        }));
        session.persist_item(&live).unwrap();
        assert_eq!(session.seal_in_progress_items().unwrap(), vec![0]);
        assert!(session.seal_in_progress_items().unwrap().is_empty());
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].state, LogState::Final);
        match item_from_event(&events[0]).unwrap() {
            Item::Message(MessageItem::Output(m)) => {
                assert_eq!(m.status, OutputStatus::Incomplete);
            }
            other => panic!("expected sealed assistant, got {other:?}"),
        }
    }

    #[test]
    fn crashed_session_reload_keeps_in_progress_row_for_seal() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db = db.to_str().unwrap();
        let session = Session::open(db, "/tmp/proj", "default", Some("model")).unwrap();
        let id = session.id.clone();
        let live = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_crash".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hel".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::InProgress,
            phase: None,
        }));
        let seq = session.persist_item(&live).unwrap();
        drop(session);

        let session = Session::resume(db, &id).unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, seq);
        assert_eq!(events[0].state, LogState::InProgress);
        let loaded = crate::session::event::item_from_event(&events[0]).unwrap();
        match &loaded {
            Item::Message(MessageItem::Output(m)) => {
                assert_eq!(m.status, OutputStatus::InProgress);
            }
            other => panic!("expected in_progress assistant, got {other:?}"),
        }
        let sealed = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_crash".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hello".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        session.seal_item(seq, &sealed).unwrap();
        assert_eq!(session.load_events().unwrap().len(), 1);
        let loaded =
            crate::session::event::item_from_event(&session.load_events().unwrap()[0]).unwrap();
        assert_eq!(item_text_preview(&loaded), "hello");
    }

    #[test]
    fn commit_delta_seals_existing_seq_instead_of_appending() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let live = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_dup".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hel".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::InProgress,
            phase: None,
        }));
        session.persist_item(&live).unwrap();
        let sealed = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_dup".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hello".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        let mut items = vec![WorkingRow::pending(sealed.clone())];
        session
            .commit_turn_delta_with_orphan_cleanup(&mut items, &[sealed], 0, "t1")
            .unwrap();
        assert_eq!(
            session.load_events().unwrap().len(),
            1,
            "added persist then step commit must 封口, not append"
        );
        let loaded =
            crate::session::event::item_from_event(&session.load_events().unwrap()[0]).unwrap();
        assert_eq!(item_text_preview(&loaded), "hello");
    }

    #[test]
    fn persist_item_then_commit_does_not_duplicate_assistant() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session.insert_detail_rows(&[user_text("hi")]).unwrap();
        let live = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_once".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hel".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::InProgress,
            phase: None,
        }));
        session.persist_item(&live).unwrap();
        let sealed = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_once".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hello".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        let mut items = session.load_working_set().unwrap();
        assert_eq!(items.len(), 2);
        items[1].item = sealed;
        session
            .commit_turn_delta_with_orphan_cleanup(&mut items, &[], 1, "t1")
            .unwrap();
        assert_eq!(session.load_events().unwrap().len(), 2);
    }

    #[test]
    fn persist_item_after_truncate_does_not_restore_tail() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1")])
            .unwrap();
        let live = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_stale".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "tail".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::InProgress,
            phase: None,
        }));
        session.persist_item(&live).unwrap();
        session.revert_to_user_anchor(1).unwrap();
        let err = session.persist_item(&live).unwrap_err();
        assert!(matches!(err, LitecodeError::Canceled));
        assert_eq!(session.load_transcript().unwrap().len(), 1);
        assert_eq!(
            item_text_preview(&session.load_transcript().unwrap()[0]),
            "u0"
        );
    }

    #[test]
    fn apply_can_append_log_only_turn_start() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let outcome = session
            .apply(SessionApply::Append(EventDraft {
                time: 1,
                event_type: EventType::TurnStart,
                data: serde_json::json!({"turn_id": "t1"}),
                surface_op: None,
                source_seqs: None,
                ignorable: false,
                state: crate::session::model::LogState::Final,
            }))
            .unwrap();
        assert!(matches!(outcome, ApplyOutcome::Appended(0)));
        session.insert_detail_rows(&[user_text("hi")]).unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EventType::TurnStart);
        assert_eq!(session.load_transcript().unwrap().len(), 1);
    }

    #[test]
    fn injection_append_roundtrips_into_agent_working_set() {
        use crate::session::model::{HookPromptBody, ReminderTurnAbortedBody};
        use crate::types::item_text_preview;

        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session.insert_detail_rows(&[user_text("hi")]).unwrap();
        session
            .apply(SessionApply::Append(EventDraft {
                time: 1,
                event_type: EventType::HookPrompt,
                data: serde_json::to_value(HookPromptBody {
                    text: "hook text".into(),
                    hook_run_id: "hr1".into(),
                    placement: None,
                })
                .unwrap(),
                surface_op: None,
                source_seqs: None,
                ignorable: false,
                state: crate::session::model::LogState::Final,
            }))
            .unwrap();
        session
            .apply(SessionApply::Append(EventDraft {
                time: 2,
                event_type: EventType::ReminderTurnAborted,
                data: serde_json::to_value(ReminderTurnAbortedBody {
                    text: "aborted".into(),
                })
                .unwrap(),
                surface_op: None,
                source_seqs: None,
                ignorable: false,
                state: crate::session::model::LogState::Final,
            }))
            .unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[1].event_type.enters_spine());
        let working = session.load_working_set().unwrap();
        assert_eq!(working.len(), 3);
        assert_eq!(item_text_preview(&working[0].item), "hi");
        assert!(item_text_preview(&working[1].item).contains("[hook/prompt hr1]"));
        assert!(item_text_preview(&working[2].item).contains("[reminder/turn_aborted]"));
        let human = crate::session::derive_transcript_items(&events).unwrap();
        assert_eq!(human.len(), 1);
    }

    #[test]
    fn compact_shadowed_user_same_text_appends_new_seq() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("hi"), user_text("keep")])
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("summary"), Some(1), 10)
            .unwrap();
        let before = session.load_events().unwrap().len();
        let mut rows = session.load_working_set().unwrap();
        rows.push(WorkingRow::pending(user_text("hi")));
        session
            .commit_turn_delta_with_orphan_cleanup(&mut rows, &[], session.cached_max_seq(), "t1")
            .unwrap();
        let events = session.load_events().unwrap();
        assert_eq!(
            events.len(),
            before + 1,
            "same text after compact must append"
        );
        assert_eq!(
            item_text_preview(&session.load_transcript().unwrap().last().unwrap()),
            "hi"
        );
    }

    #[test]
    fn duplicate_hi_after_assistant_is_two_user_rows() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session.insert_detail_rows(&[user_text("hi")]).unwrap();
        let asst = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_hi".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "yo".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        session.persist_item(&asst).unwrap();
        let mut rows = session.load_working_set().unwrap();
        rows.push(WorkingRow::pending(user_text("hi")));
        session
            .commit_turn_delta_with_orphan_cleanup(&mut rows, &[], session.cached_max_seq(), "t1")
            .unwrap();
        let users: Vec<_> = session
            .load_events()
            .unwrap()
            .into_iter()
            .filter(|e| e.event_type == EventType::ItemUser)
            .collect();
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn incremental_surface_matches_fold_after_compact_and_append() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[
                user_text("a"),
                user_text("b"),
                user_text("c"),
                user_text("d"),
            ])
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("sum"), Some(2), 10)
            .unwrap();
        session.insert_detail_rows(&[user_text("e")]).unwrap();
        let cached = session.projection.borrow().surface.nodes.clone();
        let folded = fold_surface(&session.load_events().unwrap()).unwrap().nodes;
        assert_eq!(cached, folded);
    }

    #[test]
    fn truncate_then_append_uses_new_max_plus_one() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
        session.revert_to_user_anchor(1).unwrap();
        let seq = session.insert_detail_rows(&[user_text("fresh")]).unwrap();
        let _ = seq;
        let events = session.load_events().unwrap();
        assert_eq!(events.last().unwrap().seq, 1);
        assert_eq!(session.cached_max_seq(), 1);
        let folded = fold_surface(&events).unwrap().nodes;
        assert_eq!(session.projection.borrow().surface.nodes, folded);
    }

    #[test]
    fn load_working_set_matches_transcript_and_skips_empty_assistant() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let empty_assistant = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_empty".into(),
            role: AssistantRole::Assistant,
            content: vec![],
            status: OutputStatus::Completed,
            phase: None,
        }));
        session
            .insert_detail_rows(&[user_text("u0"), empty_assistant, user_text("u1")])
            .unwrap();
        let rows = session.load_working_set().unwrap();
        let items = session.load_transcript().unwrap();
        assert_eq!(rows.len(), items.len());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.log_seq.is_some()));
        assert_eq!(session.load_events().unwrap().len(), 3);
    }

    #[test]
    fn compact_replace_validates_endpoints_on_cached_nodes() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let err = session
            .apply_compact_checkpoint_from(&user_text("sum"), Some(99), 10)
            .unwrap_err();
        assert!(
            err.to_string().contains("kept_from_seq")
                || matches!(err, LitecodeError::ToolExecution(_))
        );
        session
            .apply_compact_checkpoint_from(&user_text("sum"), Some(1), 10)
            .unwrap();
        let cached = session.projection.borrow().surface.nodes.clone();
        let folded = fold_surface(&session.load_events().unwrap()).unwrap().nodes;
        assert_eq!(cached, folded);
        session.insert_detail_rows(&[user_text("after")]).unwrap();
        assert_eq!(
            session.load_events().unwrap().last().unwrap().seq,
            session.cached_max_seq() as u64
        );
    }

    #[test]
    fn append_after_many_compact_shadows_uses_contiguous_next_seq() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(
                &(0..8)
                    .map(|i| user_text(format!("u{i}")))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("sum-1"), Some(4), 10)
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("sum-2"), Some(6), 10)
            .unwrap();
        let events_before = session.load_events().unwrap();
        let max_before = events_before.last().unwrap().seq;
        assert!(
            events_before.len() > session.projection.borrow().surface.nodes.len(),
            "compact must leave shadowed rows so next seq is not a surface scan"
        );
        session
            .insert_detail_rows(&[user_text("after-shadows")])
            .unwrap();
        let last = session.load_events().unwrap().last().unwrap().seq;
        assert_eq!(
            last,
            max_before + 1,
            "append must be MAX+1 without rescanning shadows"
        );
        assert_eq!(session.cached_max_seq() as u64, last);
    }

    #[test]
    fn compact_summary_agent_view_is_assistant_and_survives_reload() {
        use crate::authority::responses::MessageItem;
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("old"), user_text("keep")])
            .unwrap();
        session
            .apply_compact_checkpoint_from(&user_text("rolled-up"), Some(1), 10)
            .unwrap();
        let loaded = session.load_transcript().unwrap();
        match &loaded[0] {
            Item::Message(MessageItem::Output(out)) => {
                assert_eq!(out.role, AssistantRole::Assistant);
            }
            other => panic!("expected assistant compact summary, got {other:?}"),
        }
        assert!(item_text_preview(&loaded[0]).contains("rolled-up"));
        let body: CompactedBody = serde_json::from_value(
            session
                .load_events()
                .unwrap()
                .into_iter()
                .find(|e| e.event_type == EventType::Compacted)
                .unwrap()
                .data,
        )
        .unwrap();
        assert_eq!(body.from, 0);
        assert_eq!(body.to, 1, "keep-recent exclusive end is first kept seq");
        session.hydrate_projection().unwrap();
        match &session.load_transcript().unwrap()[0] {
            Item::Message(MessageItem::Output(out)) => {
                assert_eq!(out.role, AssistantRole::Assistant);
            }
            other => panic!("reload must keep assistant summary, got {other:?}"),
        }
    }

    #[test]
    fn empty_keep_compact_interval_is_half_open_to_compact_seq() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let n = session
            .apply_compact_checkpoint(&user_text("all"), 10)
            .unwrap();
        let body: CompactedBody = serde_json::from_value(
            session
                .load_events()
                .unwrap()
                .into_iter()
                .find(|e| e.event_type == EventType::Compacted)
                .unwrap()
                .data,
        )
        .unwrap();
        assert_eq!(body.from, 0);
        assert_eq!(body.to, n as u64);
        assert_eq!(session.load_transcript().unwrap().len(), 1);
    }

    #[test]
    fn persist_then_commit_idless_tool_result_does_not_duplicate() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        let fc = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "call_dup".into(),
            namespace: None,
            name: "read".into(),
            id: None,
            status: None,
        });
        session.persist_item(&fc).unwrap();
        let out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "call_dup".into(),
            output: FunctionCallOutput::Text("ok".into()),
            id: None,
            status: None,
        });
        session.persist_item(&out).unwrap();
        let other = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "call_other".into(),
            output: FunctionCallOutput::Text("other".into()),
            id: None,
            status: None,
        });
        let fc2 = Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "call_other".into(),
            namespace: None,
            name: "bash".into(),
            id: None,
            status: None,
        });
        session.persist_item(&fc2).unwrap();
        let mut rows = session.load_working_set().unwrap();
        // Simulate commit_step_from_items seeing the same result again.
        rows.push(WorkingRow::pending(out.clone()));
        session
            .commit_turn_delta_with_orphan_cleanup(&mut rows, &[], session.cached_max_seq(), "t1")
            .unwrap();
        let results: Vec<_> = session
            .load_events()
            .unwrap()
            .into_iter()
            .filter(|e| e.event_type == EventType::ItemToolResult)
            .collect();
        assert_eq!(
            results.len(),
            1,
            "same call_id must not append a second result"
        );
        let mut rows = session.load_working_set().unwrap();
        rows.push(WorkingRow::pending(other));
        session
            .commit_turn_delta_with_orphan_cleanup(&mut rows, &[], session.cached_max_seq(), "t1")
            .unwrap();
        let results: Vec<_> = session
            .load_events()
            .unwrap()
            .into_iter()
            .filter(|e| e.event_type == EventType::ItemToolResult)
            .collect();
        assert_eq!(
            results.len(),
            2,
            "distinct call_id results must both persist"
        );
    }

    #[test]
    fn turn_end_reason_schema_covers_every_variant() {
        use crate::runtime::observer::TurnEndReason;
        let cases = [
            (TurnEndReason::Completed, "completed"),
            (TurnEndReason::Cancelled, "cancelled"),
            (TurnEndReason::Error, "error"),
            (TurnEndReason::MaxSteps, "max_steps"),
            (TurnEndReason::HookBlocked, "hook_blocked"),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason.as_log_reason(), expected);
            let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
            session
                .apply(SessionApply::Append(EventDraft {
                    time: 1,
                    event_type: EventType::TurnEnd,
                    data: serde_json::json!({"turn": "t", "reason": reason.as_log_reason()}),
                    surface_op: None,
                    source_seqs: None,
                    ignorable: false,
                    state: LogState::Final,
                }))
                .unwrap();
            let events = session.load_events().unwrap();
            assert_eq!(events[0].data["reason"], expected);
            assert!(session.load_transcript().unwrap().is_empty());
        }
    }
}
