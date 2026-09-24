//! Projecting a provider's output stream into the session log.
//!
//! # Why this exists
//!
//! A model call is handed its output twice: once as a stream, so the human can
//! watch it arrive, and once as the call's return value, which is what the next
//! request replays. Those are the *same* items. Giving them two identities in the
//! log is what produces a transcript where an item's own settled copy lands after
//! the text that came after it — the reader then sees reasoning appear once the
//! turn is over, in the wrong place, and a reload cannot fix it because the wrong
//! order is what was written down.
//!
//! So a streamed item gets its `seq` once, when it starts, and every later write
//! addresses that same `seq`:
//!
//! ```text
//! output_item.added  → begin     one row, in flight, real turn id
//! delta              → update    while it arrives, coalesced
//! segment *.done     → update    content so far, still in flight
//! call returns       → seal      with the call's own copy of the item
//! ```
//!
//! # One call owns the identity map
//!
//! A projection instance lives for exactly one `complete_with_stream_events`, so
//! the provider's `item_id`/`output_index` association is scoped to that call and
//! never leaks into the log. The log's identity is `seq`; a provider that reuses
//! an id in a later call gets a new row there, because this map no longer exists
//! by then.
//!
//! # What the reader sees is what the log holds
//!
//! Content reaches the client only after it is written: a delta updates the row
//! and the row is then published. There is no second, in-memory-only body, so a
//! reconnect or a hard refresh reconstructs exactly what was on screen. Deltas
//! are coalesced on a short interval so a fast stream costs a bounded number of
//! writes rather than one transaction per token — and the coalescing is invisible
//! to the reader, because a coalesced delta is simply one that has not been
//! published yet.
//!
//! # The seal is the call's copy, not the stream's
//!
//! Sealing uses the item as the call returned it, never the mid-stream snapshot.
//! Providers may re-issue content between the two — an OpenAI reasoning item's
//! `encrypted_content` is re-encoded by the terminal payload — so the streamed
//! copy is not what a later replay sends upstream. Sealing with it would leave a
//! settled row whose content no longer equals the transcript's own replay, and
//! the difference would have to be absorbed somewhere downstream. Sealing with
//! the returned copy also means the row the stream opened is byte-identical to
//! the row the turn commits, so committing finds it and settles nothing twice.
//!
//! The stream therefore never appends: a completed call cannot add a second row
//! for an item it already streamed.
//!
//! # A write that fails is not a detail
//!
//! The first failure to open, update or settle a row is kept and returned by
//! [`StreamProjection::settle`], so the call fails instead of leaving the log
//! disagreeing with what the model produced. Swallowing it would put a row in
//! flight that nothing is left to settle.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::authority::responses::Item;
use crate::llm::{StreamItemAccumulator, item_id_of, mark_items_incomplete};
use crate::runtime::observer::{InternalEvent, RuntimeObserver};
use crate::session::{Seq, SessionManager};
use crate::types::{LitecodeError, Result, StreamEvents};

/// How long content may sit un-published before the next event flushes it.
///
/// Small enough that text still reads as streaming, large enough that a fast
/// provider costs a bounded number of writes per second instead of one per token.
pub(crate) const FLUSH_INTERVAL: Duration = Duration::from_millis(80);

/// One model call's stream, projected onto the session's rows.
pub(crate) struct StreamProjection {
    sessions: Arc<SessionManager>,
    observer: Arc<dyn RuntimeObserver>,
    session_id: String,
    turn_id: String,
    state: Mutex<ProjectionState>,
}

/// The `seq` a streamed item owns for the rest of its life.
struct OpenRow {
    seq: Seq,
    output_index: u32,
    item_type: String,
}

#[derive(Default)]
struct ProjectionState {
    /// Provider item id → the row opened for it. Scoped to this call.
    open: HashMap<String, OpenRow>,
    /// Provider output index → item id. The reverse map makes the per-call
    /// one-to-one association enforceable rather than merely documented.
    by_index: HashMap<u32, String>,
    /// Provider item id → the payload received so far.
    seen: StreamItemAccumulator,
    /// Items whose own `output_item.done` arrived. That payload is the item's
    /// terminal, so a missing batch terminal must not downgrade it.
    settled_by_provider: HashSet<String>,
    /// Open items holding content that has not been written yet.
    dirty: HashSet<String>,
    last_flush: Option<Instant>,
    /// Whether a request terminal already supplied the authoritative ordered list.
    terminal_observed: bool,
    /// First begin/update/seal failure. Kept, not logged and forgotten.
    failed: Option<LitecodeError>,
}

/// Shared handle: the stream callback folds events in, the caller settles after.
pub(crate) type SharedStreamProjection = Arc<StreamProjection>;

impl StreamProjection {
    pub(crate) fn new(
        sessions: Arc<SessionManager>,
        observer: Arc<dyn RuntimeObserver>,
        session_id: String,
        turn_id: String,
    ) -> SharedStreamProjection {
        Arc::new(Self {
            sessions,
            observer,
            session_id,
            turn_id,
            state: Mutex::new(ProjectionState::default()),
        })
    }

    /// Fold one stream event into the log.
    ///
    /// The stream callback cannot return an error, so the first failure is kept
    /// and handed back by [`Self::settle`]. Every later event is ignored: once a
    /// row could not be written, continuing would only widen the disagreement.
    pub(crate) fn observe(&self, event: &StreamEvents) {
        let mut state = self.lock();
        if state.failed.is_some() {
            return;
        }
        state.seen.observe(event);

        match event {
            StreamEvents::ResponseOutputItemAdded(added) => {
                let item = Item::from(added.item.clone());
                let Some(id) = item_id_of(&item) else {
                    return;
                };
                self.ensure_open(&mut state, &id, &item, added.output_index);
            }
            StreamEvents::ResponseOutputItemDone(ev) => {
                let item = Item::from(ev.item.clone());
                if let Some(id) = item_id_of(&item)
                    && self.ensure_open(&mut state, &id, &item, ev.output_index)
                {
                    state.settled_by_provider.insert(id.clone());
                    self.checkpoint(&mut state, &id, ev.output_index);
                }
            }
            // A segment the provider itself declared finished. The row stays in
            // flight — only the call's own copy settles it — but the content
            // lands now, so a process that dies here keeps what it had received.
            StreamEvents::ResponseOutputTextDone(ev) => {
                self.checkpoint(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseReasoningTextDone(ev) => {
                self.checkpoint(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseReasoningSummaryTextDone(ev) => {
                self.checkpoint(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseReasoningSummaryPartDone(ev) => {
                self.checkpoint(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseReasoningSummaryPartAdded(ev) => {
                self.touch(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseFunctionCallArgumentsDone(ev) => {
                self.checkpoint(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseOutputTextDelta(ev) => {
                self.touch(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseReasoningTextDelta(ev) => {
                self.touch(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseReasoningSummaryTextDelta(ev) => {
                self.touch(&mut state, &ev.item_id, ev.output_index)
            }
            StreamEvents::ResponseFunctionCallArgumentsDelta(ev) => {
                self.touch(&mut state, &ev.item_id, ev.output_index)
            }
            // The batch terminal names the items the call is returning. An item
            // the stream never announced still has a place in the log, and that
            // place is its `output_index` — so it is opened now rather than left
            // for the commit to append after everything that followed it.
            StreamEvents::ResponseCompleted(ev) => {
                let items: Vec<Item> = ev.response.output.iter().cloned().map(Item::from).collect();
                state.terminal_observed = true;
                self.adopt_terminal(&mut state, &items)
            }
            StreamEvents::ResponseIncomplete(ev) => {
                let items: Vec<Item> = ev.response.output.iter().cloned().map(Item::from).collect();
                state.terminal_observed = true;
                self.adopt_terminal(&mut state, &items)
            }
            _ => {}
        }
    }

    /// Settle every row this call opened.
    ///
    /// `returned` is the call's authoritative item list: `Some` when the call
    /// produced one (completed/incomplete output, or the partial items attached
    /// to an interrupted stream), `None` when it did not. With `Some`, an item
    /// the stream opened but the call did not return is a contract breach — the
    /// log would keep a row the transcript never replays. With `None` there is
    /// nothing authoritative to compare against, so every open row keeps the
    /// content it received and settles as incomplete.
    pub(crate) fn settle(&self, returned: Option<&[Item]>) -> Result<()> {
        let mut state = self.lock();
        if state.failed.is_none()
            && !state.terminal_observed
            && let Some(items) = returned
        {
            self.adopt_terminal(&mut state, items);
        }
        let observed = state.failed.take();
        let outcome = self.settle_open(&mut state, returned);
        match observed.or_else(|| state.failed.take()) {
            Some(first) => Err(first),
            None => outcome,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ProjectionState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fail(&self, state: &mut ProjectionState, error: LitecodeError) {
        if state.failed.is_none() {
            tracing::error!(
                error = %error,
                session_id = %self.session_id,
                "streamed output could not be written to the session log"
            );
            state.failed = Some(error);
        }
    }

    fn item_type(item: &Item) -> &'static str {
        match item {
            Item::Message(_) => "message",
            Item::Reasoning(_) => "reasoning",
            Item::FunctionCall(_) => "function_call",
            Item::FunctionCallOutput(_) => "function_call_output",
            _ => "other",
        }
    }

    /// Establish or validate the call-local `index ↔ id ↔ type ↔ seq` identity.
    fn ensure_open(
        &self,
        state: &mut ProjectionState,
        id: &str,
        item: &Item,
        output_index: u32,
    ) -> bool {
        let item_type = Self::item_type(item);
        if let Some(open) = state.open.get(id) {
            if open.output_index != output_index {
                self.fail(
                    state,
                    LitecodeError::Llm(format!(
                        "provider item `{id}` moved from output index {} to {output_index}",
                        open.output_index
                    )),
                );
                return false;
            }
            if open.item_type != item_type {
                self.fail(
                    state,
                    LitecodeError::Llm(format!(
                        "provider item `{id}` changed type from `{}` to `{item_type}`",
                        open.item_type
                    )),
                );
                return false;
            }
            // Tool-name patches legitimately re-announce the same association.
            state.dirty.insert(id.to_string());
            self.flush(state);
            return state.failed.is_none();
        }
        if let Some(held) = state.by_index.get(&output_index) {
            self.fail(
                state,
                LitecodeError::Llm(format!(
                    "output index {output_index} belongs to provider item `{held}`, not `{id}`"
                )),
            );
            return false;
        }
        self.begin(state, id, item, output_index);
        state.failed.is_none()
    }

    fn begin(&self, state: &mut ProjectionState, id: &str, item: &Item, output_index: u32) {
        match self
            .sessions
            .begin_stream_item(&self.session_id, item, &self.turn_id)
        {
            Ok(seq) => {
                state.open.insert(
                    id.to_string(),
                    OpenRow {
                        seq,
                        output_index,
                        item_type: Self::item_type(item).to_string(),
                    },
                );
                state.by_index.insert(output_index, id.to_string());
                self.publish(&[seq]);
            }
            Err(error) => self.fail(state, error),
        }
    }

    fn validate_reference(&self, state: &mut ProjectionState, id: &str, output_index: u32) -> bool {
        if !state.open.contains_key(id) {
            let Some(item) = state.seen.get(id).cloned() else {
                return false;
            };
            if !self.ensure_open(state, id, &item, output_index) {
                return false;
            }
        }
        let open = &state.open[id];
        if open.output_index == output_index {
            return true;
        }
        self.fail(
            state,
            LitecodeError::Llm(format!(
                "provider item `{id}` emitted output index {output_index}; expected {}",
                open.output_index
            )),
        );
        false
    }

    /// Mark content as arrived, publishing it if the coalescing window elapsed.
    fn touch(&self, state: &mut ProjectionState, id: &str, output_index: u32) {
        if id.is_empty() || !self.validate_reference(state, id, output_index) {
            return;
        }
        state.dirty.insert(id.to_string());
        if state
            .last_flush
            .is_none_or(|at| at.elapsed() >= FLUSH_INTERVAL)
        {
            self.flush(state);
        }
    }

    /// Publish `id` now: a segment the provider declared finished is a natural
    /// boundary, so it is not made to wait for the coalescing window.
    fn checkpoint(&self, state: &mut ProjectionState, id: &str, output_index: u32) {
        if id.is_empty() || !self.validate_reference(state, id, output_index) {
            return;
        }
        state.dirty.insert(id.to_string());
        self.flush(state);
    }

    /// Write every dirty open row, then publish the rows that changed.
    fn flush(&self, state: &mut ProjectionState) {
        if state.dirty.is_empty() {
            return;
        }
        state.last_flush = Some(Instant::now());
        let mut ids: Vec<String> = state.dirty.drain().collect();
        ids.sort_by_key(|id| state.open.get(id).map(|row| row.seq));
        let mut seqs = Vec::with_capacity(ids.len());
        let mut assistant_preview = None;
        for id in ids {
            let Some(seq) = state.open.get(&id).map(|row| row.seq) else {
                continue;
            };
            let Some(item) = state.seen.get(&id).cloned() else {
                continue;
            };
            match self
                .sessions
                .update_stream_item(&self.session_id, seq, &item)
            {
                Ok(()) => {
                    let preview = crate::session::data::sqlite::session::Session::last_assistant_message_preview(
                        std::slice::from_ref(&item),
                    );
                    if !preview.is_empty() {
                        assistant_preview = Some(preview);
                    }
                    seqs.push(seq);
                }
                Err(error) => {
                    self.fail(state, error);
                    return;
                }
            }
        }
        self.publish(&seqs);
        if !seqs.is_empty() {
            self.publish_preview(assistant_preview);
        }
    }

    /// Open rows for terminal items the stream never announced, then flush.
    fn adopt_terminal(&self, state: &mut ProjectionState, output: &[Item]) {
        let mut terminal_ids = HashSet::new();
        for (index, item) in output.iter().enumerate() {
            let Some(id) = item_id_of(item) else {
                self.fail(
                    state,
                    LitecodeError::Llm(format!(
                        "terminal output index {index} has no provider item id"
                    )),
                );
                return;
            };
            if !terminal_ids.insert(id.clone()) {
                self.fail(
                    state,
                    LitecodeError::Llm(format!(
                        "terminal returned provider item `{id}` more than once"
                    )),
                );
                return;
            }
            if let Some(held_index) = state.open.get(&id).map(|open| open.output_index) {
                if !self.ensure_open(state, &id, item, index as u32) {
                    return;
                }
                debug_assert_eq!(held_index, index as u32);
                continue;
            }
            if let Some(later) = state
                .open
                .values()
                .find(|open| open.output_index > index as u32)
            {
                self.fail(
                    state,
                    LitecodeError::Llm(format!(
                        "terminal-only item `{id}` at output index {index} cannot be inserted before already durable output index {}",
                        later.output_index
                    )),
                );
                return;
            }
            if !self.ensure_open(state, &id, item, index as u32) {
                return;
            }
        }
        state.dirty.extend(state.open.keys().cloned());
        self.flush(state);
    }

    fn settle_open(&self, state: &mut ProjectionState, returned: Option<&[Item]>) -> Result<()> {
        let mut ids: Vec<String> = state.open.keys().cloned().collect();
        ids.sort_by_key(|id| state.open[id].seq);
        let mut unsealed: Vec<String> = Vec::new();
        let mut seqs = Vec::new();
        let mut preview = String::new();
        for id in ids {
            let Some(seq) = state.open.get(&id).map(|row| row.seq) else {
                continue;
            };
            let confirmed = returned.and_then(|items| {
                items
                    .iter()
                    .find(|item| item_id_of(item).as_deref() == Some(id.as_str()))
            });
            let payload = match confirmed {
                Some(item) => Some(item.clone()),
                None => state.seen.get(&id).cloned(),
            };
            let Some(mut payload) = payload else {
                let output_index = state.open[&id].output_index;
                state.by_index.remove(&output_index);
                state.open.remove(&id);
                continue;
            };
            if confirmed.is_none() {
                if returned.is_some() {
                    // The call returned a list and this item is not in it. Keeping
                    // the row would leave the log holding an item the transcript
                    // never replays; dropping it is not possible in an append-only
                    // log, so the call fails and the turn is reported as such.
                    unsealed.push(id.clone());
                    continue;
                }
                // No authoritative list at all. The content received is all there
                // is; an item the provider never confirmed is short of content and
                // says so, while one that got its own `output_item.done` keeps the
                // payload the provider gave it.
                if !state.settled_by_provider.contains(&id) {
                    mark_items_incomplete(std::slice::from_mut(&mut payload));
                }
            }
            match self
                .sessions
                .seal_stream_item(&self.session_id, seq, &payload)
            {
                Ok(()) => {
                    seqs.push(seq);
                    // The session row's one-line preview moved with this seal (a
                    // settled assistant message is what it shows), so the list is
                    // told now instead of waiting for the turn to commit.
                    let settled = crate::session::data::sqlite::session::Session::last_assistant_message_preview(
                        std::slice::from_ref(&payload),
                    );
                    if !settled.is_empty() {
                        preview = settled;
                    }
                    state.by_index.remove(&state.open[&id].output_index);
                    state.open.remove(&id);
                }
                Err(error) => self.fail(state, error),
            }
        }
        self.publish(&seqs);
        if !seqs.is_empty() {
            self.publish_preview((!preview.is_empty()).then_some(preview));
        }

        if !unsealed.is_empty() {
            return Err(LitecodeError::Llm(format!(
                "the response omitted {} item(s) the stream announced ({}); one provider \
                 item cannot be streamed without being returned",
                unsealed.len(),
                unsealed.join(", ")
            )));
        }
        Ok(())
    }

    fn publish_preview(&self, assistant_preview: Option<String>) {
        self.observer
            .on_internal(InternalEvent::SessionPreviewUpdated {
                preview: None,
                assistant_preview,
                updated_at: chrono::Utc::now().timestamp_millis(),
            });
    }

    /// Hand the changed rows to live subscribers. The bytes are already durable.
    fn publish(&self, seqs: &[Seq]) {
        if seqs.is_empty() {
            return;
        }
        self.observer.on_internal(InternalEvent::BufferRestamp {
            seqs: seqs.to_vec(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::*;
    use crate::authority::responses::ResponseStreamEvent;
    use crate::config::TurnGuard;
    use crate::runtime::observer::NoopObserver;
    use crate::session::working::WorkingRow;
    use crate::types::user_text;

    /// A recorded GPT-family response: twelve reasoning items (three of them with
    /// multi-part summaries), then the message — each item announced, streamed and
    /// completed — then a terminal payload that re-issues every reasoning item with
    /// fresh `encrypted_content`.
    fn fixture_events() -> Vec<ResponseStreamEvent> {
        include_str!("../../tests/fixtures/sse/responses/reasoning_summary_parts.txt")
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|json| serde_json::from_str(json).expect("fixture event"))
            .collect()
    }

    /// The authoritative list for the call: what the codec returns on completion.
    fn terminal_items(events: &[ResponseStreamEvent]) -> Vec<Item> {
        for event in events {
            if let ResponseStreamEvent::ResponseCompleted(done) = event {
                return done
                    .response
                    .output
                    .iter()
                    .cloned()
                    .map(Item::from)
                    .collect();
            }
        }
        panic!("the fixture has no terminal payload");
    }

    struct Row {
        seq: i64,
        turn_id: String,
        item_type: String,
        state: String,
        body: String,
    }

    struct Fixture {
        _dir: TempDir,
        db: PathBuf,
        sessions: Arc<SessionManager>,
        sid: String,
    }

    impl Fixture {
        fn open() -> Self {
            let dir = TempDir::new().expect("tempdir");
            let db = dir.path().join("sessions.db");
            let sessions = Arc::new(SessionManager::new_for_test(
                Arc::new(TurnGuard::new()),
                db.to_string_lossy().into_owned(),
            ));
            let sid = sessions
                .open_session_sync("/p", "default", Some("m"))
                .expect("session");
            Self {
                _dir: dir,
                db,
                sessions,
                sid,
            }
        }

        fn rows(&self) -> Vec<Row> {
            let conn = Connection::open(&self.db).expect("reopen");
            let mut stmt = conn
                .prepare(
                    "SELECT seq, turn_id, item_type, state, ifnull(body, '') FROM transcript_items
                     WHERE session_id = ?1 ORDER BY seq",
                )
                .expect("prepare");
            let rows = stmt
                .query_map(rusqlite::params![self.sid], |row| {
                    Ok(Row {
                        seq: row.get(0)?,
                        turn_id: row.get(1)?,
                        item_type: row.get(2)?,
                        state: row.get(3)?,
                        body: row.get(4)?,
                    })
                })
                .expect("query");
            rows.collect::<rusqlite::Result<Vec<_>>>().expect("collect")
        }

        /// One call: fold every event, then settle with the call's own items.
        fn run_call(&self, events: &[ResponseStreamEvent], turn: &str) -> Result<()> {
            let items = terminal_items(events);
            self.run_call_with(events, Some(&items), turn)
        }

        fn run_call_with(
            &self,
            events: &[ResponseStreamEvent],
            returned: Option<&[Item]>,
            turn: &str,
        ) -> Result<()> {
            let projection = StreamProjection::new(
                Arc::clone(&self.sessions),
                Arc::new(NoopObserver),
                self.sid.clone(),
                turn.to_string(),
            );
            for event in events {
                projection.observe(event);
            }
            projection.settle(returned)
        }
    }

    fn added_event(id: &str, output_index: u32, reasoning: bool) -> ResponseStreamEvent {
        use crate::authority::responses::{
            AssistantRole, OutputItem, OutputMessage, OutputStatus, ReasoningItem,
            ResponseOutputItemAddedEvent,
        };
        let item = if reasoning {
            OutputItem::Reasoning(ReasoningItem {
                id: Some(id.to_string()),
                summary: vec![],
                content: None,
                encrypted_content: None,
                status: Some(OutputStatus::InProgress),
            })
        } else {
            OutputItem::Message(OutputMessage {
                id: id.to_string(),
                role: AssistantRole::Assistant,
                content: vec![],
                status: OutputStatus::InProgress,
                phase: None,
            })
        };
        ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
            sequence_number: output_index as u64 + 1,
            output_index,
            item,
        })
    }

    fn projection_for(f: &Fixture) -> SharedStreamProjection {
        StreamProjection::new(
            Arc::clone(&f.sessions),
            Arc::new(NoopObserver),
            f.sid.clone(),
            "turn-1".into(),
        )
    }

    fn summary_texts(body: &str) -> Vec<String> {
        let value: serde_json::Value = serde_json::from_str(body).expect("body json");
        value["summary"]
            .as_array()
            .expect("summary array")
            .iter()
            .map(|part| part["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// The whole point: one row per item, in the order the model produced them,
    /// carrying the authoritative copy, attributed to the turn that ran.
    #[test]
    fn a_streamed_item_settles_into_the_row_it_opened() {
        let events = fixture_events();
        let f = Fixture::open();
        f.run_call(&events, "turn-1").expect("call");

        let rows = f.rows();
        assert_eq!(rows.len(), 13, "twelve reasoning items and one message");
        assert!(
            rows[..12].iter().all(|r| r.item_type == "reasoning"),
            "reasoning comes first: {:?}",
            rows.iter()
                .map(|r| r.item_type.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(rows[12].item_type, "message", "the text is last");
        assert!(
            rows.iter().all(|r| r.state == "final"),
            "nothing is left in flight"
        );
        assert!(
            rows.iter().all(|r| r.turn_id == "turn-1"),
            "every row belongs to the real turn: {:?}",
            rows.iter().map(|r| r.turn_id.as_str()).collect::<Vec<_>>()
        );
        assert!(
            !rows.iter().any(|r| r.turn_id.starts_with("orphan-")),
            "a running turn never writes an orphan row"
        );

        // The terminal copy is what the log keeps, not the mid-stream snapshot.
        assert!(rows[0].body.contains("TERMINAL0"), "body: {}", rows[0].body);
        assert!(!rows[0].body.contains("STREAM0"));
        assert!(!rows[0].body.contains("SHELL0"));

        // Multi-part summaries arrive whole, in their own slots.
        assert_eq!(summary_texts(&rows[0].body).len(), 3);
        assert_eq!(summary_texts(&rows[1].body).len(), 2);
        assert_eq!(summary_texts(&rows[7].body).len(), 2);
        assert!(summary_texts(&rows[2].body).is_empty());
    }

    /// Content is durable while the call is still running: a reader that arrives
    /// mid-stream sees the same bytes the log holds, not a private buffer.
    #[test]
    fn content_lands_while_the_call_is_still_running() {
        let events = fixture_events();
        let f = Fixture::open();
        let projection = StreamProjection::new(
            Arc::clone(&f.sessions),
            Arc::new(NoopObserver),
            f.sid.clone(),
            "turn-1".into(),
        );
        // Announce and stream the first reasoning item only.
        let mut seen = 0;
        for event in &events {
            projection.observe(event);
            if let ResponseStreamEvent::ResponseReasoningSummaryTextDone(_) = event {
                seen += 1;
            }
            if seen == 1 {
                break;
            }
        }
        let rows = f.rows();
        assert_eq!(rows.len(), 1, "the open item already owns a row");
        assert_eq!(rows[0].state, "in_progress", "the call is still running");
        assert_eq!(
            summary_texts(&rows[0].body),
            vec!["Checked how the rows are written. ".to_string()],
            "the log holds the part that arrived, not a private buffer: {}",
            rows[0].body
        );
        assert!(
            !rows[0].body.contains("TERMINAL0"),
            "the call has not returned, so its copy cannot be in the log yet"
        );
        projection.settle(None).expect("settle the abandoned call");

        let rows = f.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "final");
        let value: serde_json::Value = serde_json::from_str(&rows[0].body).expect("body json");
        assert_eq!(value["status"].as_str(), Some("incomplete"));
        assert_eq!(
            summary_texts(&rows[0].body),
            vec!["Checked how the rows are written. ".to_string()],
            "partial reasoning keeps what it received"
        );
    }

    /// Settling and then committing the same call must find every row already in
    /// place. Re-appending here is exactly what used to push an item's settled copy
    /// past the text that followed it.
    #[test]
    fn committing_the_call_after_the_stream_appends_nothing() {
        let events = fixture_events();
        let items = terminal_items(&events);
        let f = Fixture::open();
        f.sessions
            .insert_detail_rows(&f.sid, &[user_text("go")])
            .expect("user turn");
        f.run_call(&events, "turn-1").expect("call");
        let streamed = f.rows();
        assert_eq!(streamed.len(), 14, "one user row plus thirteen items");
        let last_seq = streamed.last().expect("a row").seq;

        let mut rows = vec![WorkingRow::persisted(0, user_text("go"))];
        for (i, item) in items.iter().enumerate() {
            rows.push(WorkingRow::persisted((i + 1) as u64, item.clone()));
        }
        let (kind, working, _) = f
            .sessions
            .commit_turn_delta(&f.sid, rows, last_seq, "turn-1")
            .expect("commit");

        let after = f.rows();
        assert_eq!(
            after.len(),
            14,
            "the commit must not add a second copy of anything; kind={kind:?}"
        );
        let seqs: Vec<i64> = after.iter().map(|r| r.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted, "rows stay in seq order");
        assert_eq!(working.len(), 14, "the working set matches the log");
    }

    /// A stream that dies mid-item keeps what it received: the item that never got
    /// a terminal of its own settles as incomplete, and the ones the provider did
    /// finish keep the payload the provider gave them.
    #[test]
    fn an_item_the_call_never_confirmed_settles_incomplete() {
        let mut events = fixture_events();
        // Drop the batch terminal and the message's own terminal: the reasoning
        // items were confirmed one by one, the text never was.
        events.retain(|ev| {
            !matches!(ev, ResponseStreamEvent::ResponseCompleted(_))
                && !matches!(
                    ev,
                    ResponseStreamEvent::ResponseOutputItemDone(done)
                        if matches!(
                            Item::from(done.item.clone()),
                            Item::Message(_)
                        )
                )
        });
        let f = Fixture::open();
        f.run_call_with(&events, None, "turn-1").expect("call");

        let rows = f.rows();
        assert_eq!(rows.len(), 13, "every opened item is still on record");
        assert!(
            rows.iter().all(|r| r.state == "final"),
            "a died stream must not leave rows in flight"
        );
        let last = rows.last().expect("a row");
        assert_eq!(last.item_type, "message");
        let value: serde_json::Value = serde_json::from_str(&last.body).expect("body json");
        assert_eq!(
            value["status"].as_str(),
            Some("incomplete"),
            "the text that arrived is kept, and honestly labelled"
        );
        assert!(
            value["content"][0]["text"]
                .as_str()
                .is_some_and(|t| !t.is_empty()),
            "the partial text is not thrown away: {}",
            last.body
        );
        assert!(
            summary_texts(&rows[0].body).len() == 3,
            "partial reasoning keeps the parts it received"
        );
    }

    /// An item the provider finished on its own is not downgraded just because the
    /// batch terminal never arrived.
    #[test]
    fn an_item_the_provider_finished_keeps_its_own_terminal() {
        let mut events = fixture_events();
        events.retain(|ev| !matches!(ev, ResponseStreamEvent::ResponseCompleted(_)));
        let f = Fixture::open();
        f.run_call_with(&events, None, "turn-1").expect("call");

        let rows = f.rows();
        assert_eq!(rows.len(), 13);
        assert!(
            rows.iter().all(|r| r.state == "final"),
            "a died stream must not leave rows in flight"
        );
        let value: serde_json::Value = serde_json::from_str(&rows[0].body).expect("body json");
        assert_ne!(
            value["status"].as_str(),
            Some("incomplete"),
            "`output_item.done` is that item's terminal payload: {}",
            rows[0].body
        );
    }

    /// One call's identity map does not outlive it. A later call is free to reuse
    /// the provider's id: that is a different item and it gets its own row, in its
    /// own place, and it streams while it arrives.
    #[test]
    fn a_call_that_reuses_a_provider_id_opens_a_new_row() {
        let f = Fixture::open();
        let events = fixture_events();
        f.run_call(&events, "turn-1").expect("first call");
        let first = f.rows();
        assert_eq!(first.len(), 13);

        // A second call reuses every id from the first.
        f.run_call(&events, "turn-2").expect("second call");
        let rows = f.rows();
        assert_eq!(
            rows.len(),
            26,
            "a reused provider id is a new item, not a rewrite: {:?}",
            rows.iter()
                .map(|r| (r.seq, r.item_type.as_str(), r.turn_id.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(
            rows.iter().all(|r| r.state == "final"),
            "both calls left settled rows"
        );
        assert_eq!(
            rows[13].turn_id, "turn-2",
            "the new rows carry the new turn"
        );
        assert!(
            !rows.iter().any(|r| r.turn_id.starts_with("orphan-")),
            "a running turn never writes an orphan row"
        );
    }

    /// A missing `added` is tolerated when a semantic delta still carries the
    /// full call-local identity. The first such delta opens the durable row.
    #[test]
    fn an_item_the_stream_never_announced_opens_on_its_first_delta() {
        let mut events = fixture_events();
        // Drop the announcement of the first reasoning item; its deltas and the
        // terminal payload still name it.
        events.retain(|ev| {
            !matches!(
                ev,
                ResponseStreamEvent::ResponseOutputItemAdded(added)
                    if matches!(
                        Item::from(added.item.clone()),
                        Item::Reasoning(r) if r.id.as_deref() == Some("rs_a0")
                    )
            )
        });
        let f = Fixture::open();
        f.run_call(&events, "turn-1").expect("call");

        let rows = f.rows();
        assert_eq!(
            rows.len(),
            13,
            "the item opened by its first delta still owns exactly one row: {:?}",
            rows.iter().map(|r| r.seq).collect::<Vec<_>>()
        );
        assert!(
            rows.iter().all(|r| r.state == "final"),
            "and it is settled by the projector, not left for the commit"
        );
        let unannounced: Vec<&Row> = rows
            .iter()
            .filter(|r| r.body.contains("TERMINAL0"))
            .collect();
        assert_eq!(
            unannounced.len(),
            1,
            "the item the terminal named is on record exactly once"
        );
        assert_eq!(unannounced[0].item_type, "reasoning");
        assert_eq!(
            unannounced[0].seq, rows[0].seq,
            "the first semantic delta opens the row even when `added` is absent"
        );
    }

    /// A call that returns a list without an item the stream announced is a
    /// contract breach: the log would keep a row the transcript never replays.
    #[test]
    fn a_returned_list_that_omits_a_streamed_item_fails_the_call() {
        let f = Fixture::open();
        let events = fixture_events();
        let mut items = terminal_items(&events);
        items.remove(0);
        let error = f
            .run_call_with(&events, Some(&items), "turn-1")
            .expect_err("an item was streamed but not returned");
        assert!(
            error.to_string().contains("omitted"),
            "the error must say what happened, got {error}"
        );
    }

    #[test]
    fn same_call_identity_conflicts_fail_closed() {
        let f = Fixture::open();

        let moved = projection_for(&f);
        moved.observe(&added_event("msg_1", 0, false));
        moved.observe(&added_event("msg_1", 1, false));
        assert!(
            moved
                .settle(None)
                .expect_err("one id may not move indexes")
                .to_string()
                .contains("moved from output index")
        );

        let reused = projection_for(&f);
        reused.observe(&added_event("msg_2", 0, false));
        reused.observe(&added_event("msg_3", 0, false));
        assert!(
            reused
                .settle(None)
                .expect_err("one index may not identify two items")
                .to_string()
                .contains("belongs to provider item")
        );

        let changed = projection_for(&f);
        changed.observe(&added_event("same", 0, false));
        changed.observe(&added_event("same", 0, true));
        assert!(
            changed
                .settle(None)
                .expect_err("one item may not change type")
                .to_string()
                .contains("changed type")
        );
    }

    #[test]
    fn terminal_duplicate_id_fails_closed() {
        let f = Fixture::open();
        let projection = projection_for(&f);
        let items = [
            match added_event("dup", 0, false) {
                ResponseStreamEvent::ResponseOutputItemAdded(ev) => Item::from(ev.item),
                _ => unreachable!(),
            },
            match added_event("dup", 1, false) {
                ResponseStreamEvent::ResponseOutputItemAdded(ev) => Item::from(ev.item),
                _ => unreachable!(),
            },
        ];
        assert!(
            projection
                .settle(Some(&items))
                .expect_err("terminal ids must be unique")
                .to_string()
                .contains("more than once")
        );
    }

    /// A write that cannot happen is reported, not swallowed: the row would
    /// otherwise sit in flight with nothing left to settle it.
    #[test]
    fn a_write_the_session_refuses_fails_the_call() {
        let f = Fixture::open();
        let events = fixture_events();
        let projection = StreamProjection::new(
            Arc::clone(&f.sessions),
            Arc::new(NoopObserver),
            f.sid.clone(),
            "turn-1".into(),
        );
        for event in &events {
            projection.observe(event);
        }
        // Settle the first row behind the projection's back. Its own settle now
        // has nothing legitimate to write, and a silent `continue` would leave the
        // remaining rows in flight while the turn looked successful.
        let first = f.rows()[0].seq as u64;
        let settled = terminal_items(&events)
            .into_iter()
            .find(|item| item_id_of(item).as_deref() == Some("rs_a0"))
            .expect("the first item");
        f.sessions
            .seal_stream_item(&f.sid, first, &settled)
            .expect("settle behind the projection");

        let error = projection
            .settle(Some(&terminal_items(&events)))
            .expect_err("the refused write must surface");
        assert!(
            error.to_string().contains("only a row in flight"),
            "the session's own refusal is what comes back, got {error}"
        );
    }
}
