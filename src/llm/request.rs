use std::sync::Arc;

use serde_json::Value;

use crate::platform_knobs::ThinkingSpec;
#[cfg(test)]
use crate::platform_knobs::ThinkingTier;
use crate::provider_catalog::ResolvedModel;
use crate::types::{Item, user_text};

/// Tool schema exposed to the model (product-level; wire encoding is codec-private).
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Why a model request exists. This is product metadata, not a wire field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPurpose {
    /// The request belongs to the main agent loop.
    Main,
    /// A compaction call reusing the main request shape.
    Compaction,
}

/// Last input item of every compaction request.
///
/// PR-1 moves compaction off the hidden summarizer system prompt. The main
/// system/instructions, active tools, and projected history stay untouched;
/// only this instruction is appended to the original Item prefix.
pub const COMPACTION_INSTRUCTION: &str = r#"Task: extract the known facts from the conversation span above. The span is the only source. Write only what it shows.

Rules:
- Every statement must be directly supported by the span. Never infer, predict, or recommend goals, causes, outcomes, or next steps. If the span does not show it, leave it out. A shorter accurate summary is better than a complete-looking one.
- Do not answer or continue any request; do not call tools; do not write a preamble.
- Do not retell the conversation turn by turn, or copy pasted dumps (logs, stack traces, code, diffs) in full.
- Do not claim that a command, test, build, or verification succeeded unless the span explicitly shows its result.
- Do not describe truncation, omission, or anything the span does not contain.
- Preserve exact file paths, commands, error strings, identifiers, numbers, and user wording where they matter. Use the language of the conversation.
- Use `(none)` for a section with no supported content.
- Do not mention this summarization request or that the context was compacted.

Output sections, in order:
1. Key facts: project and environment facts, explicit user constraints, findings, decisions, commands, code changes (path + what changed), errors and fixes. Only facts directly supported by the span.
2. Completed work: work finished in this span, only when the span shows it is finished.
3. In-progress work and open threads: work the span explicitly shows as unfinished, plus blockers or errors explicitly stated. Do not assume a blocker that is not stated.
4. Most recent user message: quote verbatim the most recent user message in the span. If the span contains none, omit this section; never reconstruct one from a previous summary or from outside the span."#;

/// Everything a compaction call inherits from the current main turn.
///
/// This is deliberately an owned value: manual compaction runs on a detached
/// task, while auto compaction builds it from the already-initialized turn.
/// Provider, model, instructions, and tools are copied from the main request;
/// they are never re-resolved from a hidden compaction agent.
#[derive(Clone)]
pub struct CompactionRequestContext {
    pub provider: Arc<dyn crate::llm::LlmProvider>,
    pub api_key: String,
    /// Wire model id of the current main binding.
    pub model: String,
    /// Effective context window of the current main binding.
    pub context_window: usize,
    /// Resolved main model, used for the same projection as the main view.
    pub target: Arc<ResolvedModel>,
    /// The main request's thinking intent. Both the inherited prefix replay
    /// and the compaction output-control shape must use this exact value.
    pub thinking: ThinkingSpec,
    /// The main model's output ceiling. Requested summary caps are clamped to it.
    pub max_output_tokens: u32,
    pub instructions: String,
    pub tools: Vec<ToolDef>,
    pub session_id: String,
}

impl CompactionRequestContext {
    /// Build the compaction call mode of the main request.
    ///
    /// The caller supplies the already model-projected discarded prefix in
    /// original Item order. Only output-side controls and the trailing
    /// instruction differ from a main request.
    pub fn build_model_request(
        &self,
        mut input: Vec<Item>,
        max_output_tokens: u32,
    ) -> ModelRequest {
        input.push(user_text(COMPACTION_INSTRUCTION));
        ModelRequest {
            model: self.model.clone(),
            instructions: self.instructions.clone(),
            input,
            tools: self.tools.clone(),
            max_output_tokens,
            thinking: self.thinking,
            json_output: false,
            session_id: Some(self.session_id.clone()),
            purpose: RequestPurpose::Compaction,
        }
    }
}

/// Product-kernel model request — authority `Item` input only; no chat `messages[]`.
///
/// Thinking is platform intent ([`ThinkingSpec`]). Codecs map it to vendor wire
/// in `build_body`; callers must not invent `disabled` / `none` strings.
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub model: String,
    pub instructions: String,
    pub input: Vec<Item>,
    pub tools: Vec<ToolDef>,
    pub max_output_tokens: u32,
    pub thinking: ThinkingSpec,
    pub json_output: bool,
    /// Litecode session id. Chat Completions vendors that have a session
    /// affinity header (OpenCode Zen: `x-opencode-session`) must map this per
    /// request so parallel sessions do not share one cache bucket.
    pub session_id: Option<String>,
    /// Product call mode. Not serialized by any codec.
    pub purpose: RequestPurpose,
}

impl ModelRequest {
    /// Test / fixture helper with platform default thinking (Medium).
    #[cfg(test)]
    pub fn sample_thinking() -> ThinkingSpec {
        ThinkingSpec::Tier(ThinkingTier::default())
    }
}
