pub mod budget;
pub mod compact;
pub mod env;
pub mod estimate;
pub mod keep_recent;
pub mod media_budget;
pub mod summary;
pub mod system;
pub mod tail_reminders;
pub mod view;

use std::cell::RefCell;
use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use crate::llm::LlmProvider;
use crate::session::manager::SessionManager;
use crate::session::store::Session;
use crate::session::task_state::TaskReminders;
use crate::session::working::{WorkingRow, align_working, project_items};
use crate::types::{Item, LitecodeError, Result, Transcript};

pub use budget::{BudgetPolicy, ProviderPromptBaseline, manual_compact_eligible};
pub use compact::CompactPolicy;
pub use env::{Context, build_context};
pub use system::{
    BUILTIN_CODE_REVIEW, BUILTIN_COMPACTION, BUILTIN_IDENTITY, BUILTIN_REMINDER, BUILTIN_TONE,
    BUILTIN_TOOLS, build_compaction_system_prompt, build_system_prompt, compose_system_prompt,
};
pub use view::{HotView, PreparedView};

/// Result of persisting a transcript delta.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommitStepOutcome {
    pub committed: bool,
    pub discarded: bool,
    pub preview: Option<(String, i64)>,
    /// Existing log rows sealed by this commit and requiring live re-stamps.
    pub sealed_seqs: Vec<crate::session::event::Seq>,
}

struct PipelineState {
    hot: HotView,
    prepared: Option<PreparedView>,
    working: Vec<WorkingRow>,
    /// Model-visible working-set length — compact cut mapping only, not persist.
    surface_len: usize,
    /// Last observed log `MAX(seq)` (`-1` if empty). Discarded iff the table is shorter.
    log_max_seq: i64,
    turn_id: Option<String>,
}

/// L1 context pipeline: single prepare / commit entry for LLM-bound context.
pub struct ContextPipeline {
    budget: BudgetPolicy,
    compact: CompactPolicy,
    data_root: PathBuf,
    state: RefCell<PipelineState>,
}

impl ContextPipeline {
    pub fn new(context_window: usize, _ctx: Context, data_root: PathBuf) -> Self {
        Self {
            budget: BudgetPolicy::new(context_window),
            compact: CompactPolicy,
            data_root,
            state: RefCell::new(PipelineState {
                hot: HotView::new(),
                prepared: None,
                working: Vec::new(),
                surface_len: 0,
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
        self.state.borrow().prepared.clone()
    }

    pub fn take_prepared_view(&self) -> Option<PreparedView> {
        self.state.borrow_mut().prepared.take()
    }

    /// Persist working set last synced from the session gate (and pending tail).
    pub fn working_set(&self) -> Vec<WorkingRow> {
        self.state.borrow().working.clone()
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

        let mut state = self.state.borrow_mut();
        state.turn_id = turn_id;
        state.surface_len = rows.len();
        state.log_max_seq = max_seq;
        state.working = rows.clone();
        state.hot.replace(project_items(&rows));
        state.prepared = None;
        Ok(rows)
    }

    pub fn persisted_prefix_len(&self) -> usize {
        self.state.borrow().surface_len
    }

    pub fn end_turn(&self) {
        let mut state = self.state.borrow_mut();
        state.turn_id = None;
        state.surface_len = 0;
        state.log_max_seq = -1;
        state.working.clear();
        state.hot.replace(Vec::new());
        state.prepared = None;
    }

    /// Snip + compact + capability projection.
    /// Compact anti-forgetting reminder rides on the checkpoint Item (label first).
    /// Synthetic unanswered-call pads exist only on the ephemeral LLM view.
    ///
    /// `model` is the turn LLM definition: unsupported modalities are replaced with
    /// text placeholders on the ephemeral LLM view only (persisted transcript untouched).
    pub async fn prepare_step(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        provider: &dyn LlmProvider,
        api_key: &str,
        compact_model: &str,
        compact_system: &str,
        compact_max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        turn_items: &mut Transcript,
        step: u64,
        cancel: &CancellationToken,
        task_state: &TaskReminders,
        model: &crate::config::schema::ModelDefinition,
    ) -> Result<bool> {
        // Returns whether a full compaction ran — single source of truth for
        // the caller's phase/compaction events (no duplicate budget math).
        if cancel.is_cancelled() {
            return Ok(false);
        }

        if let Ok(from_log) = sessions.data().working_set_blocking(session_id) {
            let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
            let persisted_len = from_log.len();
            let tail: Vec<Item> = if turn_items.len() > persisted_len {
                turn_items[persisted_len..].to_vec()
            } else {
                Vec::new()
            };
            let mut rows = from_log;
            for item in tail {
                rows.push(WorkingRow::pending(item));
            }
            *turn_items = project_items(&rows);
            let mut state = self.state.borrow_mut();
            state.working = rows;
            state.surface_len = persisted_len;
            state.log_max_seq = max_seq;
        }

        let mut transcript = turn_items.clone();
        let committed_len = self.state.borrow().surface_len;
        let persisted_seqs: Vec<crate::session::event::Seq> = self
            .state
            .borrow()
            .working
            .iter()
            .take(committed_len)
            .filter_map(|row| row.log_seq)
            .collect();
        let reminder = tail_reminders::build_compaction_content(task_state);

        let compacted = self
            .compact
            .compact_if_needed(
                &self.budget,
                sessions,
                session_id,
                provider,
                api_key,
                compact_model,
                compact_system,
                compact_max_tokens,
                prompt_baseline,
                &mut transcript,
                committed_len,
                &persisted_seqs,
                reminder.as_deref(),
                step,
                cancel,
            )
            .await?;

        if cancel.is_cancelled() {
            return Ok(false);
        }

        if compacted {
            let persisted = sessions.data().working_set_blocking(session_id)?;
            let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
            let mut rows = persisted;
            for item in transcript.iter().skip(rows.len()) {
                rows.push(WorkingRow::pending(item.clone()));
            }
            let mut state = self.state.borrow_mut();
            state.surface_len = rows.iter().filter(|r| r.log_seq.is_some()).count();
            state.log_max_seq = max_seq;
            state.working = rows;
        } else {
            let mut rows = self.state.borrow().working.clone();
            let valid_call_ids: std::collections::HashSet<String> = rows
                .iter()
                .filter_map(|row| match &row.item {
                    Item::FunctionCall(fc) => Some(fc.call_id.clone()),
                    _ => None,
                })
                .collect();
            rows.retain(|row| {
                !matches!(
                    &row.item,
                    Item::FunctionCallOutput(out) if !valid_call_ids.contains(&out.call_id)
                )
            });
            self.state.borrow_mut().working = rows;
        }

        // Crash / force-kill recovery: dangling FunctionCalls must be padded on
        // the ephemeral LLM view so Chat providers accept the request. Do not
        // persist synthetic outputs as `detail` — the disk keeps the hanging
        // FunctionCall until a real result or abort seal.
        *turn_items = project_items(&self.state.borrow().working);

        let mut llm_items = turn_items.clone();
        Session::pad_unanswered_calls(&mut llm_items);
        crate::runtime::project_llm_input_for_model(&mut llm_items, model);

        let token_count = self
            .budget
            .token_count_with_baseline(&llm_items, prompt_baseline);
        let mut state = self.state.borrow_mut();
        state.hot.replace(turn_items.clone());
        state.prepared = Some(PreparedView {
            items: llm_items,
            token_count,
            instructions: None,
        });
        Ok(compacted)
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

    pub fn commit_step_from_items(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        items: &mut Vec<Item>,
    ) -> Result<CommitStepOutcome> {
        let mut rows = self.state.borrow().working.clone();
        align_working(&mut rows, items);
        let outcome = self.commit_step(sessions, session_id, &mut rows)?;
        *items = project_items(&rows);
        Ok(outcome)
    }

    pub fn commit_step_with_turn(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        rows: &mut Vec<WorkingRow>,
        turn_id: &str,
    ) -> Result<CommitStepOutcome> {
        let expected_max_seq = self.state.borrow().log_max_seq;
        let tid = {
            let state = self.state.borrow();
            if turn_id.is_empty() {
                state.turn_id.clone().unwrap_or_default()
            } else {
                turn_id.to_string()
            }
        };
        let (kind, working) = sessions
            .commit_turn_delta(session_id, rows.clone(), expected_max_seq, &tid)
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        *rows = working;
        match kind {
            crate::session::data::command::CommitKind::Idempotent => {
                let mut state = self.state.borrow_mut();
                state.working = rows.clone();
                state.surface_len = rows.len();
                state.log_max_seq = sessions.entry_wire_seq_cursor(session_id).0;
                state.hot.replace(project_items(rows));
                state.prepared = None;
                Ok(CommitStepOutcome {
                    committed: false,
                    discarded: true,
                    preview: None,
                    sealed_seqs: Vec::new(),
                })
            }
            crate::session::data::command::CommitKind::MetaUpdated
            | crate::session::data::command::CommitKind::Appended { .. } => {
                let mut state = self.state.borrow_mut();
                state.working = rows.clone();
                state.surface_len = rows.len();
                state.log_max_seq = sessions.entry_wire_seq_cursor(session_id).0;
                state.hot.replace(project_items(rows));
                Ok(CommitStepOutcome {
                    committed: true,
                    discarded: false,
                    preview: None,
                    sealed_seqs: Vec::new(),
                })
            }
            _ => {
                let mut state = self.state.borrow_mut();
                state.working = rows.clone();
                state.surface_len = rows.len();
                state.log_max_seq = sessions.entry_wire_seq_cursor(session_id).0;
                state.hot.replace(project_items(rows));
                Ok(CommitStepOutcome::default())
            }
        }
    }

    /// Token estimate for the last prepared view or hot items.
    pub fn current_token_estimate(&self, turn_items: &[Item]) -> usize {
        if let Some(ref view) = self.state.borrow().prepared {
            return view.token_count;
        }
        self.budget.token_count(turn_items, 0)
    }

    pub fn will_compact(&self, items: &[Item], last_prompt_tokens: u64) -> bool {
        let token_count = self.budget.token_count(items, last_prompt_tokens);
        self.budget.should_compact(token_count)
    }
}
