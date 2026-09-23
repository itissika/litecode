use serde_json::Value;

use crate::platform_knobs::ThinkingSpec;
#[cfg(test)]
use crate::platform_knobs::ThinkingTier;
use crate::types::{Item, user_text};

/// Tool schema exposed to the model (product-level; wire encoding is codec-private).
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
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
    pub temperature: f64,
    pub thinking: ThinkingSpec,
    pub json_output: bool,
    /// Litecode session id. Chat Completions vendors that have a session
    /// affinity header (OpenCode Zen: `x-opencode-session`) must map this per
    /// request so parallel sessions do not share one cache bucket.
    pub session_id: Option<String>,
}

impl ModelRequest {
    /// One-shot compaction summarizer: no tools, thinking off, caller output cap.
    pub fn compact(
        model: impl Into<String>,
        system: &str,
        prompt: &str,
        max_output_tokens: u32,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            instructions: system.to_string(),
            input: vec![user_text(prompt)],
            max_output_tokens,
            temperature: 0.3,
            tools: vec![],
            thinking: ThinkingSpec::Off,
            json_output: false,
            session_id: Some(session_id.into()),
        }
    }

    /// Test / fixture helper with platform default thinking (Medium).
    #[cfg(test)]
    pub fn sample_thinking() -> ThinkingSpec {
        ThinkingSpec::Tier(ThinkingTier::default())
    }
}
