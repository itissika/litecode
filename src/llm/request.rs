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
    /// Issuer for each input Item, in the same order. This is a sidecar derived
    /// from durable Session seqs and request/header boundaries; IDs are not used
    /// to reconstruct it.
    pub input_origins: Vec<crate::llm::ItemOrigin>,
    /// The issuer of the endpoint receiving this request.
    pub issuer: String,
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
            input_origins: vec![crate::llm::ItemOrigin::Host],
            issuer: String::new(),
        }
    }

    /// Test / fixture helper with platform default thinking (Medium).
    #[cfg(test)]
    pub fn sample_thinking() -> ThinkingSpec {
        ThinkingSpec::Tier(ThinkingTier::default())
    }

    /// Project the replayed input onto the target endpoint's identity rules.
    ///
    /// Per-request only: the session log is never rewritten, so switching back to
    /// the minting endpoint still finds its own identities intact.
    pub fn project_replay(&mut self, store: crate::llm::StoreMode) -> crate::llm::ProjectionReport {
        let source = std::mem::take(&mut self.input);
        let projected = crate::llm::project_for_target(
            &source,
            &self.input_origins,
            &self.issuer,
            store,
        );
        self.input = projected.items;
        projected.report
    }
}

#[cfg(test)]
mod replay_projection_tests {
    use super::*;
    use crate::authority::responses::{
        AssistantRole, OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
        ReasoningItem,
    };
    use crate::llm::{ItemOrigin, StoreMode};

    const OPEN: &str = "opencode@https://opencode.ai/zen/v1/responses";
    const OTHER: &str = "commandcode@https://api.commandcode.ai/provider/v1/responses";

    fn assistant(id: &str, text: &str) -> Item {
        Item::Message(crate::authority::responses::MessageItem::Output(OutputMessage {
            id: id.into(),
            role: AssistantRole::Assistant,
            status: OutputStatus::Completed,
            phase: None,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: text.into(), annotations: vec![], logprobs: None,
            })],
        }))
    }

    fn request(input: Vec<Item>, input_origins: Vec<ItemOrigin>, issuer: &str) -> ModelRequest {
        ModelRequest {
            model: "m".into(), instructions: "sys".into(), input,
            tools: vec![], max_output_tokens: 64, temperature: 0.0,
            thinking: ThinkingSpec::Off, json_output: false,
            session_id: Some("ses".into()), input_origins, issuer: issuer.into(),
        }
    }

    #[test]
    fn switching_endpoints_strips_only_the_foreign_identity_in_request_copy() {
        let input = vec![
            crate::types::user_text("hi"),
            assistant("msg_a", "first"),
            assistant("msg_b", "second"),
        ];
        let mut request = request(input.clone(), vec![
            ItemOrigin::Host,
            ItemOrigin::Issuer { issuer: OPEN.into(), request_key: "t:1".into() },
            ItemOrigin::Issuer { issuer: OTHER.into(), request_key: "t:2".into() },
        ], OPEN);
        let report = request.project_replay(StoreMode::Stored);
        assert_eq!(report.identities_kept, 1);
        assert_eq!(report.identities_stripped, 1);
        assert_eq!(
            crate::types::item_text_preview(&request.input[2]), "second",
            "foreign identity removal must keep semantic content"
        );
        assert!(matches!(
            &request.input[2],
            Item::Message(crate::authority::responses::MessageItem::Output(message)) if message.id.is_empty()
        ));
        assert_eq!(input[2], assistant("msg_b", "second"), "session-owned copy is unchanged");
    }

    #[test]
    fn old_unknown_origin_is_never_guessed_from_a_valid_prefix() {
        let mut request = request(
            vec![assistant("msg_legacy", "text")],
            vec![ItemOrigin::Unknown],
            OPEN,
        );
        request.project_replay(StoreMode::Stored);
        let Item::Message(crate::authority::responses::MessageItem::Output(message)) =
            &request.input[0] else { panic!("expected assistant message") };
        assert!(message.id.is_empty());
    }

    #[test]
    fn stateless_replay_requires_encrypted_reasoning() {
        let input = vec![
            Item::Reasoning(ReasoningItem {
                id: Some("rs_cipher".into()), summary: vec![], content: None,
                encrypted_content: Some("gAAAA".into()), status: None,
            }),
            Item::Reasoning(ReasoningItem {
                id: Some("rs_bare".into()), summary: vec![], content: None,
                encrypted_content: None, status: None,
            }),
        ];
        let same = ItemOrigin::Issuer { issuer: OPEN.into(), request_key: "t:1".into() };
        let mut request = request(vec![input[0].clone(), input[1].clone()], vec![same.clone(), same], OPEN);
        let report = request.project_replay(StoreMode::Stateless);
        assert_eq!(report.items_dropped, 1);
        assert_eq!(request.input.len(), 1);
        let Item::Reasoning(reasoning) = &request.input[0] else { panic!("expected reasoning") };
        assert_eq!(reasoning.id, None);
        assert_eq!(reasoning.encrypted_content.as_deref(), Some("gAAAA"));
    }
}
