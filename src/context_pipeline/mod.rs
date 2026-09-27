pub mod budget;
pub mod compact;
pub mod env;
pub mod estimate;
pub mod keep_recent;
pub mod media_budget;
pub mod summary;
pub mod system;
pub mod view;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use crate::llm::item_id_of;
use crate::session::manager::SessionManager;
use crate::session::store::Session;
use crate::session::working::{WorkingRow, project_items};
use crate::types::{Item, LitecodeError, Result};

pub use budget::{BudgetPolicy, ProviderPromptBaseline, manual_compact_eligible};
pub use compact::CompactPolicy;
pub use env::{Context, build_context};
pub use system::build_system_prompt;
pub use view::{HotView, PreparedView};

/// Result of persisting a transcript delta.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommitStepOutcome {
    pub committed: bool,
    pub discarded: bool,
    pub preview: Option<crate::session::data::command::SessionListPreview>,
    /// Existing log rows sealed by this commit and requiring live re-stamps.
    pub sealed_seqs: Vec<crate::session::event::Seq>,
}

struct PipelineState {
    hot: HotView,
    prepared: Option<PreparedView>,
    working: Vec<WorkingRow>,
    /// Last observed log `MAX(seq)` (`-1` if empty). A later commit treats a
    /// smaller cursor as a revert and discards the turn's tail.
    log_max_seq: i64,
    turn_id: Option<String>,
}

/// L1 context pipeline: single prepare / commit entry for LLM-bound context.
pub struct ContextPipeline {
    budget: BudgetPolicy,
    compact: CompactPolicy,
    data_root: PathBuf,
    /// Mutex so a subagent turn can run as a `Send` tool future on the parent runtime.
    state: Mutex<PipelineState>,
}

/// Map a padded LLM view back to durable rows without treating a provider item ID
/// as unique across requests. The pad operation preserves the original item order;
/// synthetic outputs have no matching source item and receive `None`.
fn align_padded_item_seqs(
    source_items: &[Item],
    source_seqs: &[Option<crate::session::event::Seq>],
    view_items: &[Item],
) -> Vec<Option<crate::session::event::Seq>> {
    debug_assert_eq!(source_items.len(), source_seqs.len());
    let mut source_index = 0usize;
    let aligned: Vec<_> = view_items
        .iter()
        .map(|item| {
            if source_items.get(source_index) == Some(item) {
                let seq = source_seqs[source_index];
                source_index += 1;
                seq
            } else {
                None
            }
        })
        .collect();
    debug_assert_eq!(source_index, source_items.len());
    aligned
}

/// Bind this step's items onto stream rows opened after `cursor`. A provider
/// id is only unique inside one call, so older rows are not candidates.
fn attach_step_items(rows: &mut Vec<WorkingRow>, items: &[Item], cursor: i64) {
    let mut used = HashSet::new();
    for item in items {
        let bound = item_id_of(item).and_then(|id| {
            rows.iter().position(|row| {
                let Some(seq) = row.log_seq else {
                    return false;
                };
                (seq as i64) > cursor
                    && !used.contains(&seq)
                    && item_id_of(&row.item).as_deref() == Some(id.as_str())
            })
        });
        if let Some(index) = bound {
            let seq = rows[index].log_seq.expect("bound row has a seq");
            used.insert(seq);
            rows[index].replace_item(item.clone());
        } else {
            rows.push(WorkingRow::pending(item.clone()));
        }
    }
}

impl ContextPipeline {
    pub fn new(context_window: usize, _ctx: Context, data_root: PathBuf) -> Self {
        Self {
            budget: BudgetPolicy::new(context_window),
            compact: CompactPolicy,
            data_root,
            state: Mutex::new(PipelineState {
                hot: HotView::new(),
                prepared: None,
                working: Vec::new(),
                log_max_seq: -1,
                turn_id: None,
            }),
        }
    }

    /// Override keep-recent token window (integration tests).
    pub fn with_keep_recent_tokens(mut self, tokens: usize) -> Self {
        self.budget = self.budget.with_keep_recent_tokens(tokens);
        self
    }

    pub fn data_root(&self) -> &PathBuf {
        &self.data_root
    }

    pub fn sync_context(&self, _ctx: &Context) {}

    pub fn prepared_view(&self) -> Option<PreparedView> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepared
            .clone()
    }

    pub fn take_prepared_view(&self) -> Option<PreparedView> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepared
            .take()
    }

    /// Persist working set last synced from the session gate (and pending tail).
    pub fn working_set(&self) -> Vec<WorkingRow> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .working
            .clone()
    }

    /// The turn this pipeline is currently running, if any.
    ///
    /// Rows written while a turn is running belong to it; nothing may be written
    /// without one.
    pub fn current_turn_id(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .turn_id
            .clone()
    }

    /// Load turn working set from Session DB (§5.1 turn load — sole path).
    pub fn begin_turn(
        &self,
        sessions: &SessionManager,
        session_id: &str,
    ) -> Result<Vec<WorkingRow>> {
        self.begin_turn_with_id(sessions, session_id, None)
    }

    pub fn begin_turn_with_id(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        turn_id: Option<String>,
    ) -> Result<Vec<WorkingRow>> {
        let rows = sessions.data().working_set_blocking(session_id)?;
        let max_seq = sessions.entry_wire_seq_cursor(session_id).0;

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.turn_id = turn_id;
        state.log_max_seq = max_seq;
        state.working = rows.clone();
        state.hot.replace(project_items(&rows));
        state.prepared = None;
        Ok(rows)
    }

    /// Install rows the caller already holds (a `begin_turn` load plus pending
    /// tails) without writing them.
    pub fn stage_working(&self, rows: Vec<WorkingRow>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.working = rows;
    }

    /// Reload when the log cursor moved. Pending rows (`log_seq == None`) are
    /// appended after the fold. A shorter log is a revert: drop the pending
    /// tail and leave `log_max_seq` so the next commit still sees the shrink.
    fn sync_turn_working(&self, sessions: &SessionManager, session_id: &str) {
        let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
        let (must_reload, shrunk, pending) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let shrunk = !state.working.is_empty() && max_seq < state.log_max_seq;
            let must_reload = state.working.is_empty() || max_seq != state.log_max_seq;
            let pending: Vec<WorkingRow> = state
                .working
                .iter()
                .filter(|row| row.log_seq.is_none())
                .cloned()
                .collect();
            (must_reload, shrunk, pending)
        };
        if !must_reload {
            return;
        }
        let Ok(mut rows) = sessions.data().working_set_blocking(session_id) else {
            return;
        };
        if !shrunk {
            rows.extend(pending);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.working = rows;
        if !shrunk {
            state.log_max_seq = max_seq;
        }
    }

    pub fn end_turn(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.turn_id = None;
        state.log_max_seq = -1;
        state.working.clear();
        state.hot.replace(Vec::new());
        state.prepared = None;
    }

    /// Compact when the budget says so. Does not build the model view.
    ///
    /// Returns whether a compaction ran. Callers that compact sync the request
    /// seam again before [`Self::build_view`], so restored reminders land after
    /// the summary row.
    pub async fn compact_step(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        llm: crate::llm::CompactLlmCall<'_>,
        compact_system: &str,
        compact_max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        step: u64,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        // Returns whether a full compaction ran — single source of truth for
        // the caller's phase/compaction events (no duplicate budget math).
        if cancel.is_cancelled() {
            return Ok(false);
        }

        self.sync_turn_working(sessions, session_id);
        let mut rows = self.working_set();
        let compacted = self
            .compact
            .compact_if_needed(
                &self.budget,
                sessions,
                session_id,
                llm,
                compact_system,
                compact_max_tokens,
                prompt_baseline,
                &mut rows,
                step,
                cancel,
            )
            .await?;

        if cancel.is_cancelled() {
            return Ok(false);
        }

        if compacted {
            // `compact_if_needed` already reloaded the fold once and put the
            // pending tail back. Do not load the log again.
            let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.log_max_seq = max_seq;
            state.working = rows;
        }
        Ok(compacted)
    }

    /// Build the ephemeral model view from the current working set.
    ///
    /// Reloads the log first so reminders appended after compaction are in the
    /// view. Synthetic unanswered-call pads exist only on this view.
    pub fn build_view(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        prompt_baseline: &ProviderPromptBaseline,
        model: &crate::provider_catalog::ResolvedModel,
    ) -> Result<()> {
        self.sync_turn_working(sessions, session_id);
        let (turn_view, source_seqs) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                project_items(&state.working),
                state
                    .working
                    .iter()
                    .map(|row| row.log_seq)
                    .collect::<Vec<_>>(),
            )
        };

        let mut llm_items = turn_view.clone();
        Session::pad_unanswered_calls(&mut llm_items);
        let item_seqs = align_padded_item_seqs(&turn_view, &source_seqs, &llm_items);
        crate::runtime::project_llm_input_for_model(&mut llm_items, model);
        let media_limit = media_budget::media_budget_limit(self.budget.context_window);
        media_budget::apply_media_token_budget(&mut llm_items, media_limit);
        // Refs stay small in the token count (image cost ignores URL length).
        // Expand them only after budgeting, and only on this ephemeral view.
        crate::session::media::resolve_user_media(&mut llm_items, &self.data_root);

        let token_count = self
            .budget
            .token_count_with_baseline(&llm_items, prompt_baseline);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.hot.replace(turn_view);
        state.prepared = Some(PreparedView {
            items: llm_items,
            item_seqs,
            token_count,
            instructions: None,
        });
        Ok(())
    }

    /// Persist item delta since the last commit.
    ///
    /// On success, orphan `FunctionCallOutput`s are removed from `rows` (same
    /// set the store dropped from the in-memory working set). On commit failure,
    /// `rows` is unchanged.
    pub fn commit_step(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        rows: &mut Vec<WorkingRow>,
    ) -> Result<CommitStepOutcome> {
        self.commit_step_with_turn(sessions, session_id, rows, "")
    }

    /// Commit `items` produced by this step.
    ///
    /// Rows the stream already opened (`seq` above the cursor at view-build
    /// time) are claimed by provider item id. Everything else is a pending
    /// tail. A failed commit leaves those new rows out of `state.working`.
    pub fn persist_new(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        items: &[Item],
    ) -> Result<CommitStepOutcome> {
        let cursor = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .log_max_seq;
        self.sync_turn_working(sessions, session_id);
        let mut rows = self.working_set();
        attach_step_items(&mut rows, items, cursor);
        self.commit_step(sessions, session_id, &mut rows)
    }

    pub fn commit_step_with_turn(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        rows: &mut Vec<WorkingRow>,
        turn_id: &str,
    ) -> Result<CommitStepOutcome> {
        let expected_max_seq = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .log_max_seq;
        let tid = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if turn_id.is_empty() {
                state.turn_id.clone().unwrap_or_default()
            } else {
                turn_id.to_string()
            }
        };
        let (kind, working, preview) = sessions
            .commit_turn_delta(session_id, rows.clone(), expected_max_seq, &tid)
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        *rows = working;
        let (committed, discarded, sealed_seqs, clear_prepared) = match kind {
            crate::session::data::command::CommitKind::Idempotent => {
                (false, true, Vec::new(), true)
            }
            crate::session::data::command::CommitKind::Sealed { seqs } => {
                (true, false, seqs, false)
            }
            crate::session::data::command::CommitKind::MetaUpdated
            | crate::session::data::command::CommitKind::Appended { .. } => {
                (true, false, Vec::new(), false)
            }
            _ => (false, false, Vec::new(), false),
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.working = rows.clone();
        state.log_max_seq = sessions.entry_wire_seq_cursor(session_id).0;
        state.hot.replace(project_items(rows));
        if clear_prepared {
            state.prepared = None;
        }
        Ok(CommitStepOutcome {
            committed,
            discarded,
            preview,
            sealed_seqs,
        })
    }

    /// Token estimate for the last prepared view or hot items.
    pub fn current_token_estimate(&self, turn_items: &[Item]) -> usize {
        if let Some(count) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepared
            .as_ref()
            .map(|view| view.token_count)
        {
            return count;
        }
        self.budget.token_count(turn_items, 0)
    }

    pub fn will_compact(&self, items: &[Item], last_prompt_tokens: u64) -> bool {
        let token_count = self.budget.token_count(items, last_prompt_tokens);
        self.budget.should_compact(token_count)
    }
}

#[cfg(test)]
mod turn_window_tests {
    use super::*;
    use crate::types::user_text;

    #[test]
    fn padding_alignment_uses_order_not_provider_ids() {
        let source = vec![user_text("same"), user_text("same")];
        let padded = vec![
            user_text("same"),
            crate::types::assistant_text("host pad"),
            user_text("same"),
        ];
        assert_eq!(
            align_padded_item_seqs(&source, &[Some(10), Some(20)], &padded),
            vec![Some(10), None, Some(20)]
        );
    }
}

/// A streamed item already owns a row; the commit that follows must find it.
///
/// The projector writes rows while the model is still talking, so the pipeline's
/// cached working set is stale by the time the agent hands the same items back.
/// Treating them as new is what appends a second copy of every streamed item —
/// the duplicate the reader sees as repeated reasoning and repeated tool calls.
#[cfg(test)]
mod stream_commit_tests {
    use std::sync::Arc;

    use super::*;
    use crate::authority::responses::{AssistantRole, OutputMessage, OutputStatus};
    use crate::config::TurnGuard;
    use crate::session::SessionManager;
    use crate::types::user_text;

    fn assistant(id: &str, text: &str) -> Item {
        Item::Message(crate::authority::responses::MessageItem::Output(
            OutputMessage {
                id: id.to_string(),
                role: AssistantRole::Assistant,
                content: vec![
                    crate::authority::responses::OutputMessageContent::OutputText(
                        crate::authority::responses::OutputTextContent {
                            text: text.to_string(),
                            annotations: vec![],
                            logprobs: None,
                        },
                    ),
                ],
                status: OutputStatus::Completed,
                phase: None,
            },
        ))
    }

    fn test_context() -> Context {
        let root = PathBuf::from("/p");
        Context {
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(&root),
            cwd: root,
            agents_md: None,
            claude_md: None,
        }
    }

    fn persisted_seqs(sessions: &SessionManager, sid: &str) -> Vec<u64> {
        sessions
            .data()
            .events_blocking(sid)
            .expect("events")
            .into_iter()
            .map(|event| event.seq)
            .collect()
    }

    #[test]
    fn committing_a_streamed_item_does_not_append_a_second_copy() {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid = sessions
            .open_session_sync("/p", "default", Some("m"))
            .expect("session");
        sessions
            .insert_detail_rows(&sid, &[user_text("go")])
            .expect("user row");

        let streamed = assistant("asst_stream", "hello");
        let pipeline = ContextPipeline::new(0, test_context(), PathBuf::from("/p"));
        // The view cursor is the user row. The stream opens its row after that.
        pipeline
            .begin_turn_with_id(&sessions, &sid, Some("turn-1".into()))
            .expect("turn");
        let seq = sessions
            .begin_stream_item(&sid, &streamed, "turn-1")
            .expect("open");
        sessions
            .seal_stream_item(&sid, seq, &streamed)
            .expect("seal");

        let before = persisted_seqs(&sessions, &sid);
        assert_eq!(
            before.len(),
            2,
            "user row plus one streamed row: {before:?}"
        );

        pipeline
            .persist_new(&sessions, &sid, &[streamed])
            .expect("commit");

        let after = persisted_seqs(&sessions, &sid);
        assert_eq!(
            after, before,
            "persist_new must bind the streamed item to the row it already owns, not append"
        );
        let owned = sessions
            .data()
            .working_set_blocking(&sid)
            .expect("working")
            .into_iter()
            .find(|row| row.log_seq == Some(seq))
            .expect("streamed seq");
        assert_eq!(item_id_of(&owned.item).as_deref(), Some("asst_stream"));
    }
}
