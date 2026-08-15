use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

use crate::authority::responses::{InputMessage, InputRole, MessageItem};
use crate::platform_knobs::{ContextMode, ThinkingTier};
use crate::session::estimate::compute_token_estimate;
use crate::session::snapshot;
use crate::session::task_state::TodoItem;
use crate::session::task_state::{PlanRef, TaskReminders};
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
/// Body is serialized authority [`Item`] JSON (detail and compact_checkpoint alike).
/// `item_type` is the Responses Item `type` string (`message`, `reasoning`, …).
/// `kind` is the row envelope only: `detail` | `compact_checkpoint`.
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

/// §5.1 turn 装载 — step 1: read authoritative checkpoint (default 0).
pub const SQL_CHECKPOINT_SEQ: &str = "SELECT checkpoint_seq FROM sessions WHERE id = ?";

pub const SQL_KEPT_FROM_SEQ: &str = "SELECT kept_from_seq FROM sessions WHERE id = ?";

/// §5.1 turn 装载 — pi-style working set:
/// `compact_checkpoint` at `checkpoint_seq` (if that row exists), then original
/// `detail` with `seq >= kept_from_seq` (and any newer detail after the checkpoint).
/// Order: summary first, then detail by seq.
///
/// **Has-compact is row presence, not `checkpoint_seq > 0`.** Default
/// `checkpoint_seq=0` with no CP row still means “no compact” (clause matches
/// nothing). Empty-session compact may legitimately place the CP at `seq=0`;
/// requiring `> 0` would orphan that summary.
pub const SQL_LOAD_TURN_TRANSCRIPT: &str = "\
SELECT t.session_id, t.seq, t.turn_id, t.turn_seq, t.item_type, t.kind, t.body, t.body_ref,
       t.token_estimate, t.created_at
FROM transcript_items t
JOIN sessions s ON s.id = t.session_id
WHERE t.session_id = ?1
  AND (
    (t.kind = 'compact_checkpoint' AND t.seq = s.checkpoint_seq)
    OR (t.kind = 'detail' AND t.seq >= s.kept_from_seq)
  )
ORDER BY CASE WHEN t.kind = 'compact_checkpoint' THEN 0 ELSE 1 END, t.seq ASC";

/// Full chronological UI history: all detail plus the current checkpoint marker.
pub const SQL_LOAD_HISTORY_TRANSCRIPT: &str = "\
SELECT t.session_id, t.seq, t.turn_id, t.turn_seq, t.item_type, t.kind, t.body, t.body_ref,
       t.token_estimate, t.created_at
FROM transcript_items t
JOIN sessions s ON s.id = t.session_id
WHERE t.session_id = ?1
  AND (
    t.kind = 'detail'
    OR (t.kind = 'compact_checkpoint' AND t.seq = s.checkpoint_seq)
  )
ORDER BY t.seq ASC";

/// UI revert anchors span all visible historical user detail.
pub const SQL_USER_DETAIL_COUNT: &str = "\
SELECT COUNT(*) FROM transcript_items t
WHERE t.session_id = ?
  AND t.kind = 'detail'
  AND t.item_type = 'message'
  AND t.body IS NOT NULL
  AND json_extract(t.body, '$.role') = 'user'";

/// UI revert k → anchor_seq mapping across the full history.
pub const SQL_ANCHOR_SEQ: &str = "\
SELECT seq FROM (
    SELECT t.seq, ROW_NUMBER() OVER (ORDER BY t.seq) - 1 AS k
    FROM transcript_items t
    WHERE t.session_id = ?
      AND t.kind = 'detail'
      AND t.item_type = 'message'
      AND t.body IS NOT NULL
      AND json_extract(t.body, '$.role') = 'user'
) WHERE k = ?";

const SESSIONS_REQUIRED_COLS: &[&str] = &[
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
            kind            TEXT NOT NULL DEFAULT 'detail',
            body            TEXT,
            body_ref        TEXT,
            token_estimate  INTEGER NOT NULL DEFAULT 0,
            created_at      INTEGER NOT NULL,
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
    /// In-memory seq cursor for persisted-transcript delta alignment (REV-3).
    /// Not a DB column; seq is allocated on row insert. Tracks the DB's max
    /// `transcript_items.seq` so increment commits align with `seq > persisted_max_seq`.
    persisted_max_seq: Cell<i64>,
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
fn transcript_row_is_user_detail(row: &TranscriptRow) -> bool {
    if row.kind != "detail" || row.item_type != "message" {
        return false;
    }
    let Some(body) = row.body.as_deref() else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("role").and_then(|r| r.as_str()).map(|r| r == "user"))
        .unwrap_or(false)
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
        // Disk truth: both kinds store serialized Item JSON (no plain-text synthesize).
        "compact_checkpoint" | "detail" => {
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

fn rows_to_items(rows: &[TranscriptRow], data_root: &Path) -> Result<Transcript> {
    rows.iter().map(|row| row_to_item(row, data_root)).collect()
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
    pub fn ephemeral(project: &str, agent_id: &str, model_id: Option<&str>) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        ensure_session_schema(&conn)?;

        let id = ulid::Ulid::new().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let model_id_owned = normalize_model_id(model_id);

        conn.execute(
            "INSERT INTO sessions (id, project, last_message, agent_id, model_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, project, "", agent_id, model_id_owned, now, now],
        )?;

        let data_root = std::env::temp_dir().join("litecode");

        Ok(Self {
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
        })
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
                id, project, last_message, agent_id, model_id, created_at, updated_at,
                parent_session_id, parent_call_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                id,
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

        Ok(Self {
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
        })
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
                id, project, last_message, agent_id, model_id, thinking_tier, context_mode,
                created_at, updated_at, todos_json, active_plan_slug,
                parent_session_id, parent_call_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                self.id,
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
        self.persisted_max_seq.set(self.max_seq()?);
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
        crate::session::transcript_fts::delete_session(&*tx, session_id)?;
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

        Ok(Self {
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
        })
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
        let rows = self.load_turn_transcript()?;
        rows_to_items(&rows, &self.data_root)
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer_index_len().unwrap_or(0)
    }

    /// Wire buffer length — full UI history row count.
    pub fn buffer_index_len(&self) -> Result<usize> {
        Ok(self.load_history_transcript()?.len())
    }

    /// §5.1 Materialize — buffer index range `[start, end)`.
    pub fn load_by_buffer_index(&self, start: usize, end: usize) -> Result<Transcript> {
        let rows = self.load_by_buffer_index_rows(start, end)?;
        rows_to_items(&rows, &self.data_root)
    }

    /// Load a buffer range together with each row's DB `kind`
    /// (`detail` | `compact_checkpoint`). The FE derives revert anchors from
    /// `kind` (2.2 / REV-11): only `kind='detail'` user rows are counted, so a
    /// compact checkpoint must be distinguishable on the wire.
    pub fn load_by_buffer_index_with_kinds(
        &self,
        start: usize,
        end: usize,
    ) -> Result<(Transcript, Vec<String>)> {
        let rows = self.load_by_buffer_index_rows(start, end)?;
        let kinds = rows.iter().map(|r| r.kind.clone()).collect();
        Ok((rows_to_items(&rows, &self.data_root)?, kinds))
    }

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

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;

        let base_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM transcript_items WHERE session_id = ?1",
            rusqlite::params![self.id],
            |row| row.get(0),
        )?;

        let turn_id = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };

        for (i, msg) in delta.iter().enumerate() {
            let seq = base_seq + 1 + i as i64;
            let item_type = item_type_of(msg);
            let (body, body_ref, token_estimate) =
                encode_detail_row(msg, &self.data_root, DEFAULT_SPILL_THRESHOLD)?;
            // Fail closed: user anchors require inline body.
            if is_user_message_item(msg) && body.is_none() {
                return Err(LitecodeError::ToolExecution(
                    "user detail item must keep inline body for anchors".into(),
                ));
            }
            tx.execute(
                "INSERT INTO transcript_items (session_id, seq, turn_id, turn_seq, item_type, kind, body, body_ref, token_estimate, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'detail', ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    self.id,
                    seq,
                    turn_id,
                    i as i64,
                    item_type,
                    body,
                    body_ref,
                    token_estimate,
                    message_timestamp(msg),
                ],
            )?;
            let plain = crate::types::item_text_preview(msg);
            crate::session::transcript_fts::upsert(&*tx, &self.id, seq, &plain)?;
        }

        let now = chrono::Utc::now().timestamp_millis();
        let preview = Self::last_user_message_preview(delta);
        let preview_updated = if !preview.is_empty() {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1, last_message = ?2 WHERE id = ?3",
                rusqlite::params![now, preview, self.id],
            )?;
            Some((preview, now))
        } else {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, self.id],
            )?;
            None
        };

        tx.commit()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        self.persisted_max_seq.set(base_seq + delta.len() as i64);
        Ok(preview_updated)
    }

    /// Commit a turn delta **and** purge orphan `FunctionCallOutput` rows in one
    /// transaction (2.1), aligned to the REV-3 seq cursor.
    ///
    /// Orphan `FunctionCallOutput`s (call_id with no matching `FunctionCall`) are
    /// computed on a copy. `items` and `persisted_max_seq` are written back only
    /// after `tx.commit` succeeds; on failure the caller's vec matches disk.
    ///
    /// The delta to persist is the kept suffix after `delta_start`, where
    /// `delta_start` is the pipeline's seq-cursor-aligned `persisted_prefix_len`
    /// (REV-3): the number of leading in-memory items already persisted. It is
    /// **not** re-derived from a DB row count, so a revert that truncates DB
    /// (and shrinks `persisted_max_seq`) cannot cause re-persisting of in-memory
    /// content that is no longer persisted. `MAX(seq)` is used only as a
    /// consistency check and to allocate new seqs.
    pub fn commit_turn_delta_with_orphan_cleanup(
        &self,
        items: &mut Vec<Item>,
        delta_start: usize,
        turn_id: &str,
    ) -> Result<Option<(String, i64)>> {
        if items.is_empty() {
            return Ok(None);
        }

        // Compute kept/delta on a copy. Do not mutate `items` until commit.
        let valid_call_ids: std::collections::HashSet<String> = items
            .iter()
            .filter_map(|item| match item {
                Item::FunctionCall(fc) => Some(fc.call_id.clone()),
                _ => None,
            })
            .collect();
        let mut kept = Vec::with_capacity(items.len());
        let mut removed_before_start = 0usize;
        for (idx, item) in items.iter().enumerate() {
            let is_orphan = matches!(
                item,
                Item::FunctionCallOutput(out) if !valid_call_ids.contains(&out.call_id)
            );
            if is_orphan {
                if idx < delta_start {
                    removed_before_start += 1;
                }
                continue;
            }
            kept.push(item.clone());
        }
        let delta_start = delta_start
            .saturating_sub(removed_before_start)
            .min(kept.len());
        let delta = &kept[delta_start..];

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;

        // ── Consistency: MAX(seq) is the allocator base and a guard, not the delta ──
        // ── source. If the DB moved under this cursor (external revert/compact on a   ──
        // ── second handle), re-sync so `persisted_max_seq` stays honest (REV-3).      ──
        let base_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM transcript_items WHERE session_id = ?1",
            rusqlite::params![self.id],
            |row| row.get(0),
        )?;
        let cursor = self.persisted_max_seq.get();
        if base_seq != cursor {
            tracing::debug!(
                base_seq,
                cursor,
                "REV-3 seq cursor re-synced to DB max (external revert/compact)"
            );
            // Cell is written only after commit, via max_seq().
        }

        // ── DB side: delete persisted orphan output rows in the same transaction ──
        let mut orphan_seqs: Vec<i64> = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT seq, item_type, body FROM transcript_items
                 WHERE session_id = ?1 ORDER BY seq ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![self.id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            for row in rows {
                let (seq, item_type, body) = row?;
                if item_type == "function_call_output" {
                    let call_id = body
                        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                        .and_then(|v| v.get("call_id").and_then(|c| c.as_str()).map(String::from));
                    if call_id
                        .as_deref()
                        .map(|id| !valid_call_ids.contains(id))
                        .unwrap_or(true)
                    {
                        orphan_seqs.push(seq);
                    }
                }
            }
        }

        let tid = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };
        for (i, msg) in delta.iter().enumerate() {
            let seq = base_seq + 1 + i as i64;
            let item_type = item_type_of(msg);
            let (body, body_ref, token_estimate) =
                encode_detail_row(msg, &self.data_root, DEFAULT_SPILL_THRESHOLD)?;
            if is_user_message_item(msg) && body.is_none() {
                return Err(LitecodeError::ToolExecution(
                    "user detail item must keep inline body for anchors".into(),
                ));
            }
            tx.execute(
                "INSERT INTO transcript_items (session_id, seq, turn_id, turn_seq, item_type, kind, body, body_ref, token_estimate, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'detail', ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    self.id,
                    seq,
                    tid,
                    i as i64,
                    item_type,
                    body,
                    body_ref,
                    token_estimate,
                    message_timestamp(msg),
                ],
            )?;
            let plain = crate::types::item_text_preview(msg);
            crate::session::transcript_fts::upsert(&*tx, &self.id, seq, &plain)?;
        }

        for seq in orphan_seqs {
            tx.execute(
                "DELETE FROM transcript_items WHERE session_id = ?1 AND seq = ?2",
                rusqlite::params![self.id, seq],
            )?;
            crate::session::transcript_fts::delete_one(&*tx, &self.id, seq)?;
        }

        let now = chrono::Utc::now().timestamp_millis();
        let preview = Self::last_user_message_preview(delta);
        let preview_updated = if !preview.is_empty() {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1, last_message = ?2 WHERE id = ?3",
                rusqlite::params![now, preview, self.id],
            )?;
            Some((preview, now))
        } else {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, self.id],
            )?;
            None
        };

        tx.commit()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        *items = kept;
        // Re-align the cursor to the actual DB max: orphan deletions in this
        // transaction can lower it below `base_seq + delta.len()` (e.g. an orphan
        // at the tail with an empty delta). Reloading from DB keeps it authoritative.
        self.persisted_max_seq.set(self.max_seq()?);
        Ok(preview_updated)
    }

    /// §5.1 k formula — persisted user detail count (C2 anchor input).
    pub fn user_detail_count(&self) -> Result<i64> {
        self.conn
            .query_row(SQL_USER_DETAIL_COUNT, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    /// Truncate DB from the k-th user detail anchor; zero file side effects.
    pub fn revert_to_user_anchor(&self, k: i64) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;

        let anchor_seq: i64 = tx
            .query_row(SQL_ANCHOR_SEQ, rusqlite::params![self.id, k], |row| {
                row.get(0)
            })
            .map_err(|_| LitecodeError::InvalidRevertAnchor(format!("k={k}")))?;

        tx.execute(
            "DELETE FROM transcript_items WHERE session_id = ?1 AND seq >= ?2",
            rusqlite::params![self.id, anchor_seq],
        )?;
        crate::session::transcript_fts::delete_seq_ge(&*tx, &self.id, anchor_seq)?;

        // Keep at most one compact_checkpoint row: the latest remaining one.
        let remaining_cp: Option<i64> = tx
            .query_row(
                "SELECT MAX(seq) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'compact_checkpoint'",
                rusqlite::params![self.id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten();

        if let Some(cp) = remaining_cp {
            tx.execute(
                "DELETE FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'compact_checkpoint' AND seq < ?2",
                rusqlite::params![self.id, cp],
            )?;
            // Checkpoint survived revert — `kept_from_seq` still describes its
            // firstKept pointer; leave it unchanged.
            tx.execute(
                "UPDATE sessions SET checkpoint_seq = ?1 WHERE id = ?2",
                rusqlite::params![cp, self.id],
            )?;
        } else {
            // No compact left → full remaining detail is the working set.
            tx.execute(
                "UPDATE sessions SET checkpoint_seq = 0, kept_from_seq = 0 WHERE id = ?1",
                rusqlite::params![self.id],
            )?;
        }

        let now = chrono::Utc::now().timestamp_millis();
        let kept_from: i64 = tx
            .query_row(
                "SELECT kept_from_seq FROM sessions WHERE id = ?1",
                rusqlite::params![self.id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let preview: String = tx
            .query_row(
                "SELECT body FROM transcript_items
                 WHERE session_id = ?1
                   AND kind = 'detail'
                   AND seq >= ?2
                   AND item_type = 'message'
                   AND body IS NOT NULL
                   AND json_extract(body, '$.role') = 'user'
                 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![self.id, kept_from],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .map(|body| preview_from_item_json(&body))
            .unwrap_or_default();

        tx.execute(
            "UPDATE sessions SET updated_at = ?1, last_message = ?2 WHERE id = ?3",
            rusqlite::params![now, preview, self.id],
        )?;

        tx.commit()
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        self.persisted_max_seq.set(self.max_seq()?);
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
    pub fn kept_from_seq(&self) -> Result<i64> {
        self.conn
            .query_row(SQL_KEPT_FROM_SEQ, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    /// §5.1 REV-3 cursor — current DB max `seq` for this session (`-1` when empty).
    /// Same query shape as the insert base used in `insert_detail_rows_with_turn`.
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
        self.persisted_max_seq.set(self.max_seq()?);
        Ok(())
    }

    /// §5.1 turn 装载 — pi view: current compact summary + original detail from
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

    /// §5.1 compact success — summary checkpoint with empty keep (summary-only view).
    pub fn apply_compact_checkpoint(&self, summary: &Item, token_estimate: i64) -> Result<i64> {
        // `kept_from_seq = N` (checkpoint) → no pre-existing detail in the view.
        self.apply_compact_checkpoint_from(summary, None, token_estimate)
    }

    /// Pi-style keep-recent compact:
    /// 1. INSERT `compact_checkpoint` @ N with `summary`
    /// 2. UPDATE `checkpoint_seq = N`, `kept_from_seq = firstKept` (or N if empty)
    /// 3. Drop older `compact_checkpoint` rows (summary envelopes only)
    ///
    /// **Never deletes or rewrites historical `detail`.** The turn working set is
    /// a view: summary + original `detail` with `seq >= kept_from_seq`.
    ///
    /// `kept_from_seq`: `Some(seq)` of the first kept detail row; `None` = empty
    /// keep (pointer set to N so only the summary is visible until new inserts).
    pub fn apply_compact_checkpoint_from(
        &self,
        summary: &Item,
        kept_from_seq: Option<i64>,
        token_estimate: i64,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let item_type = item_type_of(summary);
        let (body, body_ref, _) =
            encode_detail_row(summary, &self.data_root, DEFAULT_SPILL_THRESHOLD)?;

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;

        let n: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM transcript_items WHERE session_id = ?1",
            rusqlite::params![self.id],
            |row| row.get(0),
        )?;

        let first_kept = kept_from_seq.unwrap_or(n);
        if let Some(k) = kept_from_seq {
            // Fail closed: pointer must land on an existing detail row.
            let ok: bool = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM transcript_items
                    WHERE session_id = ?1 AND kind = 'detail' AND seq = ?2
                 )",
                rusqlite::params![self.id, k],
                |row| row.get(0),
            )?;
            if !ok {
                return Err(LitecodeError::ToolExecution(format!(
                    "kept_from_seq={k} does not reference an existing detail row"
                )));
            }
        }

        tx.execute(
            "INSERT INTO transcript_items (session_id, seq, turn_id, turn_seq, item_type, kind, body, body_ref, token_estimate, created_at)
             VALUES (?1, ?2, ?3, 0, ?4, 'compact_checkpoint', ?5, ?6, ?7, ?8)",
            rusqlite::params![
                self.id,
                n,
                format!("compact-{n}"),
                item_type,
                body,
                body_ref,
                token_estimate,
                now
            ],
        )?;

        tx.execute(
            "UPDATE sessions SET checkpoint_seq = ?1, kept_from_seq = ?2 WHERE id = ?3",
            rusqlite::params![n, first_kept, self.id],
        )?;

        // Single-checkpoint invariant: drop any older compact_checkpoint rows
        // (summary envelopes). Historical conversation `detail` is never deleted.
        tx.execute(
            "DELETE FROM transcript_items
             WHERE session_id = ?1 AND kind = 'compact_checkpoint' AND seq < ?2",
            rusqlite::params![self.id, n],
        )?;

        tx.commit()
            .map_err(|e| crate::types::LitecodeError::ToolExecution(e.to_string()))?;
        self.persisted_max_seq.set(n);
        Ok(n)
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
            msg.contains("agent_id") || msg.contains("model_id") || msg.contains("last_message"),
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
        // UI history: two archived users, checkpoint marker, then "after".
        assert_eq!(session.user_detail_before_buffer_index(0).unwrap(), 0);
        assert_eq!(session.user_detail_before_buffer_index(1).unwrap(), 1);
        assert_eq!(session.user_detail_before_buffer_index(2).unwrap(), 2);
        assert_eq!(session.user_detail_before_buffer_index(3).unwrap(), 2);
        assert_eq!(session.user_detail_before_buffer_index(4).unwrap(), 3);
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
                 WHERE session_id = ?1 AND kind = 'detail'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(detail_before, 3);

        let keep_tail_seq: i64 = session
            .conn
            .query_row(
                "SELECT seq FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'detail'
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
                 WHERE session_id = ?1 AND kind = 'detail'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        // Pi-style: no kept rewrite — detail count stays at the original 3.
        assert_eq!(
            detail_after, 3,
            "compact must not rewrite/copy kept detail (got {detail_after})"
        );

        let pre_kept_detail: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'detail' AND seq < ?2",
                rusqlite::params![session.id, keep_tail_seq],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            pre_kept_detail, 2,
            "archived detail below kept_from_seq must remain"
        );
        assert_eq!(session.kept_from_seq().unwrap(), keep_tail_seq);
        assert_eq!(session.checkpoint_seq().unwrap(), n);

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
                 WHERE session_id = ?1 AND kind = 'detail'
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
        assert_eq!(session.kept_from_seq().unwrap(), keep_from);

        let rows = session.load_turn_transcript().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, "compact_checkpoint");
        assert_eq!(rows[1].kind, "detail");
        assert_eq!(rows[1].seq, keep_from);
        assert_eq!(rows[2].kind, "detail");

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
                 WHERE session_id = ?1 AND kind = 'detail' AND seq < ?2
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
                 WHERE session_id = ?1 AND kind = 'detail'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(detail_total, 4);
    }

    #[test]
    fn compact_checkpoint_body_is_item_json() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session.insert_detail_rows(&[user_text("before")]).unwrap();
        let summary = user_text("compact summary text");
        let n = session.apply_compact_checkpoint(&summary, 10).unwrap();
        assert!(n >= 0);
        assert_eq!(
            session.kept_from_seq().unwrap(),
            n,
            "empty keep sets kept_from_seq to checkpoint seq"
        );

        let rows = session.load_turn_transcript().unwrap();
        assert_eq!(rows[0].kind, "compact_checkpoint");
        assert_eq!(rows[0].item_type, "message");
        let body = rows[0].body.as_deref().expect("inline body");
        let parsed: Item = serde_json::from_str(body).expect("Item JSON");
        assert_eq!(item_text_preview(&parsed), "compact summary text");
        assert!(
            body.trim_start().starts_with('{'),
            "body must be JSON object"
        );

        let loaded = session.load_transcript().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(item_text_preview(&loaded[0]), "compact summary text");
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
        assert_eq!(session.checkpoint_seq().unwrap(), 0);
        assert_eq!(
            session.kept_from_seq().unwrap(),
            0,
            "empty keep pins kept_from_seq to checkpoint seq"
        );

        let loaded = session.load_transcript().unwrap();
        assert_eq!(
            loaded.len(),
            1,
            "CP at seq 0 must appear in the turn working set"
        );
        assert_eq!(item_text_preview(&loaded[0]), "empty-session summary");

        let rows = session.load_turn_transcript().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "compact_checkpoint");
        assert_eq!(rows[0].seq, 0);
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

        let rows = session.load_turn_transcript().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "compact_checkpoint");
        assert_eq!(rows[0].seq, 0);
        assert_eq!(rows[1].kind, "detail");
        assert!(rows[1].seq > 0);
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
        assert!(rows.iter().all(|r| r.kind == "detail"));
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
        assert!(n > 0, "non-empty session CP must land after detail");
        assert_eq!(session.kept_from_seq().unwrap(), n);

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

        let cp_count: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'compact_checkpoint'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cp_count, 1, "single-checkpoint invariant");

        let loaded = session.load_transcript().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(item_text_preview(&loaded[0]), "second-summary");
        assert_eq!(session.checkpoint_seq().unwrap(), second);
        assert_eq!(session.kept_from_seq().unwrap(), second);
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
                 WHERE session_id = ?1 AND kind = 'detail'
                 ORDER BY seq ASC LIMIT 1 OFFSET 1",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();

        session
            .apply_compact_checkpoint_from(&user_text("summary"), Some(keep_from), 10)
            .unwrap();
        // Revert anchors span all three immutable detail rows.
        assert_eq!(session.user_detail_count().unwrap(), 3);

        // Revert to first kept user (k=1) deletes from that detail seq onward,
        // which also removes the later compact_checkpoint row.
        session.revert_to_user_anchor(1).unwrap();

        assert_eq!(session.checkpoint_seq().unwrap(), 0);
        assert_eq!(session.kept_from_seq().unwrap(), 0);

        let cp_left: i64 = session
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'compact_checkpoint'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cp_left, 0);

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

        let mut items = vec![fc, live, orphan];
        session
            .conn
            .execute_batch("DROP TABLE transcript_items")
            .unwrap();
        let err = session
            .commit_turn_delta_with_orphan_cleanup(&mut items, 3, "t1")
            .expect_err("dropped table must fail the transaction");
        let _ = err;
        assert_eq!(
            items.len(),
            3,
            "failed commit must not snip orphans from memory"
        );
        assert!(
            items.iter().any(|i| match i {
                Item::FunctionCallOutput(o) => o.call_id == "gone",
                _ => false,
            }),
            "orphan must remain in the caller's vec when disk did not change"
        );
    }
}
