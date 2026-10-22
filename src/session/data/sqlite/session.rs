use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::conn::SharedDb;

use crate::authority::responses::{
    AssistantRole, InputMessage, InputRole, MessageItem, OutputStatus,
};
use crate::platform_knobs::{ContextMode, ThinkingTier};
use crate::session::data::command::{SessionListPreview, UserAnchorWindow};
use crate::session::estimate::compute_token_estimate;
use crate::session::event::{
    EventDraft, EventType, Seq, SessionEvent, finalize_draft, item_from_event,
    skip_empty_assistant_item, spine_agent_item,
};
use crate::session::model::{CompactedBody, LogState, SESSION_LOG_SCHEMA_VERSION};
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

/// One provider bill, stored on the `request/header` row that opened the request.
///
/// The item body is not this record. Cache hit and miss split `prompt_tokens`;
/// they are not added again when summing cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_hit_tokens: u64,
    pub cache_miss_tokens: u64,
}

impl ProviderUsage {
    /// Round-trip tokens billed for this request.
    pub fn billed(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }

    pub fn from_meter_last(meter: &SessionContextMeter) -> Self {
        Self {
            prompt_tokens: meter.prompt_tokens,
            completion_tokens: meter.completion_tokens,
            cache_hit_tokens: meter.cache_hit_tokens,
            cache_miss_tokens: meter.cache_miss_tokens,
        }
    }
}

/// Every recorded request bill for one session, oldest first folded in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderUsageRollup {
    pub latest_seq: Option<u64>,
    pub latest: Option<ProviderUsage>,
    pub billed_tokens: u64,
    pub request_count: u64,
}

impl ProviderUsageRollup {
    pub fn from_rows(rows: impl IntoIterator<Item = (i64, ProviderUsage)>) -> Self {
        let mut rollup = Self::default();
        for (seq, usage) in rows {
            if seq < 0 {
                continue;
            }
            rollup.billed_tokens = rollup.billed_tokens.saturating_add(usage.billed());
            rollup.request_count = rollup.request_count.saturating_add(1);
            rollup.latest_seq = Some(seq as u64);
            rollup.latest = Some(usage);
        }
        rollup
    }
}

/// Prompt the occupancy ring may show.
///
/// A recorded bill is that prompt while the request is still the latest context.
/// A compact checkpoint with a higher seq retires it, even if the meter row still
/// holds the pre-compact figures. No bill at all falls back to the meter.
pub fn occupancy_usage(
    rollup: &ProviderUsageRollup,
    compacted_seq: Option<u64>,
    meter: &SessionContextMeter,
) -> Option<ProviderUsage> {
    if let (Some(seq), Some(usage)) = (rollup.latest_seq, rollup.latest.clone()) {
        let retired = compacted_seq.is_some_and(|compacted| compacted > seq);
        if retired {
            return None;
        }
        return Some(usage);
    }
    if meter.is_empty() {
        None
    } else {
        Some(ProviderUsage::from_meter_last(meter))
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

/// Append-origin user messages. Replace summaries are not user rows.
pub const SQL_USER_DETAIL_COUNT: &str = "\
SELECT COUNT(*) FROM transcript_items t
WHERE t.session_id = ?
  AND t.kind = 'item/user'";

/// The revert anchor is the user row's own `seq`. Anything else is not a cut point.
pub(crate) fn require_user_anchor(conn: &Connection, session_id: &str, seq: i64) -> Result<()> {
    if seq < 0 {
        return Err(LitecodeError::InvalidRevertAnchor(format!("seq={seq}")));
    }
    let kind: Option<String> = conn
        .query_row(
            "SELECT kind FROM transcript_items WHERE session_id = ?1 AND seq = ?2",
            rusqlite::params![session_id, seq],
            |row| row.get(0),
        )
        .optional()?;
    if kind.as_deref() != Some("item/user") {
        return Err(LitecodeError::InvalidRevertAnchor(format!("seq={seq}")));
    }
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
        preview: Option<SessionListPreview>,
        /// Existing rows changed in place and must be re-sent to live clients.
        sealed_seqs: Vec<Seq>,
        /// Rows appended by this commit (new seqs), in log order.
        appended_seqs: Vec<Seq>,
        /// True when this commit sealed or appended at least one row.
        mutated: bool,
    },
}

/// The three write primitives. All durable mutation goes through [`Session::apply`].
#[derive(Debug)]
pub enum SessionApply {
    Append(EventDraft),
    Seal {
        seq: Seq,
        item: Item,
    },
    /// Delete every row at or after this user row's `seq`.
    Truncate {
        anchor_seq: i64,
    },
}

pub enum ApplyOutcome {
    Appended(Seq),
    /// Sealed an existing row in place; carries the real seq it rewrote.
    Sealed {
        seq: Seq,
    },
    /// Truncated from a user row's `seq`. `anchor` is that seq: everything at
    /// or after it is gone.
    Truncated {
        anchor: i64,
    },
}

/// What an `InsertDetails` mutation actually wrote.
pub struct InsertDetailsOutcome {
    /// `(last_message, updated_at)` when the delta updated the session preview.
    pub preview: Option<(String, i64)>,
    /// Inclusive `(first, last)` seq of the rows appended, when any were.
    pub seq_range: Option<(Seq, Seq)>,
}

impl SessionApply {
    pub fn as_kind_name(&self) -> &'static str {
        match self {
            Self::Append(_) => "append",
            Self::Seal { .. } => "seal",
            Self::Truncate { .. } => "truncate",
        }
    }
}

/// Incremental surface + seq identity for one live Session. Hydrated once on
/// open; write primitives update it under the write gate.
///
/// Two seq cursors live here and are never mixed:
/// * `next_seq` is the **allocation high-water**: the seq the next append will
///   take. It is persisted in `sessions.next_seq` and only ever advances; a
///   truncate never lowers it, so a deleted seq is never handed out again.
/// * `active_max_seq` is the `MAX(seq)` of the rows currently in the log (`-1`
///   when empty). It may drop when a truncate removes the tail and is what the
///   stale-turn / revert guard is based on.
#[derive(Debug, Clone)]
struct LogProjection {
    surface: Surface,
    next_seq: Seq,
    active_max_seq: i64,
    items_by_seq: HashMap<Seq, Item>,
    reminders_by_seq: HashMap<Seq, crate::reminder::Reminder>,
    compacted_seqs: HashSet<Seq>,
    /// Seqs whose row is still in flight. Tracked from the log's own lifecycle
    /// state, never from the payload's `status` field.
    in_progress: HashSet<Seq>,
}

impl Default for LogProjection {
    fn default() -> Self {
        Self {
            surface: Surface::default(),
            next_seq: 0,
            active_max_seq: -1,
            items_by_seq: HashMap::new(),
            reminders_by_seq: HashMap::new(),
            compacted_seqs: HashSet::new(),
            in_progress: HashSet::new(),
        }
    }
}

impl LogProjection {
    /// `MAX(seq)` of the live rows, `-1` when the log is empty.
    fn active_max_seq(&self) -> i64 {
        self.active_max_seq
    }

    fn apply_event(&mut self, event: &SessionEvent, item: Option<&Item>) -> Result<()> {
        if let Some(plan) = plan_surface(&self.surface, event)? {
            if let crate::session::surface::SurfacePlan::Replace { start_idx, len, .. } = &plan {
                let shadowed: Vec<Seq> = self.surface.nodes[*start_idx..*start_idx + *len].to_vec();
                for seq in shadowed {
                    self.items_by_seq.remove(&seq);
                    self.reminders_by_seq.remove(&seq);
                    self.compacted_seqs.remove(&seq);
                }
            }
            apply_plan(&mut self.surface, plan);
        }
        self.next_seq = self.next_seq.max(event.seq.saturating_add(1));
        self.active_max_seq = self.active_max_seq.max(event.seq as i64);
        if event.state == LogState::InProgress {
            self.in_progress.insert(event.seq);
        } else {
            self.in_progress.remove(&event.seq);
        }
        if let Some(item) = item {
            if self.surface.nodes.contains(&event.seq) {
                self.items_by_seq.insert(event.seq, item.clone());
            }
        } else if event.event_type.enters_spine()
            && self.surface.nodes.contains(&event.seq)
            && let Ok(assembled) = spine_agent_item(event)
        {
            self.items_by_seq.insert(event.seq, assembled);
        }
        if self.surface.nodes.contains(&event.seq) {
            match &event.event_type {
                EventType::Reminder(_) => {
                    if let Ok(reminder) = serde_json::from_value(event.data.clone()) {
                        self.reminders_by_seq.insert(event.seq, reminder);
                    }
                }
                EventType::Compacted => {
                    self.compacted_seqs.insert(event.seq);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn seal_item(&mut self, seq: Seq, item: &Item) {
        if self.surface.nodes.contains(&seq) {
            self.items_by_seq.insert(seq, item.clone());
        }
        self.in_progress.remove(&seq);
    }

    /// Replace the cached payload of a row that is still in flight.
    fn touch_item(&mut self, seq: Seq, item: &Item) {
        if self.surface.nodes.contains(&seq) {
            self.items_by_seq.insert(seq, item.clone());
        }
        self.in_progress.insert(seq);
    }
}

pub struct Session {
    db: Rc<SharedDb>,
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
    projection: RefCell<LogProjection>,
}

impl Session {
    fn conn(&self) -> &Connection {
        self.db.conn()
    }
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
    tx: &Connection,
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
    let item_type = if matches!(event.event_type, EventType::Reminder(_)) {
        "message".to_string()
    } else if let Some(item) = item {
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
    if let Some(body_ref) = &body_ref
        && let Some(blob_id) = blob_id_from_ref(body_ref)
    {
        let rel = crate::session::data::blob::rel_path_for(&blob_id);
        super::ops::register_blob_ref(tx, &blob_id, session_id, seq, 0, &rel.to_string_lossy())?;
    }
    Ok(())
}

fn blob_id_from_ref(body_ref: &str) -> Option<String> {
    let rest = body_ref.strip_prefix(BLOB_PREFIX)?;
    let (id, _) = rest.split_once(']')?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Whether the row at `seq` has settled, i.e. whether its content is fixed for
/// good. The database is the authority: the projection is a cache and may not
/// have projected this row at all.
fn row_is_settled(tx: &Connection, session_id: &str, seq: Seq) -> Result<bool> {
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM transcript_items WHERE session_id = ?1 AND seq = ?2",
            rusqlite::params![session_id, seq as i64],
            |row| row.get(0),
        )
        .optional()?;
    Ok(state.as_deref() == Some(LogState::Final.as_str()))
}

/// Guards shared by every rewrite of a streaming row.
///
/// Rewriting a row is the one write that changes content at an existing `seq`, so
/// it is allowed only while the row is in flight: `final` means the content at
/// this `(session_id, seq)` is fixed for good, and the derived index is built on
/// that with no mechanism for noticing a change after projection. A late rewrite
/// of a settled row would therefore not fail loudly — it would leave the index
/// quietly serving text that no longer exists.
fn assert_stream_row_rewritable(
    tx: &Connection,
    session_id: &str,
    seq: Seq,
    op: &str,
) -> Result<()> {
    let current: Option<(String, String)> = tx
        .query_row(
            "SELECT event_type, state FROM transcript_items WHERE session_id = ?1 AND seq = ?2",
            rusqlite::params![session_id, seq as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((current_kind, current_state)) = current else {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "{op}: no row at seq {seq}"
        )));
    };
    if current_state != LogState::InProgress.as_str() {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "{op}: row {seq} is `{current_state}`; only a row in flight may be rewritten"
        )));
    }
    if !matches!(
        EventType::from_str_name(&current_kind),
        EventType::ItemAssistant | EventType::ItemToolCall
    ) {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "{op}: kind `{current_kind}` is not streamable"
        )));
    }
    Ok(())
}

/// A streaming row's identity is fixed when it opens.
///
/// Update and seal may replace the payload — that is what streaming is — but they
/// may not re-point the row at a different item. Without this, a mis-addressed
/// write would merge two provider items into one `seq`, which is the very thing
/// "one item, one row" exists to prevent, and it would do it silently.
fn assert_stream_row_identity(
    tx: &Connection,
    session_id: &str,
    data_root: &Path,
    seq: Seq,
    item: &Item,
    op: &str,
) -> Result<()> {
    let events = load_events_range_on(tx, session_id, seq as i64, seq as i64 + 1, data_root)?;
    let Some(event) = events.into_iter().next() else {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "{op}: no row at seq {seq}"
        )));
    };
    // A body that is not an Item (a compaction checkpoint) is already refused by
    // the kind check; there is no identity to compare.
    let Ok(existing) = spine_agent_item(&event) else {
        return Ok(());
    };
    let held = item_type_of(&existing);
    let written = item_type_of(item);
    if held != written {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "{op}: row {seq} holds a `{held}` item; a `{written}` item may not overwrite it"
        )));
    }
    if let (Some(held), Some(written)) = (item_log_id(&existing), item_log_id(item))
        && held != written
    {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "{op}: row {seq} belongs to `{held}`, not `{written}`"
        )));
    }
    Ok(())
}

/// Write `item` over the row at `seq`, keeping the row's lifecycle state.
fn rewrite_event_row_body(
    tx: &Connection,
    session_id: &str,
    data_root: &Path,
    seq: Seq,
    item: &Item,
    state: LogState,
) -> Result<()> {
    let seq_i = seq as i64;
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
            state.as_str(),
            session_id,
            seq_i,
        ],
    )?;
    // Re-encoding moves the body: the blob ref for this seq must track the new
    // body_ref, otherwise startup GC treats the spilled blob as orphaned and
    // deletes a body the transcript still references.
    tx.execute(
        "DELETE FROM session_blob_refs WHERE session_id = ?1 AND seq = ?2",
        rusqlite::params![session_id, seq_i],
    )?;
    if let Some(body_ref) = &body_ref
        && let Some(blob_id) = blob_id_from_ref(body_ref)
    {
        let rel = crate::session::data::blob::rel_path_for(&blob_id);
        super::ops::register_blob_ref(tx, &blob_id, session_id, seq_i, 0, &rel.to_string_lossy())?;
    }
    Ok(())
}

/// Replace the payload of a row that is still in flight, leaving it in flight.
fn update_event_row(
    tx: &Connection,
    session_id: &str,
    data_root: &Path,
    seq: Seq,
    item: &Item,
) -> Result<()> {
    assert_stream_row_rewritable(tx, session_id, seq, "update_stream_item")?;
    assert_stream_row_identity(tx, session_id, data_root, seq, item, "update_stream_item")?;
    rewrite_event_row_body(tx, session_id, data_root, seq, item, LogState::InProgress)
}

fn seal_event_row(
    tx: &Connection,
    session_id: &str,
    data_root: &Path,
    seq: Seq,
    item: &Item,
) -> Result<()> {
    assert_stream_row_rewritable(tx, session_id, seq, "seal_item")?;
    assert_stream_row_identity(tx, session_id, data_root, seq, item, "seal_item")?;
    // Settling is what this write *is*; the payload's `status` field is provider
    // content and cannot decide storage state.
    rewrite_event_row_body(tx, session_id, data_root, seq, item, LogState::Final)
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
    tx: &Connection,
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
fn refresh_compact_pointers_from_log(tx: &Connection, session_id: &str) -> Result<()> {
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
           AND (
             kind IN (
               'item/user', 'item/assistant', 'item/tool_call', 'item/tool_result',
               'plan/execute'
             )
             OR kind LIKE 'reminder/%'
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
        let blob_id = crate::session::data::put_text(data_root, &body_json)?;
        return Ok((
            None,
            Some(format!("{BLOB_PREFIX}{blob_id}]")),
            token_estimate,
        ));
    }
    Ok((Some(body_json), None, token_estimate))
}

fn row_body_text(row: &TranscriptRow, data_root: &Path) -> Result<String> {
    if let Some(body) = &row.body {
        return Ok(body.clone());
    }
    if let Some(body_ref) = &row.body_ref {
        return load_blob_text(body_ref, data_root);
    }
    Err(crate::types::LitecodeError::ToolExecution(format!(
        "transcript row seq {} has no body",
        row.seq
    )))
}

fn row_to_item(row: &TranscriptRow, data_root: &Path) -> Result<Item> {
    if crate::reminder::ReminderKind::is_wire(row.kind.as_str()) {
        let raw = row_body_text(row, data_root)?;
        let reminder: crate::reminder::Reminder = serde_json::from_str(&raw)?;
        return Ok(crate::reminder::render_item(&reminder));
    }
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
        "compact_checkpoint" | "detail" | "item/user" | "item/assistant" | "item/tool_call"
        | "item/tool_result" | "plan/execute" => {
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
    match crate::session::data::read_bytes(data_root, id) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|e| LitecodeError::SessionStorage(format!("blob {id} is not utf-8: {e}"))),
        Err(_) => {
            let blob_path = blob_dir(data_root).join(format!("{id}.txt"));
            fs::read_to_string(blob_path).map_err(Into::into)
        }
    }
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
    let event_type_name = event_type.clone();
    let event_type = EventType::from_str_name(&event_type);
    let state = LogState::from_str_name(state.as_deref().unwrap_or("final"));
    let source_seqs = match source_seqs.as_deref() {
        None | Some("") => None,
        Some(raw) => Some(serde_json::from_str(raw)?),
    };
    // Retired model reminders stay in the log for seq continuity. They are not
    // a live kind, so they do not enter the spine and are not sent to the model.
    if event_type_name == "reminder/model" || kind == "reminder/model" {
        let data = match body.as_deref() {
            Some(raw) if !raw.is_empty() => serde_json::from_str(raw)?,
            _ => serde_json::Value::Null,
        };
        return Ok(SessionEvent {
            seq,
            time: created_at,
            event_type: EventType::Unknown("reminder/model".into()),
            data,
            surface_op: None,
            source_seqs,
            ignorable: true,
            state,
        });
    }
    if matches!(event_type, EventType::Reminder(_)) || crate::reminder::ReminderKind::is_wire(&kind)
    {
        let data = match body.as_deref() {
            Some(raw) if !raw.is_empty() => serde_json::from_str(raw)?,
            _ => {
                return Err(LitecodeError::InvalidSessionEvent(format!(
                    "reminder row {seq} has no body"
                )));
            }
        };
        let surface_op = match surface_op.as_str() {
            "" => None,
            raw => Some(serde_json::from_str(raw)?),
        };
        return Ok(SessionEvent {
            seq,
            time: created_at,
            event_type,
            data,
            surface_op,
            source_seqs,
            ignorable: false,
            state,
        });
    }
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

fn clip_preview(content: &str) -> String {
    let preview: String = content.chars().take(200).collect();
    if content.chars().count() > 200 {
        format!("{preview}…")
    } else {
        preview
    }
}

fn preview_from_item_json(body: &str) -> String {
    match serde_json::from_str::<Item>(body) {
        Ok(item) => clip_preview(&item_text_preview(&item)),
        Err(_) => String::new(),
    }
}

fn write_session_list_previews(
    tx: &Connection,
    session_id: &str,
    items: &[Item],
) -> Result<Option<SessionListPreview>> {
    let now = chrono::Utc::now().timestamp_millis();
    let user = Session::last_user_message_preview(items);
    let assistant = Session::last_assistant_message_preview(items);
    let user = (!user.is_empty()).then_some(user);
    let assistant = (!assistant.is_empty()).then_some(assistant);
    match (&user, &assistant) {
        (Some(user), Some(assistant)) => {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1, last_message = ?2, last_assistant = ?3 WHERE id = ?4",
                rusqlite::params![now, user, assistant, session_id],
            )?;
        }
        (Some(user), None) => {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1, last_message = ?2 WHERE id = ?3",
                rusqlite::params![now, user, session_id],
            )?;
        }
        (None, Some(assistant)) => {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1, last_assistant = ?2 WHERE id = ?3",
                rusqlite::params![now, assistant, session_id],
            )?;
        }
        (None, None) => {
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, session_id],
            )?;
            return Ok(None);
        }
    }
    Ok(Some(SessionListPreview {
        updated_at: now,
        user,
        assistant,
    }))
}

pub(crate) fn load_meta_on(
    conn: &Connection,
    session_id: &str,
) -> Result<crate::session::model::SessionMeta> {
    let (
        project,
        created_at,
        parent_session_id,
        parent_call_id,
        responsibility,
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
        plan_revision,
        preview,
    ): (
        String,
        i64,
        Option<String>,
        Option<String>,
        String,
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
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT project, created_at, parent_session_id, parent_call_id, responsibility, subagent_depth,
                    agent_id, model_id, thinking_tier, context_mode, updated_at,
                    compacted_seq, spine_from, todos_json, active_plan_slug, plan_revision, last_message
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
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                ))
            },
        )
        .map_err(|_| LitecodeError::SessionNotFound(session_id.to_string()))?;
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
        id: session_id.to_string(),
        project,
        created_at,
        parent_session_id,
        parent_call_id,
        responsibility,
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
        plan_revision,
        preview,
    })
}

pub(crate) fn load_context_meter_on(
    conn: &Connection,
    session_id: &str,
) -> Result<SessionContextMeter> {
    let row: Option<(i64, i64, i64, i64, i64, i64, i64, i64)> = conn
        .query_row(
            "SELECT prompt_tokens, completion_tokens,
                    cache_hit_tokens, cache_miss_tokens,
                    cum_prompt_tokens, cum_completion_tokens,
                    cum_cache_hit_tokens, cum_cache_miss_tokens
             FROM session_context_meter WHERE session_id = ?1",
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

pub(crate) fn load_events_on(
    conn: &Connection,
    session_id: &str,
    data_root: &Path,
) -> Result<Vec<SessionEvent>> {
    load_events_range_on(conn, session_id, 0, i64::MAX, data_root)
}

/// `(last_seq, next_seq)` for the client buffer cursor. The two are independent:
///
/// * `last_seq` is the **active** `MAX(seq)` of the live log (`-1` when empty).
///   It moves back after a revert, which is exactly what the client must see.
/// * `next_seq` is the **persisted allocation high-water** (`sessions.next_seq`).
///   It never moves back, so after a revert it can exceed `last_seq + 1` and the
///   log may be sparse. The client uses it only as the fetch high-water
///   `[last_seq + 1, next_seq)`, which is empty right after a revert and resumes
///   at the next append (which always lands on the high-water, not in a hole).
///
/// Empty log with no history → `(-1, 0)`.
pub(crate) fn load_wire_seq_cursor_on(conn: &Connection, session_id: &str) -> Result<(i64, u64)> {
    let last: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), -1) FROM transcript_items WHERE session_id = ?1",
        rusqlite::params![session_id],
        |row| row.get(0),
    )?;
    let persisted: i64 = conn
        .query_row(
            "SELECT next_seq FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    // Defensive: a well-formed row already holds `>= last + 1`, but heal a legacy
    // or inconsistent value forward so an append can never be aimed below an
    // existing row.
    let next = persisted.max(last.saturating_add(1)).max(0) as u64;
    Ok((last, next))
}

/// Caller-chosen window of `item/user` seqs. `before` / `after` are clamped.
const USER_ANCHOR_WINDOW_MAX: i64 = 64;

pub(crate) fn user_anchor_window_on(
    conn: &Connection,
    session_id: &str,
    anchor_seq: Option<i64>,
    before: i64,
    after: i64,
) -> Result<UserAnchorWindow> {
    let before = before.clamp(0, USER_ANCHOR_WINDOW_MAX);
    let after = after.clamp(0, USER_ANCHOR_WINDOW_MAX);
    let (split, anchor) = match anchor_seq {
        Some(seq) => (seq, user_seq_if_kind(conn, session_id, seq)?),
        None => match latest_user_seq(conn, session_id)? {
            Some(seq) => (seq, Some(seq)),
            None => {
                return Ok(UserAnchorWindow {
                    seqs: Vec::new(),
                    anchor: None,
                    has_more_before: false,
                    has_more_after: false,
                });
            }
        },
    };
    let (older, has_more_before) = user_seqs_beside(conn, session_id, split, before, true)?;
    let (newer, has_more_after) = user_seqs_beside(conn, session_id, split, after, false)?;
    let mut seqs = older;
    if let Some(seq) = anchor {
        seqs.push(seq);
    }
    seqs.extend(newer);
    Ok(UserAnchorWindow {
        seqs,
        anchor,
        has_more_before,
        has_more_after,
    })
}

fn user_seq_if_kind(conn: &Connection, session_id: &str, seq: i64) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT seq FROM transcript_items
         WHERE session_id = ?1 AND seq = ?2 AND kind = 'item/user'",
        rusqlite::params![session_id, seq],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn latest_user_seq(conn: &Connection, session_id: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT seq FROM transcript_items
         WHERE session_id = ?1 AND kind = 'item/user'
         ORDER BY seq DESC LIMIT 1",
        rusqlite::params![session_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// `older`: seqs strictly below `split`, ascending, closest `limit` rows.
/// `newer`: seqs strictly above `split`, ascending, closest `limit` rows.
fn user_seqs_beside(
    conn: &Connection,
    session_id: &str,
    split: i64,
    limit: i64,
    older: bool,
) -> Result<(Vec<i64>, bool)> {
    if limit == 0 {
        let cmp = if older { "<" } else { ">" };
        let sql = format!(
            "SELECT 1 FROM transcript_items
             WHERE session_id = ?1 AND kind = 'item/user' AND seq {cmp} ?2
             LIMIT 1"
        );
        let hit: Option<i64> = conn
            .query_row(&sql, rusqlite::params![session_id, split], |row| row.get(0))
            .optional()?;
        return Ok((Vec::new(), hit.is_some()));
    }
    let (cmp, order) = if older { ("<", "DESC") } else { (">", "ASC") };
    let sql = format!(
        "SELECT seq FROM transcript_items
         WHERE session_id = ?1 AND kind = 'item/user' AND seq {cmp} ?2
         ORDER BY seq {order} LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![session_id, split, limit + 1], |row| {
        row.get(0)
    })?;
    let mut seqs = Vec::new();
    for row in rows {
        seqs.push(row?);
    }
    let has_more = seqs.len() > limit as usize;
    if has_more {
        seqs.truncate(limit as usize);
    }
    if older {
        seqs.reverse();
    }
    Ok((seqs, has_more))
}

pub(crate) fn load_events_range_on(
    conn: &Connection,
    session_id: &str,
    from: i64,
    to: i64,
    data_root: &Path,
) -> Result<Vec<SessionEvent>> {
    if from >= to {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT seq, created_at, event_type, surface_op, source_seqs, kind, body, body_ref, state
         FROM transcript_items WHERE session_id = ?1 AND seq >= ?2 AND seq < ?3 ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id, from, to], |row| {
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
            session_id,
            seq,
            created_at,
            event_type,
            surface_op,
            source_seqs,
            kind,
            body,
            body_ref,
            state,
            data_root,
        )?);
    }
    // Seqs are strictly increasing (`ORDER BY seq`) but **not** necessarily
    // contiguous: a revert deletes the tail and the next append allocates from
    // the persisted high-water, leaving a hole where the deleted rows were. A
    // stable identity does not need to be a dense one, so no hole is an error.
    Ok(events)
}

impl Session {
    /// Load the durable session metadata without leaking Live or catalog
    /// projection fields into the session domain.
    pub fn meta(&self) -> Result<crate::session::model::SessionMeta> {
        load_meta_on(self.conn(), &self.id)
    }

    #[cfg(test)]
    pub(crate) fn ephemeral(project: &str, agent_id: &str, model_id: Option<&str>) -> Result<Self> {
        let db = Rc::new(SharedDb::open_ephemeral()?);
        Self::insert_new(
            db,
            project,
            agent_id,
            model_id,
            None,
            None,
            "",
            std::env::temp_dir().join("litecode"),
            None,
            true,
        )
    }

    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(crate) fn open_shared(
        db: Rc<SharedDb>,
        data_root: PathBuf,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: Option<&str>,
        parent_call_id: Option<&str>,
        responsibility: &str,
    ) -> Result<Self> {
        Self::insert_new(
            db,
            project,
            agent_id,
            model_id,
            parent_session_id,
            parent_call_id,
            responsibility,
            data_root,
            None,
            false,
        )
    }

    fn insert_new(
        db: Rc<SharedDb>,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: Option<&str>,
        parent_call_id: Option<&str>,
        responsibility: &str,
        data_root: PathBuf,
        db_path: Option<PathBuf>,
        ephemeral: bool,
    ) -> Result<Self> {
        let id = ulid::Ulid::new().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let model_id_owned = normalize_model_id(model_id);
        let parent_session_id_owned = parent_session_id.map(|s| s.to_string());
        let parent_call_id_owned = parent_call_id.map(|s| s.to_string());
        // Delegation depth: roots are 0, children are parent + 1. Persisted so
        // the limit survives restarts (DATA.md).
        let subagent_depth: u32 = match parent_session_id {
            None => 0,
            Some(parent) => db
                .conn()
                .query_row(
                    "SELECT subagent_depth FROM sessions WHERE id = ?1",
                    [parent],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .map(|depth| depth.max(0) as u32)
                .unwrap_or(0)
                .saturating_add(1),
        };
        db.conn().execute(
            "INSERT INTO sessions (
                id, schema_version, project, last_message, agent_id, model_id, created_at, updated_at,
                parent_session_id, parent_call_id, responsibility, subagent_depth
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                responsibility,
                subagent_depth,
            ],
        )?;
        let session = Self {
            db,
            id,
            project: project.to_string(),
            agent_id: agent_id.to_string(),
            model_id: model_id_owned,
            thinking_tier: ThinkingTier::default(),
            context_mode: ContextMode::default(),
            parent_session_id: parent_session_id_owned,
            parent_call_id: parent_call_id_owned,
            data_root,
            db_path,
            ephemeral,
            persisted_max_seq: Cell::new(0),
            write_gate: Mutex::new(()),
            projection: RefCell::new(LogProjection::default()),
        };
        session.hydrate_projection()?;
        Ok(session)
    }

    /// Delete a session and every descendant. Returns the ids actually removed,
    /// children first, so the caller can write one tombstone per session.
    /// Delete a session and its descendants. Returns `(session_id, project)` for
    /// every row removed, children first, so the caller can tombstone each one
    /// and only remove their files once the transaction has committed.
    /// `(parent_session_id, parent_call_id)` of a child row; None for roots
    /// or rows without a call id.
    pub(crate) fn parent_link_on(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<(String, String)>> {
        let row: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT parent_session_id, parent_call_id FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((Some(parent), Some(call))) if !parent.is_empty() && !call.is_empty() => {
                Some((parent, call))
            }
            _ => None,
        })
    }

    /// WP-091 R7: add `{call_id: child_id}` to the parent's tombstone map.
    /// A parent row that is already gone is a no-op.
    pub(crate) fn record_deleted_child_on(
        conn: &Connection,
        parent_session_id: &str,
        call_id: &str,
        child_session_id: &str,
    ) -> Result<()> {
        let raw: Option<String> = conn
            .query_row(
                "SELECT deleted_children_json FROM sessions WHERE id = ?1",
                rusqlite::params![parent_session_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(());
        };
        let mut map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).unwrap_or_default();
        map.insert(call_id.to_string(), child_session_id.to_string());
        let encoded = serde_json::to_string(&map)
            .map_err(|e| crate::types::LitecodeError::SessionStorage(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET deleted_children_json = ?1 WHERE id = ?2",
            rusqlite::params![encoded, parent_session_id],
        )?;
        Ok(())
    }

    pub(crate) fn delete_on(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<(String, Option<String>)>> {
        let project: Option<String> = conn
            .query_row(
                "SELECT project FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        let child_ids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM sessions WHERE parent_session_id = ?1")?;
            let rows = stmt.query_map(rusqlite::params![session_id], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        let mut deleted = Vec::new();
        for child_id in &child_ids {
            deleted.extend(Self::delete_on(conn, child_id)?);
        }
        let tx = conn;
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
        tx.execute(
            "DELETE FROM session_context_meter WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        tx.execute(
            "DELETE FROM session_operations WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        super::ops::drop_blob_refs_session(&tx, session_id)?;
        deleted.push((session_id.to_string(), project));
        Ok(deleted)
    }

    pub(crate) fn cleanup_deleted_files(data_root: &Path, session_id: &str, project: Option<&str>) {
        let session_dir = data_root.join("sessions").join(session_id);
        if session_dir.exists()
            && let Err(error) = std::fs::remove_dir_all(&session_dir)
        {
            tracing::warn!(
                session_id,
                path = %session_dir.display(),
                %error,
                "cleanup_deleted_files: failed to remove session dir"
            );
        }
        if let Some(project) = project {
            let project_path = std::path::Path::new(project);
            let snaps = match crate::config::peek_workspace_id(project_path) {
                Some(id) => crate::session::snapshot_paths::snapshots_dir_for_id(&id),
                None => crate::session::snapshot_paths::snapshots_dir_for_workspace(project_path),
            };
            if let Err(error) = snapshot::delete_session_snapshots(&snaps, session_id) {
                tracing::warn!(
                    session_id,
                    %error,
                    "cleanup_deleted_files: failed to remove session snapshots"
                );
            }
        }
    }

    pub(crate) fn resume_shared(
        db: Rc<SharedDb>,
        data_root: PathBuf,
        session_id: &str,
    ) -> Result<Self> {
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
        ) = db
            .conn()
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
        let session = Self {
            db,
            id: session_id.to_string(),
            project,
            agent_id,
            model_id,
            thinking_tier,
            context_mode,
            parent_session_id,
            parent_call_id,
            data_root,
            db_path: None,
            ephemeral: false,
            persisted_max_seq: Cell::new(0),
            write_gate: Mutex::new(()),
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
        let (todos_json, active_plan_slug, plan_revision): (
            String,
            Option<String>,
            Option<String>,
        ) = self
            .conn()
            .query_row(
                "SELECT COALESCE(todos_json, '[]'), active_plan_slug, plan_revision FROM sessions WHERE id = ?1",
                rusqlite::params![self.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;

        let todos: Vec<TodoItem> = serde_json::from_str(&todos_json)
            .map_err(|e| LitecodeError::ToolExecution(format!("parse session todos: {e}")))?;

        let active_plan = active_plan_slug.map(|slug| PlanRef::with_revision(&slug, plan_revision));

        let mut state = TaskReminders { todos, active_plan };
        state.normalize();
        Ok(state)
    }

    pub fn save_task_state(&self, state: &TaskReminders) -> Result<()> {
        Self::write_task_state(self.conn(), &self.id, state)
    }

    /// Load last-known provider usage meter (empty if never written).
    pub fn load_context_meter(&self) -> Result<SessionContextMeter> {
        load_context_meter_on(self.conn(), &self.id)
    }

    /// Upsert last-known **provider** usage for this session.
    pub fn save_context_meter(&self, meter: &SessionContextMeter) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.conn()
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

    /// Fill `provider_usage` on one `request/header` row. A non-header seq, a
    /// missing row, or a column that already has a value changes nothing.
    /// Does not touch body, kind, or state.
    pub fn record_provider_usage(&self, seq: Seq, usage_json: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE transcript_items
             SET provider_usage = ?1
             WHERE session_id = ?2 AND seq = ?3
               AND event_type = 'request/header'
               AND provider_usage IS NULL",
            rusqlite::params![usage_json, self.id, seq as i64],
        )?;
        Ok(())
    }

    fn write_task_state(conn: &Connection, session_id: &str, state: &TaskReminders) -> Result<()> {
        let mut state = state.clone();
        state.normalize();

        let todos_json = serde_json::to_string(&state.todos)?;
        let (active_plan_slug, plan_revision) = state
            .active_plan
            .as_ref()
            .map(|p| (Some(p.slug.as_str()), p.revision.as_deref()))
            .unwrap_or((None, None));

        conn.execute(
            "UPDATE sessions SET todos_json = ?1, active_plan_slug = ?2, plan_revision = ?3 WHERE id = ?4",
            rusqlite::params![todos_json, active_plan_slug, plan_revision, session_id],
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
                if content.is_empty() {
                    continue;
                }
                return clip_preview(&content);
            }
        }
        String::new()
    }

    pub(crate) fn last_assistant_message_preview(items: &[Item]) -> String {
        for item in items.iter().rev() {
            let Item::Message(MessageItem::Output(message)) = item else {
                continue;
            };
            if message.role != AssistantRole::Assistant || message.status != OutputStatus::Completed
            {
                continue;
            }
            let content = item_text_preview(item);
            if content.is_empty() {
                continue;
            }
            return clip_preview(&content);
        }
        String::new()
    }
}

impl Session {
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
        let mut items_by_seq = HashMap::new();
        let mut reminders_by_seq = HashMap::new();
        let mut compacted_seqs = HashSet::new();
        let mut in_progress = HashSet::new();
        for event in &events {
            if event.state == LogState::InProgress {
                in_progress.insert(event.seq);
            }
            if node_set.contains(&event.seq) {
                match &event.event_type {
                    EventType::Reminder(_) => {
                        if let Ok(reminder) = serde_json::from_value(event.data.clone()) {
                            reminders_by_seq.insert(event.seq, reminder);
                        }
                    }
                    EventType::Compacted => {
                        compacted_seqs.insert(event.seq);
                    }
                    _ => {}
                }
            }
            let item = spine_agent_item(event).ok();
            if let Some(item) = item
                && node_set.contains(&event.seq)
            {
                items_by_seq.insert(event.seq, item);
            }
        }
        let active_max_seq = events.last().map(|e| e.seq as i64).unwrap_or(-1);
        // The persisted high-water survives truncate; heal it forward if the
        // table ever holds a seq at or past it (e.g. a row written by an older
        // writer, or a `next_seq` update that raced a crash).
        let persisted_next = self.load_persisted_next_seq()?;
        let next_seq = persisted_next.max(active_max_seq.saturating_add(1)).max(0) as Seq;
        let projection = LogProjection {
            surface,
            next_seq,
            active_max_seq,
            items_by_seq,
            reminders_by_seq,
            compacted_seqs,
            in_progress,
        };
        self.persisted_max_seq.set(projection.active_max_seq());
        *self.projection.borrow_mut() = projection;
        Ok(())
    }

    /// Persisted allocation high-water from `sessions.next_seq` (`0` if unset).
    fn load_persisted_next_seq(&self) -> Result<i64> {
        let v: Option<i64> = self
            .conn()
            .query_row(
                "SELECT next_seq FROM sessions WHERE id = ?1",
                rusqlite::params![self.id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0).max(0))
    }

    fn cached_max_seq(&self) -> i64 {
        self.projection.borrow().active_max_seq()
    }

    fn commit_projection(&self, projection: LogProjection) {
        self.persisted_max_seq.set(projection.active_max_seq());
        *self.projection.borrow_mut() = projection;
    }

    fn admit_draft_in_tx(
        &self,
        tx: &Connection,
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
        let (stored_item, cache_item, reminder_tokens) =
            if matches!(draft.event_type, EventType::Reminder(_)) {
                let reminder: crate::reminder::Reminder =
                    serde_json::from_value(draft.data.clone())?;
                let rendered = crate::reminder::render_item(&reminder);
                let tokens = crate::session::estimate::compute_token_estimate(std::slice::from_ref(
                    &rendered,
                )) as i64;
                (None, Some(rendered), tokens)
            } else if draft.event_type.is_item()
                || matches!(draft.event_type, EventType::PlanExecute)
            {
                let item = serde_json::from_value::<Item>(draft.data.clone())?;
                (Some(item.clone()), Some(item), 0)
            } else {
                (None, None, 0)
            };
        let token_estimate = if token_estimate > 0 {
            token_estimate
        } else {
            reminder_tokens
        };
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
            stored_item.as_ref(),
            &tid,
            turn_seq,
            kind,
            token_estimate,
        )?;
        // Advance the persisted allocation high-water in the same writer
        // transaction as the row it justified. `max` makes it monotonic: a
        // re-hydrated/behind cursor can never be pulled backwards here.
        tx.execute(
            "UPDATE sessions SET next_seq = max(next_seq, ?1) WHERE id = ?2",
            rusqlite::params![seq.saturating_add(1) as i64, self.id],
        )?;
        projection.apply_event(&event, cache_item.as_ref())?;
        Ok((seq, stored_item))
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
                Ok(ApplyOutcome::Sealed { seq })
            }
            SessionApply::Truncate { anchor_seq } => {
                let anchor = self.truncate_unlocked(anchor_seq)?;
                Ok(ApplyOutcome::Truncated { anchor })
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
        let tx = self.conn();
        let (seq, item) =
            self.admit_draft_in_tx(&tx, draft, turn_id, turn_seq, kind, 0, &mut projection)?;
        if let Some(item) = item.as_ref() {
            let _ = write_session_list_previews(&tx, &self.id, std::slice::from_ref(item))?;
        } else {
            let now = chrono::Utc::now().timestamp_millis();
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, self.id],
            )?;
        }
        self.commit_projection(projection);
        Ok(seq)
    }

    fn seal_unlocked(&self, seq: Seq, item: &Item) -> Result<()> {
        let tx = self.conn();
        seal_event_row(&tx, &self.id, &self.data_root, seq, item)?;
        self.projection.borrow_mut().seal_item(seq, item);
        let user = Self::last_user_message_preview(std::slice::from_ref(item));
        let assistant = Self::last_assistant_message_preview(std::slice::from_ref(item));
        if !user.is_empty() || !assistant.is_empty() {
            let _ = write_session_list_previews(&tx, &self.id, std::slice::from_ref(item))?;
        }
        Ok(())
    }

    fn truncate_unlocked(&self, anchor_seq: i64) -> Result<i64> {
        let tx = self.conn();
        require_user_anchor(tx, &self.id, anchor_seq)?;

        tx.execute(
            "DELETE FROM transcript_items WHERE session_id = ?1 AND seq >= ?2",
            rusqlite::params![self.id, anchor_seq],
        )?;
        super::ops::drop_blob_refs_ge(&tx, &self.id, anchor_seq)?;

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
        let assistant: String = tx
            .query_row(
                "SELECT body FROM transcript_items
                 WHERE session_id = ?1
                   AND kind = 'item/assistant'
                   AND body IS NOT NULL
                   AND state = 'final'
                 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![self.id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .and_then(|body| serde_json::from_str::<Item>(&body).ok())
            .map(|item| Self::last_assistant_message_preview(std::slice::from_ref(&item)))
            .unwrap_or_default();

        tx.execute(
            "UPDATE sessions
             SET updated_at = ?1,
                 last_message = ?2,
                 last_assistant = ?3
             WHERE id = ?4",
            rusqlite::params![now, preview, assistant, self.id],
        )?;
        refresh_compact_pointers_from_log(&tx, &self.id)?;

        // Remaining log is 0..anchor-1. A deleted replace must unshadow earlier
        // nodes; dropping `nodes >= anchor` from the live surface is not enough.
        self.hydrate_projection()?;
        Ok(anchor_seq)
    }

    fn bump_session_updated(
        &self,
        tx: &Connection,
        items: &[Item],
    ) -> Result<Option<SessionListPreview>> {
        write_session_list_previews(tx, &self.id, items)
    }

    /// §5.1 step INSERT — append `delta` detail rows with consecutive seq allocation.
    ///
    /// Returns `Some((preview, updated_at))` when this delta contains a user
    /// message and `last_message` was updated. Assistant/tool-only deltas no
    /// longer wipe `last_message`.
    pub fn insert_detail_rows(&self, delta: &[Item]) -> Result<InsertDetailsOutcome> {
        self.insert_detail_rows_with_turn(delta, "")
    }

    /// Append detail rows tagged with a real turn id.
    pub fn insert_detail_rows_with_turn(
        &self,
        delta: &[Item],
        turn_id: &str,
    ) -> Result<InsertDetailsOutcome> {
        if delta.is_empty() {
            return Ok(InsertDetailsOutcome {
                preview: None,
                seq_range: None,
            });
        }
        let _gate = self.lock_write();
        let mut projection = self.projection.borrow().clone();
        let tx = self.conn();
        let turn_id = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };
        let mut seq_range: Option<(Seq, Seq)> = None;
        for (i, msg) in delta.iter().enumerate() {
            let kind = surface_event_type_of(msg).as_str().to_owned();
            let mut draft =
                EventDraft::surface_item(surface_event_type_of(msg), msg, SurfaceOp::Append)?;
            draft.time = message_timestamp(msg);
            let (seq, _) =
                self.admit_draft_in_tx(&tx, draft, &turn_id, i as i64, &kind, 0, &mut projection)?;
            seq_range = Some(match seq_range {
                Some((first, last)) => (first.min(seq), last.max(seq)),
                None => (seq, seq),
            });
        }
        let preview_updated = self.bump_session_updated(&tx, delta)?;
        self.commit_projection(projection);
        Ok(InsertDetailsOutcome {
            preview: preview_updated
                .and_then(|patch| patch.user.map(|user| (user, patch.updated_at))),
            seq_range,
        })
    }

    /// Append one settled Item and return its `seq`.
    ///
    /// Appending states that the item is complete as handed over. A caller that
    /// is streaming must open the row with [`Self::begin_stream_item`] first and
    /// settle it with [`Self::seal_stream_item`], so that the item's identity and
    /// its lifecycle are both explicit.
    pub fn persist_item(&self, item: &Item) -> Result<Seq> {
        let _gate = self.lock_write();
        let mut draft =
            EventDraft::surface_item(surface_event_type_of(item), item, SurfaceOp::Append)?;
        draft.time = message_timestamp(item);
        let kind = draft.event_type.as_str().to_owned();
        self.append_unlocked(draft, "", 0, &kind)
    }

    /// Open the row for a streaming item and return its `seq`.
    ///
    /// The row is `in_progress` because this command says so — never because the
    /// payload happened to carry `status: in_progress`.
    ///
    /// Nothing here checks whether the provider has used this `item_id` before.
    /// A provider id is the caller's association key for **one** model call, not a
    /// log identity: dialects are free to reuse an id in a later call, and that is
    /// a different item which must get its own row. "One item, one row" is
    /// enforced where the association actually exists — inside a single stream
    /// projection, which refuses to open the same id twice.
    pub fn begin_stream_item(&self, item: &Item, turn_id: &str) -> Result<Seq> {
        if turn_id.is_empty() {
            return Err(LitecodeError::InvalidSessionEvent(
                "begin_stream_item: a streaming row needs a real turn id".into(),
            ));
        }
        let _gate = self.lock_write();
        let mut draft = EventDraft::stream_item(
            surface_event_type_of(item),
            item,
            SurfaceOp::Append,
            LogState::InProgress,
        )?;
        draft.time = message_timestamp(item);
        let kind = draft.event_type.as_str().to_owned();
        self.append_unlocked(draft, turn_id, 0, &kind)
    }

    /// Replace the payload of a row that is still in flight.
    ///
    /// Late content for an already-settled row is refused rather than written:
    /// the derived index watched the row settle and carries no way to notice it
    /// changing afterwards.
    pub fn update_stream_item(&self, seq: Seq, item: &Item) -> Result<()> {
        let _gate = self.lock_write();
        let tx = self.conn();
        update_event_row(&tx, &self.id, &self.data_root, seq, item)?;
        self.projection.borrow_mut().touch_item(seq, item);
        self.bump_stream_preview(&tx, item)
    }

    /// Settle a row opened by [`Self::begin_stream_item`].
    ///
    /// The payload must be the **authoritative** one: the copy the terminal
    /// response carries and that a later replay will send back upstream. Sealing
    /// with an intermediate copy is what leaves a settled row whose content no
    /// longer matches the transcript's own replay, and the mismatch then has to
    /// be absorbed somewhere downstream.
    pub fn seal_stream_item(&self, seq: Seq, item: &Item) -> Result<()> {
        let _gate = self.lock_write();
        let tx = self.conn();
        seal_event_row(&tx, &self.id, &self.data_root, seq, item)?;
        self.projection.borrow_mut().seal_item(seq, item);
        self.bump_stream_preview(&tx, item)
    }

    /// Session-list preview and `updated_at` for a streaming row that just moved.
    fn bump_stream_preview(&self, tx: &Connection, item: &Item) -> Result<()> {
        let user = Self::last_user_message_preview(std::slice::from_ref(item));
        let assistant = Self::last_assistant_message_preview(std::slice::from_ref(item));
        if !user.is_empty() || !assistant.is_empty() {
            let _ = write_session_list_previews(tx, &self.id, std::slice::from_ref(item))?;
        } else {
            let now = chrono::Utc::now().timestamp_millis();
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, self.id],
            )?;
        }
        Ok(())
    }

    /// Append one reminder. The row owns a new seq; the body is the reminder itself.
    pub fn append_reminder(&self, reminder: &crate::reminder::Reminder) -> Result<Seq> {
        let mut draft = EventDraft::reminder(reminder)?;
        draft.time = chrono::Utc::now().timestamp_millis();
        match self.apply(SessionApply::Append(draft))? {
            ApplyOutcome::Appended(seq) => Ok(seq),
            _ => unreachable!("append operation must append"),
        }
    }

    /// Diff/Restore baseline from the live projection.
    pub fn spine_reminder_view(&self) -> crate::reminder::SpineReminderView {
        let projection = self.projection.borrow();
        let nodes = projection
            .surface
            .nodes
            .iter()
            .map(|seq| {
                let node = if projection.compacted_seqs.contains(seq) {
                    crate::reminder::SpineNode::Compacted
                } else if let Some(reminder) = projection.reminders_by_seq.get(seq) {
                    crate::reminder::SpineNode::Reminder(reminder.clone())
                } else {
                    crate::reminder::SpineNode::Other
                };
                (*seq, node)
            })
            .collect::<Vec<_>>();
        crate::reminder::view_from_spine(&nodes)
    }

    /// Append the plan-execution trigger message with kind `plan/execute`.
    ///
    /// It carries a user `Item` body (so AgentView sees it as a user turn), but
    /// its kind is its own: it is system-issued on the human's behalf and is
    /// deliberately **not** a revert anchor (anchors only count `item/user`).
    pub fn append_plan_execute(&self, item: &Item) -> Result<Seq> {
        let mut draft = EventDraft::surface_item(EventType::PlanExecute, item, SurfaceOp::Append)?;
        draft.time = message_timestamp(item);
        match self.apply(SessionApply::Append(draft))? {
            ApplyOutcome::Appended(seq) => Ok(seq),
            _ => unreachable!("append operation must append"),
        }
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
                Item::FunctionCall(fc) => {
                    // WP-186: permission-class waits must become Final so
                    // seal_tail can run, but must NOT be stamped Incomplete
                    // (they are intentional unsealed tails, not crash orphans).
                    let permission_class = super::pending::should_skip_seal_for_call(
                        self.conn(),
                        &self.id,
                        &fc.call_id,
                        &fc.name,
                    )
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            error = %e,
                            call_id = %fc.call_id,
                            "pending seal-skip lookup failed during seal_in_progress"
                        );
                        crate::session::pending::PendingPermissionRecord::is_permission_class_tool_name(
                            &fc.name,
                        )
                    });
                    if permission_class {
                        if let Err(e) = super::pending::backfill_call_id(
                            self.conn(),
                            &self.id,
                            &fc.name,
                            &fc.call_id,
                        ) {
                            tracing::warn!(
                                error = %e,
                                call_id = %fc.call_id,
                                tool = %fc.name,
                                "failed to backfill pending_permissions.call_id"
                            );
                        }
                        // leave status as-is (no Incomplete)
                    } else {
                        fc.status = Some(OutputStatus::Incomplete);
                    }
                }
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

        let tx = self.conn();
        let mut projection = self.projection.borrow().clone();
        let tid = if turn_id.is_empty() {
            format!("orphan-{}", ulid::Ulid::new())
        } else {
            turn_id.to_string()
        };
        let mut mutated = false;
        let mut sealed_seqs = Vec::new();
        let mut appended_seqs = Vec::new();
        let mut sealed_items = Vec::new();
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
                if same {
                    // The bytes are already what the turn is committing. If the row
                    // settled, this is the idempotent case and nothing is written.
                    // If it is still in flight, these same bytes are still its
                    // seal: a streamed row must not survive its own commit as
                    // `in_progress`.
                    if row_is_settled(&tx, &self.id, seq)? {
                        continue;
                    }
                    seal_event_row(&tx, &self.id, &self.data_root, seq, msg)?;
                    projection.seal_item(seq, msg);
                    sealed_seqs.push(seq);
                    sealed_items.push(msg.clone());
                    mutated = true;
                    continue;
                }
                // Different content at a seq this row already owns. If the row
                // is still in flight it is sealed in place, as always. If it has
                // already settled it may not be rewritten (see `seal_event_row`),
                // and the text still has to land — so it becomes a new row.
                //
                // A settled row is immutable. The remaining text becomes a new
                // continuation row; there is no provider-id lookup in Session.
                if row_is_settled(&tx, &self.id, seq)? {
                    row.log_seq = None;
                } else {
                    seal_event_row(&tx, &self.id, &self.data_root, seq, msg)?;
                    projection.seal_item(seq, msg);
                    sealed_seqs.push(seq);
                    sealed_items.push(msg.clone());
                    mutated = true;
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
            appended_seqs.push(seq);
            appended.push(msg.clone());
        }

        let mut preview_items = appended.clone();
        preview_items.extend(sealed_items);
        let preview_updated = self.bump_session_updated(&tx, &preview_items)?;
        *rows = kept;
        self.commit_projection(projection);
        Ok(CommitDeltaOutcome::Applied {
            preview: preview_updated,
            sealed_seqs,
            appended_seqs,
            mutated,
        })
    }

    /// User-message anchors around `anchor_seq` (`None` = latest `item/user`).
    pub fn user_anchor_window(
        &self,
        anchor_seq: Option<i64>,
        before: i64,
        after: i64,
    ) -> Result<UserAnchorWindow> {
        user_anchor_window_on(self.conn(), &self.id, anchor_seq, before, after)
    }

    /// Persisted `item/user` row count.
    pub fn user_detail_count(&self) -> Result<i64> {
        self.conn()
            .query_row(SQL_USER_DETAIL_COUNT, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    /// Snapshot file stem for a user row: `next_seq` at turn start.
    ///
    /// Track runs once that user row is the last allocated seq, so the stem is
    /// `seq + 1`.
    pub fn snapshot_stem_for_user_seq(&self, seq: i64) -> Result<i64> {
        require_user_anchor(self.conn(), &self.id, seq)?;
        seq.checked_add(1)
            .ok_or_else(|| LitecodeError::InvalidRevertAnchor(format!("seq={seq} overflow")))
    }

    /// Truncate the log from this user row's `seq`. No file side effects.
    pub fn revert_to_user_anchor(&self, seq: i64) -> Result<()> {
        self.apply(SessionApply::Truncate { anchor_seq: seq })?;
        Ok(())
    }

    pub fn truncated_tool_result(content: &str, max_tokens: usize) -> String {
        let byte_limit = max_tokens.saturating_mul(4);
        if byte_limit >= content.len() {
            return content.to_string();
        }
        let preview = crate::types::utf8::prefix_utf8(content, byte_limit);
        format!("{preview}\n... [truncated]")
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

    pub fn should_snip(item: &Item) -> bool {
        skip_empty_assistant_item(item)
    }

    /// Synthesize `FunctionCallOutput`s for `FunctionCall`s that lack a matching
    /// output (crash between persist and tool completion). Used on the **ephemeral
    /// LLM view**.
    ///
    /// When the unanswered calls are still the tail of the log, startup persists
    /// the same output via [`Self::seal_tail_unanswered_calls`]. Calls already
    /// followed by later history stay unpadded on disk — an append-only log
    /// cannot put the result back beside them — and this view still supplies
    /// the pair. Callers must not persist this view.
    ///
    /// Returns the number of outputs appended.
    pub fn pad_unanswered_calls(transcript: &mut Transcript) -> usize {
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
                    result.push(Self::unanswered_call_output(&call_id, &name));
                    *padded += 1;
                }
            };

        for item in transcript.iter() {
            match item {
                Item::FunctionCall(fc) => {
                    // An answered FunctionCall starts a later batch. Flush hanging
                    // pads first so a recovered call keeps its pad in place — otherwise
                    // the ephemeral pad moves and busts the prompt-cache prefix.
                    if answered.contains(&fc.call_id) {
                        flush(&mut pending, &mut result, &mut padded);
                        result.push(item.clone());
                    } else {
                        result.push(item.clone());
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

    /// The abort seal for a tool call that never recorded a result.
    ///
    /// Same text the ephemeral view pads with, so a call closed here and a call
    /// the view still has to pad say one thing.
    fn unanswered_call_output(call_id: &str, name: &str) -> Item {
        use crate::authority::responses::{FunctionCallOutput, FunctionCallOutputItemParam};

        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.to_string(),
            output: FunctionCallOutput::Text(format!(
                "tool '{name}' was interrupted: no result was recorded \
                 (session recovered before completion)"
            )),
            id: None,
            status: None,
        })
    }

    /// Persist interrupt results for unanswered calls that are still the log tail.
    ///
    /// Append-only: the results are new rows after the calls, and the calls
    /// themselves are not rewritten. A session that still has an `in_progress`
    /// row is left untouched — startup seals those first, and a session that
    /// could not be sealed is retried on the next open. Returns the new seqs.
    pub(crate) fn seal_tail_unanswered_calls(&self) -> Result<Vec<Seq>> {
        let in_flight: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE session_id = ?1 AND state = 'in_progress'",
            rusqlite::params![self.id],
            |row| row.get(0),
        )?;
        if in_flight > 0 {
            return Ok(Vec::new());
        }

        let rows = self.load_history_transcript()?;
        let mut answered = HashSet::new();
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let item = row_to_item(row, &self.data_root).ok();
            if let Some(Item::FunctionCallOutput(output)) = &item {
                answered.insert(output.call_id.clone());
            }
            items.push(item);
        }

        let mut suffix = Vec::new();
        for index in (0..rows.len()).rev() {
            match &items[index] {
                Some(Item::FunctionCall(call))
                    if !call.call_id.is_empty() && !answered.contains(&call.call_id) =>
                {
                    suffix.push(index);
                }
                _ => break,
            }
        }
        if suffix.is_empty() {
            return Ok(Vec::new());
        }
        suffix.reverse();

        let mut sealed = Vec::new();
        let mut start = 0;
        while start < suffix.len() {
            let turn_id = rows[suffix[start]].turn_id.clone();
            let mut end = start + 1;
            while end < suffix.len() && rows[suffix[end]].turn_id == turn_id {
                end += 1;
            }
            let outputs: Vec<Item> = suffix[start..end]
                .iter()
                .filter_map(|&index| match &items[index] {
                    Some(Item::FunctionCall(call)) => {
                        // WP-186: permission-class tails stay unsealed (restore waiting).
                        let skip = super::pending::should_skip_seal_for_call(
                            self.conn(),
                            &self.id,
                            &call.call_id,
                            &call.name,
                        )
                        .unwrap_or_else(|e| {
                            tracing::warn!(
                                error = %e,
                                call_id = %call.call_id,
                                "pending seal-skip lookup failed; sealing as interrupt"
                            );
                            false
                        });
                        if skip {
                            tracing::info!(
                                session_id = %self.id,
                                call_id = %call.call_id,
                                tool = %call.name,
                                "skip seal_tail for permission-class unanswered call"
                            );
                            // R4: name-fallback synthesize sidecar when absent.
                            if let Err(e) = super::pending::ensure_name_fallback_pending(
                                self.conn(),
                                &self.id,
                                &turn_id,
                                &call.call_id,
                                &call.name,
                            ) {
                                tracing::warn!(
                                    error = %e,
                                    call_id = %call.call_id,
                                    "failed to synthesize pending_permissions fallback"
                                );
                            }
                            None
                        } else {
                            Some(Self::unanswered_call_output(&call.call_id, &call.name))
                        }
                    }
                    _ => None,
                })
                .collect();
            if !outputs.is_empty() {
                let outcome = self.insert_detail_rows_with_turn(&outputs, &turn_id)?;
                if let Some((first, last)) = outcome.seq_range {
                    sealed.extend(first..=last);
                }
            }
            start = end;
        }
        Ok(sealed)
    }

    pub fn set_agent_id(&mut self, agent_id: &str) -> Result<()> {
        self.agent_id = agent_id.to_string();
        self.conn().execute(
            "UPDATE sessions SET agent_id = ?1 WHERE id = ?2",
            rusqlite::params![agent_id, self.id],
        )?;
        Ok(())
    }

    pub fn set_model_id(&mut self, model_id: Option<&str>) -> Result<()> {
        let model_id_owned = normalize_model_id(model_id);
        self.model_id = model_id_owned.clone();
        self.conn().execute(
            "UPDATE sessions SET model_id = ?1 WHERE id = ?2",
            rusqlite::params![model_id_owned, self.id],
        )?;
        Ok(())
    }

    pub fn set_thinking_tier(&mut self, tier: ThinkingTier) -> Result<()> {
        self.thinking_tier = tier;
        self.conn().execute(
            "UPDATE sessions SET thinking_tier = ?1 WHERE id = ?2",
            rusqlite::params![tier.as_str(), self.id],
        )?;
        Ok(())
    }

    pub fn set_context_mode(&mut self, mode: ContextMode) -> Result<()> {
        self.context_mode = mode;
        self.conn().execute(
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
        self.conn()
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
        self.conn()
            .query_row(SQL_KEPT_FROM_SEQ, rusqlite::params![self.id], |row| {
                row.get(0)
            })
            .map_err(Into::into)
    }

    pub fn max_seq(&self) -> Result<i64> {
        self.conn()
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) FROM transcript_items WHERE session_id = ?1",
                rusqlite::params![self.id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// In-memory active `MAX(seq)` cursor (the log's current tail, `-1` when
    /// empty). Distinct from the persisted allocation high-water `sessions.next_seq`.
    pub fn persisted_max_seq(&self) -> i64 {
        self.persisted_max_seq.get()
    }

    /// Reload the active-tail cursor from the DB's current `MAX(seq)` (turn load,
    /// post-compact). Never touches the allocation high-water.
    pub fn reload_persisted_max_seq(&self) -> Result<()> {
        self.persisted_max_seq.set(self.cached_max_seq());
        Ok(())
    }

    /// §5.1 turn load — pi view: current compact summary + original detail from
    /// `kept_from_seq` onward (no mid-log splice / kept rewrite).
    pub fn load_turn_transcript(&self) -> Result<Vec<TranscriptRow>> {
        let mut stmt = self.conn().prepare(SQL_LOAD_TURN_TRANSCRIPT)?;
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
        let mut stmt = self.conn().prepare(SQL_LOAD_HISTORY_TRANSCRIPT)?;
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
        load_events_on(self.conn(), &self.id, &self.data_root)
    }

    /// Half-open log window `[from_seq, to_seq)` in append-origin seq order.
    pub fn load_events_range(&self, from_seq: Seq, to_seq: Seq) -> Result<Vec<SessionEvent>> {
        load_events_range_on(
            self.conn(),
            &self.id,
            from_seq as i64,
            to_seq as i64,
            &self.data_root,
        )
    }

    /// `(last_seq, next_seq)` for wire snapshots. Empty log → `(-1, 0)`.
    ///
    /// **Split semantics.** `last_seq` is the **active** `MAX(seq)` of the live
    /// rows; `next_seq` is the **persisted allocation high-water**
    /// (`sessions.next_seq`). A revert moves `last_seq` back but not `next_seq`,
    /// so the two are not tied by `next_seq == last_seq + 1`. The client keeps
    /// rows `<= last_seq` and fetches `[last_seq + 1, next_seq)`; the next append
    /// lands on the high-water, so that window refills exactly the new rows while
    /// older holes stay closed. See [`load_wire_seq_cursor_on`].
    pub fn wire_seq_cursor(&self) -> Result<(i64, u64)> {
        load_wire_seq_cursor_on(self.conn(), &self.id)
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
        let tx = self.conn();
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
        self.commit_projection(projection);
        Ok(seq as i64)
    }
}

#[cfg(test)]
mod tests {
    
    use super::*;
    use crate::authority::responses::{
        AssistantRole,
        MessageItem, OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
    };
    use crate::types::user_text;

    fn session_compact_pointers(session: &Session) -> (i64, Option<i64>, i64, i64) {
        session
            .conn()
            .query_row(
                "SELECT checkpoint_seq, compacted_seq, kept_from_seq, spine_from FROM sessions WHERE id = ?1",
                rusqlite::params![session.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
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
                .conn()
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
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE session_id = ?1 AND kind = 'item/user'",
                rusqlite::params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(detail_before, 3);

        let keep_tail_seq: i64 = session
            .conn()
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
            .conn()
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
            .conn()
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
    fn revert_retaining_earlier_compact_restores_pointers() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let keep_first: i64 = session
            .conn()
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
            .conn()
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
        // Users a,b,c then a compact, then d. Truncate from d's seq.
        session.revert_to_user_anchor(keep_second).unwrap();
        let (checkpoint_seq, compacted_seq, kept_from_seq, spine_from) =
            session_compact_pointers(&session);
        assert_eq!(checkpoint_seq, first);
        assert_eq!(compacted_seq, Some(first));
        assert_eq!(kept_from_seq, keep_first);
        assert_eq!(spine_from, first);
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
        let seq = session.begin_stream_item(&live, "t1").unwrap();
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

    /// A process that died mid-turn cannot seal what it left in flight, so the
    /// next start seals it as `Incomplete`. The row becomes settled history at
    /// that moment and is never rewritten again — the turn's remaining text
    /// arrives as a continuation row instead.
    ///
    /// This is the shape the earlier design could not express: back then the
    /// orphan was kept in flight so that the finished text could replace the
    /// fragment in place. That worked for the transcript and quietly broke
    /// search, because the derived index has no way to notice a row whose body
    /// changed after it was projected.
    #[test]
    fn a_crashed_turn_is_sealed_incomplete_and_its_continuation_appends() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let lease = crate::session::WorkspaceWriteLease::acquire(dir.path()).unwrap();
        let data = crate::session::SessionData::open(&lease, &db).unwrap();
        let id = data
            .create_session("/tmp/proj", "default", Some("model"))
            .unwrap();
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
        let receipt = data
            .mutate_blocking(crate::session::SessionMutation::BeginStreamItem {
                session_id: id.clone(),
                expected_revision: data.revision_blocking(&id).unwrap(),
                operation_id: crate::session::MutationId::new(),
                item: live,
                turn_id: "t-crash".into(),
            })
            .unwrap();
        let seq = match receipt.outcome {
            crate::session::data::CommitKind::Appended { seq } => seq,
            other => panic!("expected appended receipt, got {other:?}"),
        };
        drop(data);

        let data = crate::session::SessionData::open(&lease, &db).unwrap();
        let events = data.events_blocking(&id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, seq);
        assert_eq!(
            events[0].state,
            LogState::Final,
            "the restart sealed the orphan it found in flight"
        );
        let loaded = crate::session::event::item_from_event(&events[0]).unwrap();
        match &loaded {
            Item::Message(MessageItem::Output(m)) => {
                assert_eq!(
                    m.status,
                    OutputStatus::Incomplete,
                    "sealed as short of content, not as complete"
                );
            }
            other => panic!("expected sealed assistant, got {other:?}"),
        }
        assert_eq!(item_text_preview(&loaded), "hel", "what did arrive is kept");

        // The turn is finished afterwards. The row it wants to finish is settled,
        // so the text lands as a new row and seq 0 is left exactly as it was.
        let finished = Item::Message(MessageItem::Output(OutputMessage {
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
        let seal_receipt = data
            .mutate_blocking(crate::session::SessionMutation::CommitTurnDelta {
                session_id: id.clone(),
                expected_revision: data.revision_blocking(&id).unwrap(),
                operation_id: crate::session::MutationId::new(),
                rows: vec![crate::session::WorkingRow::persisted(seq, finished)],
                expected_max_seq: seq as i64,
                turn_id: "crash-recovery".into(),
            })
            .unwrap();
        // No in-place seal happened: the continuation was appended, and the
        // settled fragment at seq 0 is left exactly as it was.
        assert!(
            matches!(
                seal_receipt.outcome,
                crate::session::data::CommitKind::MetaUpdated
            ),
            "nothing was sealed in place, got {:?}",
            seal_receipt.outcome
        );

        let events = data.events_blocking(&id).unwrap();
        assert_eq!(events.len(), 2, "the fragment and the continuation");
        let first = crate::session::event::item_from_event(&events[0]).unwrap();
        assert_eq!(
            item_text_preview(&first),
            "hel",
            "seq {seq} is settled: the index has already projected it and nothing may change it"
        );
        let second = crate::session::event::item_from_event(&events[1]).unwrap();
        assert_eq!(item_text_preview(&second), "hello");
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
    fn truncate_then_append_does_not_reuse_deleted_seqs() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
        // Delete u1 (seq 1) and u2 (seq 2); u0 (seq 0) remains.
        session.revert_to_user_anchor(1).unwrap();
        assert_eq!(
            session.cached_max_seq(),
            0,
            "the active tail drops to seq 0 after the revert"
        );

        // The next append must allocate from the persisted high-water, not from
        // `active MAX + 1`, so the deleted seqs 1 and 2 are never handed out again.
        let outcome = session.insert_detail_rows(&[user_text("fresh")]).unwrap();
        assert_eq!(
            outcome.seq_range,
            Some((3, 3)),
            "append must use the high-water (3), not reuse the deleted seq 1"
        );
        let events = session.load_events().unwrap();
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 3],
            "the log keeps a hole where the revert deleted rows"
        );
        assert_eq!(session.cached_max_seq(), 3);
        let fresh = crate::session::event::item_from_event(events.last().unwrap()).unwrap();
        assert_eq!(item_text_preview(&fresh), "fresh");
        let folded = fold_surface(&events).unwrap().nodes;
        assert_eq!(session.projection.borrow().surface.nodes, folded);
    }

    #[test]
    fn reopen_after_truncate_keeps_high_water_and_appends_into_the_hole() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("u0"), user_text("u1"), user_text("u2")])
            .unwrap();
        session.revert_to_user_anchor(1).unwrap();
        // Re-hydrate from disk: the persisted high-water (3) must survive a
        // revert, and a fresh append must not collapse back onto seq 1.
        session.hydrate_projection().unwrap();
        assert_eq!(
            session.cached_max_seq(),
            0,
            "active tail is the surviving row"
        );
        let outcome = session
            .insert_detail_rows(&[user_text("after-reopen")])
            .unwrap();
        assert_eq!(outcome.seq_range, Some((3, 3)));
        // Full-load hydrate must tolerate the hole [0, 3] without erroring.
        let seqs: Vec<u64> = session
            .load_events()
            .unwrap()
            .iter()
            .map(|e| e.seq)
            .collect();
        assert_eq!(seqs, vec![0, 3]);
    }

    #[test]
    fn writer_working_set_matches_fold_after_append_and_keep_recent_compact() {
        let session = Session::ephemeral("/tmp/proj", "default", Some("model")).unwrap();
        session
            .insert_detail_rows(&[user_text("a"), user_text("b"), user_text("c")])
            .unwrap();
        let fold_ids = |session: &Session| {
            let events = session.load_events().unwrap();
            let surface = fold_surface(&events).unwrap();
            project_working_pairs(&surface, |seq| {
                events
                    .iter()
                    .find(|e| e.seq == seq)
                    .ok_or_else(|| {
                        LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing"))
                    })
                    .and_then(spine_agent_item)
            })
            .unwrap()
            .into_iter()
            .map(|(seq, item)| (seq, item_text_preview(&item)))
            .collect::<Vec<_>>()
        };
        let writer_ids = |session: &Session| {
            session
                .load_working_set()
                .unwrap()
                .into_iter()
                .map(|row| {
                    (
                        row.log_seq.expect("writer working row has seq"),
                        item_text_preview(&row.item),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(writer_ids(&session), fold_ids(&session));
        session
            .apply_compact_checkpoint_from(&user_text("sum"), Some(1), 10)
            .unwrap();
        let writer = writer_ids(&session);
        let folded = fold_ids(&session);
        assert_eq!(writer, folded);
        assert_eq!(writer[0].1, "sum");
        assert!(writer.iter().any(|(seq, text)| *seq == 1 && text == "b"));
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

}
