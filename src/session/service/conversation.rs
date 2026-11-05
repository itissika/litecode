//! Business: conversation and turns.
//!
//! Prompt/queue delivery, follow-ups, stop, revert, manual compaction and turn
//! start. Runtime construction, measurement and compaction workers are in
//! [`super::support`].
//!
//! Candidate contract operations: `enqueue_user_input`, `enqueue_follow_up`,
//! `start_turn`, `stop_turn_async`, `revert_to_user_anchor`,
//! `begin_file_revert`, `start_manual_compaction`.

use super::*;

impl SessionService {
    // ── Turn lifecycle ────────────────────────────────────────────────────

    fn reserve_turn(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<TurnProgress> {
        self.sessions
            .reserve_turn(session_id, turn_id, step_max, primary_agent, project)
    }

    fn release_turn_reservation(&self, session_id: &str, turn_id: &str) -> bool {
        self.sessions.release_turn_reservation(session_id, turn_id)
    }

    fn spawn_turn(
        &self,
        runtime: &RuntimeHandle,
        session_id: String,
        input: impl Into<TurnInput>,
        permission_sink: Arc<dyn PermissionSink>,
        turn_id: String,
        opts: TurnOptions,
    ) -> anyhow::Result<TurnHandle> {
        spawn_turn(
            runtime,
            session_id,
            Arc::clone(&self.sessions),
            input,
            permission_sink,
            turn_id,
            opts,
        )
    }

    pub(crate) async fn cancel_turn_async(
        &self,
        session_id: &str,
        by: StopOrigin,
    ) -> CancelOutcome {
        self.sessions.cancel_turn(session_id, by).await
    }

    /// User Stop is one business operation: invalidate durable asks before
    /// cancelling the turn, so no permission card survives the stop.
    pub(crate) async fn stop_turn_async(&self, session_id: &str, by: StopOrigin) -> CancelOutcome {
        self.clear_pending_permissions(session_id);
        self.cancel_turn_async(session_id, by).await
    }

    /// Revert the transcript to a user anchor as one business operation: take
    /// the exclusive lease, truncate, then discard messages queued for the
    /// state that was just removed. The lease lives until the operation ends.
    pub(crate) fn revert_to_user_anchor(
        &self,
        session_id: &str,
        seq: u64,
    ) -> std::result::Result<RevertGuard, RevertError> {
        let lease = match self.sessions.try_begin_revert(session_id) {
            Ok(lease) => lease,
            Err(LitecodeError::AgentAlreadyRunning) => return Err(RevertError::Busy),
            Err(error) => return Err(RevertError::Fatal(error)),
        };
        let anchor = match i64::try_from(seq) {
            Ok(anchor) => anchor,
            Err(_) => return Err(RevertError::InvalidAnchor(format!("seq={seq}"))),
        };
        match self
            .sessions
            .entry_revert_to_user_anchor(session_id, anchor)
        {
            Ok(()) => {
                if let Some(lease) = lease.as_ref() {
                    self.sessions
                        .discard_pending_messages_for_revert(session_id, lease.operation_id());
                }
                Ok(RevertGuard { _lease: lease })
            }
            Err(LitecodeError::InvalidRevertAnchor(message)) => {
                Err(RevertError::InvalidAnchor(message))
            }
            Err(error) => Err(RevertError::Internal(error.to_string())),
        }
    }

    /// Claim the exclusive file-revert operation and resolve the snapshot stem.
    /// The caller performs the workspace restore while holding this plan; the
    /// lease releases when the plan is dropped.
    pub(crate) fn begin_file_revert(
        &self,
        session_id: &str,
        seq: u64,
    ) -> std::result::Result<FileRevertPlan, RevertError> {
        let lease = match self
            .sessions
            .try_begin_operation(session_id, SessionOperationKind::Revert)
        {
            Ok(lease) => lease,
            Err(LitecodeError::AgentAlreadyRunning) => return Err(RevertError::Busy),
            Err(error) => return Err(RevertError::Fatal(error)),
        };
        let anchor = match i64::try_from(seq) {
            Ok(anchor) => anchor,
            Err(_) => return Err(RevertError::InvalidAnchor(format!("seq={seq}"))),
        };
        let stem = match self
            .sessions
            .entry_snapshot_stem_for_user_seq(session_id, anchor)
        {
            Ok(stem) => stem,
            Err(LitecodeError::InvalidRevertAnchor(message)) => {
                return Err(RevertError::InvalidAnchor(message));
            }
            Err(error) => return Err(RevertError::Fatal(error)),
        };
        Ok(FileRevertPlan {
            _lease: lease,
            stem,
        })
    }

    // ── Message delivery / cancellation ───────────────────────────────────

    pub(crate) fn append_plan_execute(
        &self,
        session_id: &str,
        item: &crate::types::Item,
    ) -> anyhow::Result<()> {
        self.sessions.append_plan_execute(session_id, item)
    }

    pub(crate) fn enqueue_follow_up(
        &self,
        session_id: &str,
        input: UserInput,
    ) -> Result<FollowUpOutcome> {
        self.sessions.enqueue_follow_up(session_id, input)
    }

    pub(crate) fn enqueue_user_input(
        &self,
        session_id: &str,
        input: UserInput,
    ) -> Result<PendingMessage> {
        self.sessions.enqueue_user_input(session_id, input)
    }

    pub(crate) fn cancel_turn(&self, session_id: &str, by: StopOrigin) -> CancelOutcome {
        self.sessions.cancel_turn_sync(session_id, by)
    }

    pub(crate) fn cancel_and_follow_up(
        &self,
        session_id: &str,
        input: UserInput,
        by: StopOrigin,
    ) -> Result<(CancelOutcome, FollowUpOutcome)> {
        self.sessions.cancel_and_follow_up(session_id, input, by)
    }

    /// Start a manual compaction as one service operation: claim the episode,
    /// build the frozen request, measure the gates, and spawn the worker. The
    /// controller only publishes the returned preview and completes its UI
    /// projection after the service has finished the business decision.
    pub(crate) async fn start_manual_compaction<F>(
        &self,
        runtime: &RuntimeHandle,
        session_id: &str,
        publish_preview: F,
    ) -> std::result::Result<ManualCompactionStart, ManualCompactionError>
    where
        F: FnOnce(&ManualCompactionPreview),
    {
        let mut runtime = runtime.clone();
        let mut lease = self
            .try_begin_compaction_episode(session_id, CompactionTrigger::Manual)
            .map_err(ManualCompactionError::Runtime)?;
        let operation_id = lease.id().to_string();

        let prepared = async {
            runtime.apply_non_engine()?;
            runtime.sync_workspace_tool_readiness();

            let default_primary = runtime.desired_primary_agent();
            let agent_id =
                self.resolve_primary_agent_for_session(&runtime, session_id, default_primary)?;
            let _ = self.ensure_session_model(&runtime.resolved, session_id, &agent_id);

            let depth = self.meta(session_id)?.subagent_depth;
            let mut compaction_runtime = self.build_runtime(
                &runtime,
                session_id.to_string(),
                &agent_id,
                depth,
                crate::permission::deny_permission_sink(),
                Arc::new(crate::runtime::observer::NoopObserver),
            )?;
            compaction_runtime.ensure_tooling_initialized().await?;
            let compaction_request = compaction_runtime.compaction_request_context()?;
            if compaction_request.context_window == 0 {
                return Err(ManualCompactionError::Runtime(LitecodeError::Config(
                    "no context window configured for this session".into(),
                )));
            }

            let budget = BudgetPolicy::new(compaction_request.context_window);
            let envelope = ContextEnvelope::from_compaction_request(&compaction_request);
            let data_root = self.data_root_path();
            let state = self.working_set_state(session_id)?;
            let (meter, breakdown) = self.measure_persisted_context_with_breakdown(
                session_id,
                &envelope,
                &compaction_request.target,
                &data_root,
            )?;
            let sealed_meter = self.measure_sealed_meter(
                session_id,
                &compaction_request,
                &ProviderPromptBaseline::default(),
                &data_root,
            );
            let preview = ManualCompactionPreview {
                context_window: compaction_request.context_window,
                envelope: envelope.clone(),
                meter: meter.clone(),
                breakdown,
            };
            publish_preview(&preview);
            let manual_eligible = sealed_meter
                .as_ref()
                .is_some_and(|meter| meter.manual_eligible());
            if !manual_eligible {
                return Err(ManualCompactionError::NothingToCompact);
            }
            if !crate::context_pipeline::CompactPolicy::can_compact_sealed(
                &state.rows,
                &state.in_progress,
                &budget,
            ) {
                let protected = crate::context_pipeline::protected_conversation_tokens_for_request(
                    &compaction_request,
                );
                if !crate::context_pipeline::CompactPolicy::can_prune_sealed(
                    &state.rows,
                    &state.in_progress,
                    protected,
                ) {
                    return Err(ManualCompactionError::NothingToCompact);
                }
            }
            Ok((compaction_request, budget, data_root, preview))
        }
        .await;

        let (compaction_request, budget, data_root, preview) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let kind = if matches!(&error, ManualCompactionError::NothingToCompact) {
                    crate::runtime::observer::CompactionFailKind::NothingToCompact
                } else {
                    crate::runtime::observer::CompactionFailKind::Failed
                };
                lease.finish(crate::session::manager::EpisodeOutcome::Failed {
                    kind,
                    message: match &error {
                        ManualCompactionError::NothingToCompact => {
                            LitecodeError::NothingToCompact.to_string()
                        }
                        ManualCompactionError::Runtime(error) => error.to_string(),
                    },
                    action: crate::session::manager::CompactionFailureAction::Continue,
                });
                return Err(error);
            }
        };

        self.spawn_compaction_worker(
            lease,
            budget,
            session_id.to_string(),
            compaction_request,
            CompactionTrigger::Manual,
            data_root,
            None,
        );
        Ok(ManualCompactionStart { operation_id })
    }

    /// Business turn start: reserve, spawn, attach, and register the turn as
    /// one operation. The caller owns runtime preparation and projection
    /// refresh; any failure releases the reservation before returning.
    pub(crate) async fn start_turn(
        &self,
        runtime: &RuntimeHandle,
        session_id: &str,
        input: impl Into<TurnInput>,
        plan_execution: bool,
        attach: bool,
        permission_sink: Arc<dyn PermissionSink>,
        turn_id: &str,
        primary_agent: &str,
        project: &str,
        step_max: u32,
        opts: TurnOptions,
    ) -> std::result::Result<(), TurnStartError> {
        if let Err(error) = self.reserve_turn(
            session_id,
            turn_id.to_string(),
            step_max,
            primary_agent,
            project,
        ) {
            return Err(match error {
                LitecodeError::AgentAlreadyRunning => TurnStartError::AgentAlreadyRunning,
                other => TurnStartError::Runtime(anyhow::anyhow!(other)),
            });
        }

        let mut input = input.into();
        if plan_execution && let TurnInput::User(user) = &input {
            match self.append_plan_execute(session_id, &crate::types::user_text(&user.text)) {
                Ok(()) => {
                    input = TurnInput::Wake {
                        anchor_user_seq: None,
                    }
                }
                Err(error) => tracing::warn!(
                    session_id,
                    error = %error,
                    "failed to persist plan-execution message; continuing as a user message"
                ),
            }
        }

        let handle = match self.spawn_turn(
            runtime,
            session_id.to_string(),
            input,
            permission_sink,
            turn_id.to_string(),
            opts,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                self.release_turn_reservation(session_id, turn_id);
                return Err(TurnStartError::Runtime(error));
            }
        };

        if attach {
            self.attach(session_id);
        }
        if let Err(error) = self
            .start_turn_with_handle(session_id, handle, primary_agent, project)
            .await
        {
            self.release_turn_reservation(session_id, turn_id);
            return Err(TurnStartError::Runtime(anyhow::anyhow!(error)));
        }
        Ok(())
    }

    async fn start_turn_with_handle(
        &self,
        session_id: &str,
        handle: TurnHandle,
        primary_agent: &str,
        project: &str,
    ) -> Result<()> {
        self.sessions
            .start_turn(
                session_id,
                handle,
                primary_agent,
                project,
                Arc::clone(&self.sessions),
            )
            .await
    }
}
