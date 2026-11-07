//! Internal support surface for process composition and live transports.
//!
//! These methods are not part of the four business contracts. They feed
//! projection snapshots, construct runtimes, start compaction workers and
//! background auto-turn reservations, run startup/GC/shutdown plumbing, and
//! route permissions. Future external adapters use the business modules instead
//! of this surface.

use super::*;

impl SessionService {
    // ── Projection / live transport ───────────────────────────────────────

    pub(crate) fn subscribe_lifecycle(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.sessions.subscribe_lifecycle()
    }

    pub(crate) fn idle_child_session_ids(&self) -> Vec<(String, String)> {
        self.sessions.idle_child_session_ids()
    }

    pub(crate) fn list_session_ids(&self) -> Result<Vec<String>> {
        self.sessions.data().list_session_ids_blocking()
    }

    pub(crate) fn llm_reconnect(
        &self,
        session_id: &str,
    ) -> Option<crate::llm::reconnect::LlmReconnect> {
        self.sessions.llm_reconnect(session_id)
    }

    pub(crate) fn entry_load_events_range(
        &self,
        session_id: &str,
        from_seq: Seq,
        to_seq: Seq,
    ) -> Result<Vec<SessionEvent>> {
        self.sessions
            .entry_load_events_range(session_id, from_seq, to_seq)
    }

    pub(crate) fn entry_truncate_epoch(&self, session_id: &str) -> u64 {
        self.sessions.entry_truncate_epoch(session_id)
    }

    pub(crate) fn entry_events(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        self.sessions.data().events_blocking(session_id)
    }

    pub(crate) fn entry_task_state(&self, session_id: &str) -> anyhow::Result<TaskReminders> {
        self.sessions
            .with_entry_task_state(session_id, |state| Ok(state.clone()))
    }

    pub(crate) fn entry_memory(
        &self,
        session_id: &str,
    ) -> Result<crate::session::memory::SessionMemory> {
        self.sessions.load_memory(session_id)
    }

    // ── UI transport / buffer facts ───────────────────────────────────────

    pub(crate) fn entry_wire_seq_cursor(&self, session_id: &str) -> (i64, u64) {
        self.sessions.entry_wire_seq_cursor(session_id)
    }

    pub(crate) fn entry_user_anchor_window(
        &self,
        session_id: &str,
        anchor_seq: Option<i64>,
        before: i64,
        after: i64,
    ) -> Result<UserAnchorWindow> {
        self.sessions
            .entry_user_anchor_window(session_id, anchor_seq, before, after)
    }

    pub(crate) fn event_buffer_snapshot(&self, session_id: &str) -> Vec<InternalEnvelope> {
        self.sessions.event_buffer_snapshot(session_id)
    }

    pub(crate) fn child_bindings_for_parent(
        &self,
        parent_session_id: &str,
    ) -> HashMap<String, String> {
        self.sessions.child_bindings_for_parent(parent_session_id)
    }

    pub(crate) fn deleted_child_bindings_for_parent(
        &self,
        parent_session_id: &str,
    ) -> HashMap<String, String> {
        self.sessions
            .deleted_child_bindings_for_parent(parent_session_id)
    }

    pub(crate) fn attach(&self, session_id: &str) -> Option<TurnProgress> {
        self.sessions.attach(session_id)
    }

    pub(crate) fn detach(&self, session_id: &str) {
        self.sessions.detach(session_id);
    }

    pub(crate) fn subscribe(
        &self,
        session_id: &str,
    ) -> Option<broadcast::Receiver<InternalEnvelope>> {
        self.sessions.subscribe(session_id)
    }

    pub(crate) fn emit_status_changed(&self, session_id: &str, status: SessionStatus) {
        self.sessions.emit_status_changed(session_id, status);
    }

    pub(crate) fn mark_startup_settled(&self) {
        self.sessions.mark_startup_settled();
    }

    pub(crate) fn llm_reconnect_retryable(&self, session_id: &str) -> bool {
        self.sessions.llm_reconnect_retryable(session_id)
    }

    // ── Runtime / compaction / lifecycle plumbing ─────────────────────────

    pub(crate) fn latest_persisted_envelope(&self, session_id: &str) -> Option<ContextEnvelope> {
        latest_persisted_envelope(&self.sessions, session_id)
    }

    pub(crate) fn measure_persisted_context_with_breakdown(
        &self,
        session_id: &str,
        envelope: &ContextEnvelope,
        model: &crate::provider_catalog::ResolvedModel,
        data_root: &Path,
    ) -> Result<(ContextMeter, ItemTokenBreakdown)> {
        crate::context_pipeline::measure_persisted_context_with_breakdown(
            &self.sessions,
            session_id,
            envelope,
            model,
            data_root,
        )
    }

    pub(crate) fn measure_working_rows_with_breakdown(
        &self,
        session_id: &str,
        envelope: &ContextEnvelope,
        model: &crate::provider_catalog::ResolvedModel,
        data_root: &Path,
        rows: &[WorkingRow],
    ) -> Result<(ContextMeter, ItemTokenBreakdown)> {
        crate::context_pipeline::measure_working_rows_with_breakdown(
            &self.sessions,
            session_id,
            envelope,
            model,
            data_root,
            rows,
        )
    }

    pub(crate) fn measure_sealed_meter(
        &self,
        session_id: &str,
        request: &crate::llm::CompactionRequestContext,
        prompt_baseline: &ProviderPromptBaseline,
        data_root: &Path,
    ) -> Option<ContextMeter> {
        crate::context_pipeline::CompactPolicy::measure_sealed_meter(
            &self.sessions,
            session_id,
            request,
            prompt_baseline,
            data_root,
        )
    }

    pub(crate) fn build_runtime(
        &self,
        runtime: &RuntimeHandle,
        session_id: String,
        agent_name: &str,
        depth: u32,
        sink: Arc<dyn PermissionSink>,
        observer: Arc<dyn RuntimeObserver>,
    ) -> Result<AgentRuntime> {
        runtime.build_runtime(
            session_id,
            Arc::clone(&self.sessions),
            agent_name,
            depth,
            sink,
            observer,
        )
    }

    pub(crate) fn ensure_session_model(
        &self,
        resolved: &ResolvedConfig,
        session_id: &str,
        agent_id: &str,
    ) -> Option<String> {
        crate::runtime::ensure_session_model(resolved, &self.sessions, session_id, agent_id)
    }

    pub(crate) fn preview_session_model(
        &self,
        resolved: &ResolvedConfig,
        session_id: &str,
        model_ref: &str,
    ) -> Result<crate::runtime::llm_resolve::SwitchPreview> {
        crate::runtime::llm_resolve::preview_session_model(
            resolved,
            &self.sessions,
            session_id,
            model_ref,
        )
    }

    pub(crate) fn spawn_compaction_worker(
        &self,
        lease: CompactionEpisodeLease,
        budget: BudgetPolicy,
        session_id: String,
        request: crate::llm::CompactionRequestContext,
        trigger: CompactionTrigger,
        data_root: PathBuf,
        overflow_rows: Option<Vec<WorkingRow>>,
    ) {
        crate::runtime::exec::spawn_compaction_worker(
            lease,
            budget,
            Arc::clone(&self.sessions),
            session_id,
            request,
            trigger,
            data_root,
            overflow_rows,
        );
    }

    pub(crate) async fn wait_startup_settled(&self) {
        self.sessions.wait_startup_settled().await;
    }

    pub(crate) async fn gc_stale_empty_sessions(&self, max_age: Duration) {
        self.sessions.gc_stale_empty_sessions(max_age).await;
    }

    pub(crate) async fn shutdown_cleanup(&self) {
        self.sessions.shutdown_cleanup().await;
    }

    pub(crate) fn set_turn_status_hook(&self, hook: TurnStatusHook) {
        self.sessions.set_turn_status_hook(hook);
    }

    pub(crate) fn try_begin_compaction_episode(
        &self,
        session_id: &str,
        trigger: CompactionTrigger,
    ) -> Result<CompactionEpisodeLease> {
        self.sessions
            .try_begin_compaction_episode(session_id, trigger)
    }

    // ── Permission routing ────────────────────────────────────────────────

    // ── Background auto-turn plumbing ─────────────────────────────────────

    pub(crate) fn subscriber_count_blocking(&self, session_id: &str) -> usize {
        self.sessions.subscriber_count_blocking(session_id)
    }

    pub(crate) fn has_pending_messages(&self, session_id: &str) -> bool {
        self.sessions.has_pending_messages(session_id)
    }

    pub(crate) fn has_follow_ups(&self, session_id: &str) -> bool {
        self.sessions.has_follow_ups(session_id)
    }

    pub(crate) fn follow_ups_len(&self, session_id: &str) -> usize {
        self.sessions.follow_ups_len(session_id)
    }

    pub(crate) fn reserve_follow_up_auto_turn(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<Option<(TurnProgress, UserInput)>> {
        self.sessions.reserve_turn_and_pop_follow_up(
            session_id,
            turn_id,
            step_max,
            primary_agent,
            project,
        )
    }

    pub(crate) fn restore_follow_up_front(&self, session_id: &str, input: UserInput) {
        self.sessions.restore_follow_up_front(session_id, input);
    }

    pub(crate) fn reserve_pending_auto_turn(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<Option<(TurnProgress, Vec<PendingMessage>)>> {
        self.sessions.reserve_turn_and_claim_pending(
            session_id,
            turn_id,
            step_max,
            primary_agent,
            project,
        )
    }

    pub(crate) fn append_user_message_with_knowledge_status(
        &self,
        session_id: &str,
        input: impl Into<UserInput>,
        workspace: &Path,
    ) -> anyhow::Result<Seq> {
        self.sessions
            .append_user_message_with_knowledge_status(session_id, input, workspace)
    }

    pub(crate) fn reserve_auto_turn(
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

    pub(crate) fn release_auto_turn(&self, session_id: &str, turn_id: &str) -> bool {
        self.sessions.release_turn_reservation(session_id, turn_id)
    }

    pub(crate) fn spawn_auto_turn(
        &self,
        runtime: &RuntimeHandle,
        session_id: String,
        input: impl Into<TurnInput>,
        permission_sink: Arc<dyn PermissionSink>,
        turn_id: String,
        opts: TurnOptions,
    ) -> anyhow::Result<TurnHandle> {
        crate::runtime::spawn_turn(
            runtime,
            session_id,
            Arc::clone(&self.sessions),
            input,
            permission_sink,
            turn_id,
            opts,
        )
    }

    pub(crate) async fn start_auto_turn_with_handle(
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

    pub(crate) fn auto_turn_permission_sink(&self, session_id: &str) -> Arc<dyn PermissionSink> {
        let is_child = self
            .meta(session_id)
            .ok()
            .and_then(|meta| meta.parent_session_id)
            .is_some_and(|parent| !parent.is_empty());
        if is_child {
            return crate::permission::deny_permission_sink();
        }
        self.last_permission_sink(session_id)
            .unwrap_or_else(crate::permission::deny_permission_sink)
    }

    pub(crate) fn last_permission_sink(&self, session_id: &str) -> Option<Arc<dyn PermissionSink>> {
        self.sessions.last_permission_sink(session_id)
    }

    pub(crate) fn set_last_permission_sink(&self, session_id: &str, sink: Arc<dyn PermissionSink>) {
        self.sessions.set_last_permission_sink(session_id, sink);
    }

    pub(crate) fn permission_route(&self, session_id: &str) -> PermissionRoute {
        self.sessions.permission_route(session_id)
    }

    pub(crate) fn existing_permission_route(&self, session_id: &str) -> Option<PermissionRoute> {
        self.sessions.existing_permission_route(session_id)
    }

    // ── Process integration ───────────────────────────────────────────────

    pub(crate) fn data_root_path(&self) -> PathBuf {
        self.sessions.data_root_path()
    }

    pub(crate) fn publish_internal(&self, session_id: &str, event: InternalEvent) -> bool {
        self.sessions.publish_internal(session_id, event)
    }
}
