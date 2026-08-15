//! Resolve provider + model binding for one LLM turn.

use std::sync::Arc;

use crate::authority::responses::{
    FunctionCallOutput, InputContent, InputTextContent, MessageItem,
};
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::ModelDefinition;
use crate::llm::LlmProvider;
use crate::platform_knobs::{
    ContextMode, ThinkingTier, effective_api_model_id, effective_context_window,
    effective_max_tokens,
};
use crate::runtime::provider_registry::{ProviderRegistry, provider_api_key};
use crate::session::manager::SessionManager;
use crate::session::media_tokens::classify_input_file;
use crate::types::{Item, LitecodeError, Result};

#[derive(Clone)]
pub struct TurnLlmBinding {
    pub provider_id: String,
    pub model_id: String,
    pub api_model_id: String,
    pub context_window: usize,
    pub max_tokens: u32,
    pub thinking_tier: ThinkingTier,
    pub context_mode: ContextMode,
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub model_def: ModelDefinition,
}

/// Resolve the LLM binding for a main session turn.
///
/// Reads **only** `session.model_id`. Empty / missing → Config error.
/// No runtime `?? agent.model_ref` fallback (`model_ref` is new-session seed only).
pub fn resolve_session_llm(
    resolved: &ResolvedConfig,
    registry: &mut ProviderRegistry,
    sessions: &SessionManager,
    session_id: &str,
    settings_revision: u64,
) -> Result<TurnLlmBinding> {
    let model_id = sessions
        .session_model_id(session_id)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            LitecodeError::Config(
                "no model configured for session: open Settings → Agents and assign a model to \
                 default (primary), then create a new session — or use agent/set-model. \
                 Also assign a model to compaction (hidden) or context compaction will fail."
                    .into(),
            )
        })?;

    binding_from_model_id(
        resolved,
        registry,
        &model_id,
        settings_revision,
        sessions.thinking_tier(session_id).unwrap_or_default(),
        sessions.context_mode(session_id).unwrap_or_default(),
    )
}

/// Resolve a binding for an agent (subagent / compaction) without session state.
///
/// Optional `model_id_override` is a catalog config id from the tool call;
/// otherwise uses the agent's Settings `model_ref`.
pub fn binding_for_agent(
    resolved: &ResolvedConfig,
    registry: &mut ProviderRegistry,
    agent_name: &str,
    model_id_override: Option<&str>,
    settings_revision: u64,
) -> Result<TurnLlmBinding> {
    let model_id = model_id_override.map(str::to_string).unwrap_or_else(|| {
        resolved
            .agents()
            .get(agent_name)
            .map(|p| p.model_ref.clone())
            .unwrap_or_default()
    });

    if model_id.trim().is_empty() {
        return Err(LitecodeError::Config(format!(
            "agent '{agent_name}' has no model_ref; open Settings → Agents and assign a model \
             (default is the primary agent; compaction is a hidden agent required for context compaction)"
        )));
    }

    binding_from_model_id(
        resolved,
        registry,
        &model_id,
        settings_revision,
        ThinkingTier::default(),
        ContextMode::default(),
    )
}

fn binding_from_model_id(
    resolved: &ResolvedConfig,
    registry: &mut ProviderRegistry,
    model_id: &str,
    settings_revision: u64,
    thinking_tier: ThinkingTier,
    context_mode: ContextMode,
) -> Result<TurnLlmBinding> {
    let model_def = resolved
        .models()
        .get(model_id)
        .cloned()
        .ok_or_else(|| LitecodeError::Config(format!("model '{model_id}' not found")))?;

    let provider_def = resolved
        .providers()
        .get(&model_def.provider_ref)
        .cloned()
        .ok_or_else(|| {
            LitecodeError::Config(format!(
                "model '{}' provider_ref '{}' does not exist",
                model_id, model_def.provider_ref
            ))
        })?;

    let provider = registry.get(&provider_def, settings_revision)?;
    let api_key = provider_api_key(&provider_def)?;

    Ok(TurnLlmBinding {
        provider_id: model_def.provider_ref.clone(),
        model_id: model_id.to_string(),
        api_model_id: effective_api_model_id(&model_def),
        context_window: effective_context_window(&model_def, context_mode),
        max_tokens: effective_max_tokens(&model_def),
        thinking_tier,
        context_mode,
        provider,
        api_key,
        model_def,
    })
}

fn require_capability(model: &ModelDefinition, cap: &str, context: &str) -> Result<()> {
    if model.supports(cap) {
        Ok(())
    } else {
        Err(LitecodeError::Llm(format!(
            "model '{}' does not support capability '{cap}'{context}",
            model.id
        )))
    }
}

fn validate_input_content(
    content: &InputContent,
    model: &ModelDefinition,
    context: &str,
) -> Result<()> {
    match content {
        InputContent::InputText(_) => Ok(()),
        InputContent::InputImage(_) => require_capability(model, "image", context),
        InputContent::InputFile(file) => {
            // Classify by filename / mime / URL suffix when possible.
            // Fail-closed for clear video/audio/image; unclassifiable document-like
            // files are allowed under `text` (Responses InputFile as generic document).
            match classify_input_file(file) {
                Some("image") => require_capability(model, "image", context),
                Some("video") => require_capability(model, "video", context),
                Some("audio") => require_capability(model, "audio", context),
                None => {
                    // Document-like / unknown: allowed if the model has text.
                    require_capability(model, "text", context)
                }
                Some(_) => unreachable!(),
            }
        }
    }
}

/// Hard-fail when any content in the LLM input uses an unsupported modality.
///
/// Walks user [`MessageItem::Input`] content and [`FunctionCallOutput::Content`] parts.
pub fn validate_llm_input_capabilities(items: &[Item], model: &ModelDefinition) -> Result<()> {
    for item in items {
        match item {
            Item::Message(MessageItem::Input(msg)) => {
                for content in &msg.content {
                    validate_input_content(content, model, "")?;
                }
            }
            Item::FunctionCallOutput(out) => {
                if let FunctionCallOutput::Content(parts) = &out.output {
                    for content in parts {
                        validate_input_content(content, model, " required by tool output")?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Ephemeral LLM-view projection: replace modalities the model cannot consume
/// with actionable text placeholders. Never mutates persisted transcript — call
/// only on a cloned prepare-step view (same contract as media_budget).
///
/// After this, [`validate_llm_input_capabilities`] should succeed for the same
/// `model` (safety net if projection is skipped).
pub fn project_llm_input_for_model(items: &mut [Item], model: &ModelDefinition) {
    for item in items.iter_mut() {
        match item {
            Item::Message(MessageItem::Input(msg)) => {
                for part in msg.content.iter_mut() {
                    if let Some(text) = omit_unsupported_part(part, model) {
                        *part = InputContent::InputText(InputTextContent { text });
                    }
                }
            }
            Item::FunctionCallOutput(out) => {
                let FunctionCallOutput::Content(parts) = &mut out.output else {
                    continue;
                };
                let mut replaced = false;
                for part in parts.iter_mut() {
                    if let Some(text) = omit_unsupported_part(part, model) {
                        *part = InputContent::InputText(InputTextContent { text });
                        replaced = true;
                    }
                }
                if replaced
                    && parts
                        .iter()
                        .all(|p| matches!(p, InputContent::InputText(_)))
                {
                    let text = parts
                        .iter()
                        .filter_map(|p| match p {
                            InputContent::InputText(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    out.output = FunctionCallOutput::Text(text);
                }
            }
            _ => {}
        }
    }
}

/// If `part` needs a capability the model lacks, return placeholder text.
fn omit_unsupported_part(part: &InputContent, model: &ModelDefinition) -> Option<String> {
    match part {
        InputContent::InputText(_) => None,
        InputContent::InputImage(img) => {
            if model.supports("image") {
                return None;
            }
            let loc = img
                .image_url
                .as_deref()
                .or(img.file_id.as_deref())
                .map(truncate_loc)
                .unwrap_or_default();
            Some(omit_note(&model.id, "image", "image", loc.as_str()))
        }
        InputContent::InputFile(file) => {
            let cap = match classify_input_file(file) {
                Some("image") => "image",
                Some("video") => "video",
                Some("audio") => "audio",
                None => return None, // document-like: keep under text
                Some(_) => return None,
            };
            if model.supports(cap) {
                return None;
            }
            let loc = file
                .filename
                .as_deref()
                .or(file.file_url.as_deref())
                .or(file.file_id.as_deref())
                .map(truncate_loc)
                .unwrap_or_default();
            Some(omit_note(&model.id, cap, cap, loc.as_str()))
        }
    }
}

fn omit_note(model_id: &str, cap: &str, kind: &str, loc: &str) -> String {
    if loc.is_empty() {
        format!("[omitted: model '{model_id}' does not support {cap}; original was {kind}]")
    } else {
        format!("[omitted: model '{model_id}' does not support {cap}; original was {kind}: {loc}]")
    }
}

fn truncate_loc(s: &str) -> String {
    const MAX: usize = 120;
    // Prefer showing a short tail for data: URLs / long paths.
    if s.len() <= MAX {
        return s.to_string();
    }
    if s.starts_with("data:") {
        return format!("{}…", &s[..MAX.min(s.len())]);
    }
    let start = s.len().saturating_sub(MAX);
    format!("…{}", &s[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        FunctionCallOutputItemParam, InputFileContent, InputImageContent, InputMessage, InputRole,
        InputTextContent,
    };
    use crate::config::resolved::WorkspaceState;
    use crate::config::schema::{
        ADAPTER_OPENAI_RESPONSES, AgentProfile, AgentRole, GlobalSettings, ModelAdapterConfig,
        ModelCapability, ModelDefinition, ProviderAuth, ProviderConnectionConfig,
        ProviderDefinition,
    };
    use crate::types::user_text;

    fn text_only_model() -> ModelDefinition {
        ModelDefinition {
            id: "text-only".into(),
            adapter_id: crate::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
            provider_ref: "main".into(),
            label: "Text".into(),
            config: crate::config::schema::ModelAdapterConfig {
                api_model_id: "text-model".into(),
                context_window: 8_000,
                max_tokens: 1024,
                thinking_mode: None,
                reasoning_effort: None,
                json_output: false,
                capabilities: vec![ModelCapability::Text],
            },
        }
    }

    fn multimodal_model() -> ModelDefinition {
        ModelDefinition {
            id: "mm".into(),
            adapter_id: crate::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
            provider_ref: "main".into(),
            label: "MM".into(),
            config: crate::config::schema::ModelAdapterConfig {
                api_model_id: "mm".into(),
                context_window: 200_000,
                max_tokens: 8192,
                thinking_mode: None,
                reasoning_effort: None,
                json_output: false,
                capabilities: vec![
                    ModelCapability::Text,
                    ModelCapability::Image,
                    ModelCapability::Video,
                    ModelCapability::Audio,
                ],
            },
        }
    }

    fn user_with_image() -> Item {
        Item::Message(MessageItem::Input(InputMessage {
            content: vec![
                InputContent::InputText(InputTextContent {
                    text: "describe".into(),
                }),
                InputContent::InputImage(InputImageContent {
                    detail: Default::default(),
                    file_id: None,
                    image_url: Some("https://example.com/a.png".into()),
                }),
            ],
            role: InputRole::User,
            status: None,
        }))
    }

    fn tool_with_image() -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "read-1".into(),
            output: FunctionCallOutput::Content(vec![
                InputContent::InputText(InputTextContent {
                    text: "screenshot".into(),
                }),
                InputContent::InputImage(InputImageContent {
                    detail: Default::default(),
                    file_id: None,
                    image_url: Some("https://example.com/a.png".into()),
                }),
            ]),
            id: None,
            status: None,
        })
    }

    #[test]
    fn capability_mismatch_is_hard_error() {
        let err =
            validate_llm_input_capabilities(&[user_with_image()], &text_only_model()).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not support capability 'image'")
        );
    }

    #[test]
    fn tool_image_capability_mismatch_is_hard_error() {
        let items = vec![tool_with_image()];
        let err = validate_llm_input_capabilities(&items, &text_only_model()).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not support capability 'image'")
        );
        assert!(err.to_string().contains("required by tool output"));
    }

    #[test]
    fn video_file_rejected_without_video_cap() {
        let items = vec![
            user_text("hi"),
            Item::Message(MessageItem::Input(InputMessage {
                content: vec![InputContent::InputFile(InputFileContent {
                    file_data: None,
                    file_id: None,
                    file_url: Some("https://example.com/v.mp4".into()),
                    filename: Some("v.mp4".into()),
                    detail: None,
                })],
                role: InputRole::User,
                status: None,
            })),
        ];
        let err = validate_llm_input_capabilities(&items, &text_only_model()).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not support capability 'video'")
        );
    }

    #[test]
    fn multimodal_accepts_image() {
        validate_llm_input_capabilities(&[user_with_image()], &multimodal_model()).unwrap();
    }

    #[test]
    fn document_file_allowed_under_text() {
        let items = vec![Item::Message(MessageItem::Input(InputMessage {
            content: vec![InputContent::InputFile(InputFileContent {
                file_data: None,
                file_id: None,
                file_url: Some("https://example.com/doc.pdf".into()),
                filename: Some("doc.pdf".into()),
                detail: None,
            })],
            role: InputRole::User,
            status: None,
        }))];
        validate_llm_input_capabilities(&items, &text_only_model()).unwrap();
    }

    #[test]
    fn project_user_image_for_text_only_then_validate_ok() {
        let mut items = vec![user_with_image()];
        assert!(validate_llm_input_capabilities(&items, &text_only_model()).is_err());
        project_llm_input_for_model(&mut items, &text_only_model());
        validate_llm_input_capabilities(&items, &text_only_model()).unwrap();
        let Item::Message(MessageItem::Input(msg)) = &items[0] else {
            panic!("expected input message");
        };
        assert!(
            msg.content
                .iter()
                .all(|p| !matches!(p, InputContent::InputImage(_)))
        );
        assert!(msg.content.iter().any(|p| matches!(
            p,
            InputContent::InputText(t) if t.text.contains("does not support image")
        )));
    }

    #[test]
    fn project_tool_image_for_text_only_then_validate_ok() {
        let mut items = vec![tool_with_image()];
        assert!(validate_llm_input_capabilities(&items, &text_only_model()).is_err());
        project_llm_input_for_model(&mut items, &text_only_model());
        validate_llm_input_capabilities(&items, &text_only_model()).unwrap();
        let Item::FunctionCallOutput(out) = &items[0] else {
            panic!("expected FCO");
        };
        match &out.output {
            FunctionCallOutput::Text(t) => {
                assert!(t.contains("screenshot"));
                assert!(t.contains("does not support image"));
                assert!(t.contains("a.png"));
            }
            FunctionCallOutput::Content(parts) => {
                assert!(
                    parts
                        .iter()
                        .all(|p| !matches!(p, InputContent::InputImage(_)))
                );
            }
        }
    }

    #[test]
    fn project_is_noop_for_multimodal() {
        let mut items = vec![user_with_image(), tool_with_image()];
        project_llm_input_for_model(&mut items, &multimodal_model());
        validate_llm_input_capabilities(&items, &multimodal_model()).unwrap();
        assert!(matches!(
            &items[0],
            Item::Message(MessageItem::Input(m))
                if m.content.iter().any(|p| matches!(p, InputContent::InputImage(_)))
        ));
        assert!(matches!(
            &items[1],
            Item::FunctionCallOutput(o)
                if matches!(&o.output, FunctionCallOutput::Content(parts)
                    if parts.iter().any(|p| matches!(p, InputContent::InputImage(_))))
        ));
    }

    #[test]
    fn project_unpoisons_session_for_next_call_model_gate() {
        // Simulates: tool image already in transcript (persisted), text-only model.
        // Without projection validate fails; with projection the call_model gate passes.
        let mut persisted = vec![tool_with_image(), user_text("what is in the image?")];
        assert!(
            validate_llm_input_capabilities(&persisted, &text_only_model()).is_err(),
            "raw transcript must still fail closed"
        );
        let mut llm_view = persisted.clone();
        project_llm_input_for_model(&mut llm_view, &text_only_model());
        validate_llm_input_capabilities(&llm_view, &text_only_model()).unwrap();
        // Persisted clone unchanged (projection is in-place on llm_view only).
        assert!(
            validate_llm_input_capabilities(&persisted, &text_only_model()).is_err(),
            "source transcript must remain unprojected"
        );
    }

    #[test]
    fn hidden_compaction_binding_uses_its_own_provider_and_credentials() {
        let mut global = GlobalSettings::default();
        global.providers.insert(
            "compact-provider".into(),
            ProviderDefinition {
                id: "compact-provider".into(),
                adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
                label: "Compact".into(),
                config: ProviderConnectionConfig {
                    endpoint: "https://compact.example/v1".into(),
                    api_key: "compact-key".into(),
                    auth: ProviderAuth::Bearer,
                },
            },
        );
        global.models.insert(
            "compact-model".into(),
            ModelDefinition {
                id: "compact-model".into(),
                adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
                provider_ref: "compact-provider".into(),
                label: "Compact".into(),
                config: ModelAdapterConfig {
                    api_model_id: "compact-api-model".into(),
                    context_window: 64_000,
                    max_tokens: 2_048,
                    thinking_mode: None,
                    reasoning_effort: None,
                    json_output: false,
                    capabilities: vec![ModelCapability::Text],
                },
            },
        );
        global.agents.insert(
            "compaction".into(),
            AgentProfile {
                role: AgentRole::Hidden,
                model_ref: "compact-model".into(),
                ..Default::default()
            },
        );

        let resolved =
            crate::config::resolved::resolve(global, WorkspaceState::new("/tmp/compact-binding"));
        let binding = binding_for_agent(
            &resolved,
            &mut ProviderRegistry::new(),
            "compaction",
            None,
            0,
        )
        .expect("resolve hidden compaction binding");

        assert_eq!(binding.provider_id, "compact-provider");
        assert_eq!(
            binding.provider.endpoint(),
            "https://compact.example/v1/responses"
        );
        assert_eq!(binding.api_key, "compact-key");
        assert_eq!(binding.api_model_id, "compact-api-model");
        assert_eq!(binding.max_tokens, 2_048);
    }
}
