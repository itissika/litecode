use crate::types::{FunctionToolCall, Item, Result, Transcript};

// current_thread runtime 下 Future 不需 Send bound，保留 async fn in trait。
#[allow(async_fn_in_trait)]
pub trait AgentDeps {
    /// Call the model using the prepared LLM view (Items + instructions).
    /// Prepared view from `prepare_step` is the source of truth — not the in-memory
    /// transcript slice. Returns output Items to append verbatim to the transcript.
    async fn call_model(&mut self) -> Result<Vec<Item>>;

    /// Execute tools for the given function calls. FunctionCall Items are already
    /// in `transcript` (from model output); this only appends FunctionCallOutput Items.
    /// On cancellation it appends an "interrupted" output for every call before
    /// returning `Canceled`, so the transcript stays valid for the next turn.
    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> Result<()>;

    async fn should_stop(&self, output: &[Item]) -> Result<bool>;

    async fn compact_if_needed(&self, transcript: &mut Transcript, step: u64) -> Result<()>;

    fn emit_todo_progress(&mut self);

    fn is_cancelled(&self) -> bool;

    fn max_steps(&self) -> u32;

    /// Persist in-memory transcript Items to session storage.
    /// On tool steps the loop calls this twice: after model output (FunctionCall
    /// sealed for the client) and again after `execute_tools` (FunctionCallOutput).
    /// On success, orphan `FunctionCallOutput`s are dropped from memory in the
    /// same operation that deletes them from DB. On commit failure, `items` is
    /// unchanged.
    fn persist_items(&self, items: &mut Vec<Item>) -> Result<()>;

    /// Called at the start of each agent-loop step; drives step/phase telemetry.
    fn begin_step(&mut self, step: u64);
}
