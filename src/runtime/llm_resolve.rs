//! Resolve the catalog model + credential for one LLM turn.

use std::sync::Arc;

use crate::authority::responses::{
    FunctionCallOutput, InputContent, InputTextContent, MessageItem,
};
use crate::config::resolved::ResolvedConfig;
use crate::llm::LlmProvider;
use crate::platform_knobs::{ContextMode, ThinkingTier, effective_context_window};
use crate::provider_catalog::{Modality, ResolvedModel};
use crate::runtime::provider_registry::{ProviderRegistry, provider_api_key};
use crate::session::manager::SessionManager;
use crate::session::media_tokens::classify_input_file;
use crate::types::{Item, LitecodeError, Result};

#[derive(Clone)]
pub struct TurnLlmBinding {
    pub provider_id: String,
    /// Stable catalog reference `{provider_id}/{model_id}`.
    pub model_ref: String,
    pub api_model_id: String,
    pub context_window: usize,
    pub max_tokens: u32,
    pub thinking_tier: ThinkingTier,
    pub context_mode: ContextMode,
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub model: Arc<ResolvedModel>,
}

impl TurnLlmBinding {
    pub fn compact_call(&self) -> crate::llm::CompactLlmCall<'_> {
        crate::llm::CompactLlmCall {
            provider: self.provider.as_ref(),
            api_key: &self.api_key,
            model: &self.api_model_id,
        }
    }
}

const MISSING_MODEL_HINT: &str =
    "open Settings → Providers and configure a provider API key, then pick a model for the agent \
     in Settings → Agents (a model that is not declared in provider-catalog.toml cannot be used)";

/// Resolve the LLM binding for a session turn — primary and child alike.
///
/// Reads **only** the row of the session the turn runs in: `model_id`
/// (a catalog reference), `thinking_tier`, `context_mode`. Empty / missing
/// model → Config error. There is no runtime `?? agent.model_ref` fallback.
pub fn resolve_session_llm(
    resolved: &ResolvedConfig,
    registry: &mut ProviderRegistry,
    sessions: &SessionManager,
    session_id: &str,
) -> Result<TurnLlmBinding> {
    let model_ref = sessions
        .session_model_id(session_id)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            LitecodeError::Config(format!(
                "no model configured for this session: pick one in the model switcher. \
                 {MISSING_MODEL_HINT}"
            ))
        })?;

    binding_from_ref(
        resolved,
        registry,
        &model_ref,
        sessions.thinking_tier(session_id).unwrap_or_default(),
        sessions.context_mode(session_id).unwrap_or_default(),
    )
}

/// Resolve the hidden compaction agent's binding — deliberately session-blind.
///
/// This is the ONLY session-independent resolve entry. Compaction must not
/// inherit the compacted session's model / tier / mode, so it reads the hidden
/// `compaction` profile's `model_ref` and runs at the platform defaults.
pub fn binding_for_agent(
    resolved: &ResolvedConfig,
    registry: &mut ProviderRegistry,
    agent_name: &str,
) -> Result<TurnLlmBinding> {
    let model_ref = resolved
        .agents()
        .get(agent_name)
        .map(|profile| profile.model_ref.clone())
        .unwrap_or_default();

    if model_ref.trim().is_empty() {
        return Err(LitecodeError::Config(format!(
            "agent '{agent_name}' has no model; assign one in Settings → Agents \
             (default is the primary agent; compaction is a hidden agent required for context \
             compaction). {MISSING_MODEL_HINT}"
        )));
    }

    binding_from_ref(
        resolved,
        registry,
        &model_ref,
        ThinkingTier::default(),
        ContextMode::default(),
    )
}

fn binding_from_ref(
    resolved: &ResolvedConfig,
    registry: &mut ProviderRegistry,
    model_ref: &str,
    thinking_tier: ThinkingTier,
    context_mode: ContextMode,
) -> Result<TurnLlmBinding> {
    let catalog = resolved.catalog();
    let model = catalog.model(model_ref).cloned().ok_or_else(|| {
        LitecodeError::Config(format!(
            "model '{model_ref}' is not declared in the provider catalog ({}). {}",
            catalog.path().display(),
            MISSING_MODEL_HINT
        ))
    })?;
    let api_key = provider_api_key(resolved, &model.provider_id)?;
    let provider = registry.get(&model)?;

    Ok(TurnLlmBinding {
        provider_id: model.provider_id.clone(),
        model_ref: model.reference.clone(),
        api_model_id: model.id.clone(),
        context_window: effective_context_window(&model, context_mode),
        max_tokens: model.max_output,
        thinking_tier,
        context_mode,
        provider,
        api_key,
        model,
    })
}

fn require_capability(model: &ResolvedModel, modality: Modality, context: &str) -> Result<()> {
    if model.supports(modality) {
        Ok(())
    } else {
        Err(LitecodeError::Llm(format!(
            "model '{}' does not support capability '{}'{context}",
            model.reference,
            modality.as_str()
        )))
    }
}

fn validate_input_content(
    content: &InputContent,
    model: &ResolvedModel,
    context: &str,
) -> Result<()> {
    match content {
        InputContent::InputText(_) => Ok(()),
        InputContent::InputImage(_) => require_capability(model, Modality::Image, context),
        InputContent::InputFile(file) => {
            // Classify by filename / mime / URL suffix when possible.
            // Fail-closed for clear video/audio/image; unclassifiable document-like
            // files are allowed under `text` (Responses InputFile as generic document).
            match classify_input_file(file) {
                Some("image") => require_capability(model, Modality::Image, context),
                Some("video") => require_capability(model, Modality::Video, context),
                Some("audio") => require_capability(model, Modality::Audio, context),
                None => require_capability(model, Modality::Text, context),
                Some(_) => unreachable!(),
            }
        }
    }
}

/// Hard-fail when any content in the LLM input uses an unsupported modality.
///
/// Walks user [MessageItem::Input] content and [FunctionCallOutput::Content] parts.
pub fn validate_llm_input_capabilities(items: &[Item], model: &ResolvedModel) -> Result<()> {
    for item in items {
        match item {
            Item::Message(MessageItem::Input(message)) => {
                for content in &message.content {
                    validate_input_content(content, model, "")?;
                }
            }
            Item::FunctionCallOutput(output) => {
                if let FunctionCallOutput::Content(parts) = &output.output {
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
pub fn project_llm_input_for_model(items: &mut [Item], model: &ResolvedModel) {
    for item in items.iter_mut() {
        match item {
            Item::Message(MessageItem::Input(message)) => {
                for part in message.content.iter_mut() {
                    if let Some(text) = omit_unsupported_part(part, model) {
                        *part = InputContent::InputText(InputTextContent { text });
                    }
                }
            }
            Item::FunctionCallOutput(output) => {
                let FunctionCallOutput::Content(parts) = &mut output.output else {
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
                        .all(|part| matches!(part, InputContent::InputText(_)))
                {
                    let text = parts
                        .iter()
                        .filter_map(|part| match part {
                            InputContent::InputText(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    output.output = FunctionCallOutput::Text(text);
                }
            }
            _ => {}
        }
    }
}

/// If `part` needs a capability the model lacks, return placeholder text.
fn omit_unsupported_part(part: &InputContent, model: &ResolvedModel) -> Option<String> {
    match part {
        InputContent::InputText(_) => None,
        InputContent::InputImage(image) => {
            if model.supports(Modality::Image) {
                return None;
            }
            let location = image
                .image_url
                .as_deref()
                .or(image.file_id.as_deref())
                .map(truncate_loc)
                .unwrap_or_default();
            Some(omit_note(&model.reference, "image", "image", location.as_str()))
        }
        InputContent::InputFile(file) => {
            let capability = match classify_input_file(file) {
                Some("image") => "image",
                Some("video") => "video",
                Some("audio") => "audio",
                None => return None, // document-like: keep under text
                Some(_) => return None,
            };
            let modality = Modality::parse(capability).expect("classified modality");
            if model.supports(modality) {
                return None;
            }
            let location = file
                .filename
                .as_deref()
                .or(file.file_url.as_deref())
                .or(file.file_id.as_deref())
                .map(truncate_loc)
                .unwrap_or_default();
            Some(omit_note(
                &model.reference,
                capability,
                capability,
                location.as_str(),
            ))
        }
    }
}

fn omit_note(model_ref: &str, capability: &str, kind: &str, location: &str) -> String {
    if location.is_empty() {
        format!("[omitted: model '{model_ref}' does not support {capability}; original was {kind}]")
    } else {
        format!(
            "[omitted: model '{model_ref}' does not support {capability}; original was {kind}: {location}]"
        )
    }
}

fn truncate_loc(text: &str) -> String {
    const MAX: usize = 120;
    // Prefer showing a short tail for data: URLs / long paths.
    if text.len() <= MAX {
        return text.to_string();
    }
    if text.starts_with("data:") {
        return format!("{}…", &text[..MAX.min(text.len())]);
    }
    let start = text.len().saturating_sub(MAX);
    format!("…{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        FunctionCallOutputItemParam, InputFileContent, InputImageContent, InputMessage, InputRole,
    };
    use crate::config::resolved::WorkspaceState;
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
    use crate::provider_catalog::ProviderCatalog;
    use crate::types::user_text;
    use std::path::Path;

    const CATALOG: &str = r#"
version = 1
[[providers]]
id = "compact-provider"
name = "Compact"
endpoint = "https://compact.example/v1"
endpoint_type = "responses"

[[models]]
id = "compact-api-model"
provider_id = "compact-provider"
context_window = 64000
max_output = 2048

[[models]]
id = "text-model"
provider_id = "compact-provider"
context_window = 8000
max_output = 1024

[[models]]
id = "mm"
provider_id = "compact-provider"
context_window = 200000
modalities = ["text", "image", "video", "audio"]
"#;

    fn catalog() -> Arc<ProviderCatalog> {
        Arc::new(ProviderCatalog::parse(CATALOG, Path::new("test-catalog.toml")).unwrap())
    }

    fn model(reference: &str) -> Arc<ResolvedModel> {
        Arc::clone(catalog().model(reference).expect("model"))
    }

    fn resolved_with(reference: &str) -> ResolvedConfig {
        let mut global = GlobalSettings::default();
        global
            .provider_credentials
            .insert("compact-provider".into(), "compact-key".into());
        global.agents.insert(
            "compaction".into(),
            AgentProfile {
                role: AgentRole::Hidden,
                model_ref: reference.into(),
                ..Default::default()
            },
        );
        crate::config::resolved::resolve(
            global,
            WorkspaceState::new("/tmp/compact-binding"),
            catalog(),
        )
    }

    fn resolved() -> ResolvedConfig {
        resolved_with("compact-provider/compact-api-model")
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
    fn capability_mismatch_is_a_hard_error() {
        let error = validate_llm_input_capabilities(&[user_with_image()], &model("compact-provider/text-model"))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not support capability 'image'")
        );
    }

    #[test]
    fn tool_image_capability_mismatch_is_a_hard_error() {
        let error =
            validate_llm_input_capabilities(&[tool_with_image()], &model("compact-provider/text-model"))
                .unwrap_err();
        assert!(error.to_string().contains("required by tool output"));
    }

    #[test]
    fn video_file_is_rejected_without_the_capability() {
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
        let error = validate_llm_input_capabilities(&items, &model("compact-provider/text-model"))
            .unwrap_err();
        assert!(error.to_string().contains("does not support capability 'video'"));
    }

    #[test]
    fn document_file_is_allowed_under_text() {
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
        validate_llm_input_capabilities(&items, &model("compact-provider/text-model")).unwrap();
    }

    #[test]
    fn projection_replaces_unsupported_parts_then_validates() {
        let text_only = model("compact-provider/text-model");
        let mut items = vec![user_with_image()];
        assert!(validate_llm_input_capabilities(&items, &text_only).is_err());
        project_llm_input_for_model(&mut items, &text_only);
        validate_llm_input_capabilities(&items, &text_only).unwrap();
        let Item::Message(MessageItem::Input(message)) = &items[0] else {
            panic!("expected input message");
        };
        assert!(message.content.iter().any(|part| matches!(
            part,
            InputContent::InputText(text) if text.text.contains("does not support image")
        )));
    }

    #[test]
    fn multimodal_model_is_untouched() {
        let multimodal = model("compact-provider/mm");
        let mut items = vec![user_with_image(), tool_with_image()];
        project_llm_input_for_model(&mut items, &multimodal);
        validate_llm_input_capabilities(&items, &multimodal).unwrap();
        assert!(matches!(
            &items[0],
            Item::Message(MessageItem::Input(message))
                if message.content.iter().any(|part| matches!(part, InputContent::InputImage(_)))
        ));
    }

    #[test]
    fn compaction_binding_uses_its_own_provider_and_credentials() {
        let binding = binding_for_agent(&resolved(), &mut ProviderRegistry::new(), "compaction")
            .expect("resolve hidden compaction binding");
        assert_eq!(binding.provider_id, "compact-provider");
        assert_eq!(binding.model_ref, "compact-provider/compact-api-model");
        assert_eq!(
            binding.provider.endpoint(),
            "https://compact.example/v1/responses"
        );
        assert_eq!(binding.api_key, "compact-key");
        assert_eq!(binding.api_model_id, "compact-api-model");
        assert_eq!(binding.max_tokens, 2_048);
        assert_eq!(binding.context_window, 64_000);
    }

    #[test]
    fn a_provider_without_a_key_is_not_ready() {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "compaction".into(),
            AgentProfile {
                role: AgentRole::Hidden,
                model_ref: "compact-provider/compact-api-model".into(),
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve(
            global,
            WorkspaceState::new("/tmp/no-key"),
            catalog(),
        );
        let error = binding_for_agent(&resolved, &mut ProviderRegistry::new(), "compaction")
            .err()
            .expect("no credential");
        assert!(error.to_string().contains("no API key"), "{error}");
    }

    #[test]
    fn an_unknown_reference_is_a_missing_model_error() {
        let error = binding_for_agent(
            &resolved_with("ghost/model"),
            &mut ProviderRegistry::new(),
            "compaction",
        )
        .err()
        .expect("unknown reference");
        assert!(error.to_string().contains("ghost/model"), "{error}");
        assert!(error.to_string().contains("provider-catalog.toml"), "{error}");
    }
}
