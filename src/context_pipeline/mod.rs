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

use crate::hook::HookDispatcher;
use crate::llm::LlmProvider;
use crate::session::manager::SessionManager;
use crate::session::store::Session;
use crate::session::task_state::TaskReminders;
use crate::types::{Item, Result, Transcript};

pub use budget::{BudgetPolicy, ProviderPromptBaseline, manual_compact_eligible};
pub use compact::CompactPolicy;
pub use env::{Context, build_context};
pub use system::{
    BUILTIN_CODE_REVIEW, BUILTIN_COMPACTION, BUILTIN_IDENTITY, BUILTIN_REMINDER, BUILTIN_TONE,
    BUILTIN_TOOLS, build_system_prompt, compose_system_prompt,
};
pub use view::{HotView, PreparedView};

/// Result of persisting a transcript delta.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommitStepOutcome {
    pub committed: bool,
    pub discarded: bool,
    pub preview: Option<(String, i64)>,
}

struct PipelineState {
    hot: HotView,
    prepared: Option<PreparedView>,
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
    pub fn new(
        _session: &Session,
        context_window: usize,
        _ctx: Context,
        data_root: PathBuf,
    ) -> Self {
        Self {
            budget: BudgetPolicy::new(context_window),
            compact: CompactPolicy,
            data_root,
            state: RefCell::new(PipelineState {
                hot: HotView::new(),
                prepared: None,
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

    /// Load turn working set from Session DB (§5.1 turn load — sole path).
    pub fn begin_turn(&self, session: &Session) -> Result<Transcript> {
        self.begin_turn_with_id(session, None)
    }

    pub fn begin_turn_with_id(
        &self,
        session: &Session,
        turn_id: Option<String>,
    ) -> Result<Transcript> {
        let items = session.load_transcript()?;
        session.reload_persisted_max_seq()?;

        let mut state = self.state.borrow_mut();
        state.turn_id = turn_id;
        state.surface_len = items.len();
        state.log_max_seq = session.max_seq()?;
        state.hot.replace(items.clone());
        state.prepared = None;
        Ok(items)
    }

    pub fn persisted_prefix_len(&self) -> usize {
        self.state.borrow().surface_len
    }

    pub fn end_turn(&self) {
        let mut state = self.state.borrow_mut();
        state.turn_id = None;
        state.surface_len = 0;
        state.log_max_seq = -1;
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
        hooks: &HookDispatcher,
        sessions: &SessionManager,
        session_id: &str,
        ctx: &Context,
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

        if let Ok(from_log) = sessions.with_entry_store(session_id, |s| Ok(s.load_transcript()?))
        {
            *turn_items = from_log;
            let max_seq = sessions.with_entry_store(session_id, |s| Ok(s.max_seq()?))?;
            let mut state = self.state.borrow_mut();
            state.surface_len = turn_items.len();
            state.log_max_seq = max_seq;
        }

        let mut transcript = turn_items.clone();
        let committed_len = self.state.borrow().surface_len;
        let reminder = tail_reminders::build_compaction_content(task_state);

        let compacted = self
            .compact
            .compact_if_needed(
                &self.budget,
                hooks,
                sessions,
                session_id,
                ctx,
                provider,
                api_key,
                compact_model,
                compact_system,
                compact_max_tokens,
                prompt_baseline,
                &mut transcript,
                committed_len,
                reminder.as_deref(),
                step,
                cancel,
            )
            .await?;

        if cancel.is_cancelled() {
            return Ok(false);
        }

        if compacted {
            let (persisted_count, max_seq) = sessions.with_entry_store(session_id, |s| {
                s.reload_persisted_max_seq()?;
                Ok((s.load_transcript()?.len(), s.max_seq()?))
            })?;
            let mut state = self.state.borrow_mut();
            state.surface_len = persisted_count;
            state.log_max_seq = max_seq;
        }

        // Crash / force-kill recovery: dangling FunctionCalls must be padded on
        // the ephemeral LLM view so Chat providers accept the request. Do not
        // persist synthetic outputs as `detail` — the disk keeps the hanging
        // FunctionCall until a real result or abort seal.
        *turn_items = transcript.clone();

        let mut llm_items = transcript;
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
    /// On success, orphan `FunctionCallOutput`s are removed from `items` (same
    /// set the store deleted from DB). On commit failure, `items` is unchanged.
    pub fn commit_step(
        &self,
        session: &Session,
        items: &mut Vec<Item>,
    ) -> Result<CommitStepOutcome> {
        self.commit_step_with_turn(session, items, "")
    }

    pub fn commit_step_with_turn(
        &self,
        session: &Session,
        items: &mut Vec<Item>,
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
        let outcome =
            session.commit_turn_delta_with_orphan_cleanup(items, &[], expected_max_seq, &tid)?;
        match outcome {
            crate::session::store::CommitDeltaOutcome::Discarded => {
                let mut state = self.state.borrow_mut();
                state.surface_len = items.len();
                state.log_max_seq = session.max_seq().unwrap_or(-1);
                state.hot.replace(items.clone());
                state.prepared = None;
                Ok(CommitStepOutcome {
                    committed: false,
                    discarded: true,
                    preview: None,
                })
            }
            crate::session::store::CommitDeltaOutcome::Applied { preview, mutated } => {
                let mut state = self.state.borrow_mut();
                state.surface_len = items.len();
                state.log_max_seq = session.max_seq().unwrap_or(-1);
                state.hot.replace(items.clone());
                if !mutated {
                    return Ok(CommitStepOutcome::default());
                }
                Ok(CommitStepOutcome {
                    committed: true,
                    discarded: false,
                    preview,
                })
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
