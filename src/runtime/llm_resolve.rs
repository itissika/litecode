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
    /// Freeze the main request shape for a compaction call mode.
    ///
    /// The caller supplies the instructions and active tools from the same
    /// turn; provider/model/target all come from this binding. No hidden
    /// compaction binding is resolved here or anywhere downstream.
    pub fn compaction_context(
        &self,
        instructions: String,
        tools: Vec<crate::llm::ToolDef>,
        session_id: String,
    ) -> crate::llm::CompactionRequestContext {
        crate::llm::CompactionRequestContext {
            provider: Arc::clone(&self.provider),
            api_key: self.api_key.clone(),
            model: self.api_model_id.clone(),
            context_window: self.context_window,
            target: Arc::clone(&self.model),
            thinking: crate::platform_knobs::ThinkingSpec::Tier(self.thinking_tier),
            max_output_tokens: self.max_tokens,
            instructions,
            tools,
            session_id,
        }
    }
}

const MISSING_MODEL_HINT: &str = "open Settings → Providers and configure a provider API key, then pick a model for the agent \
     in Settings → Agents";

/// The model a session should start on: the agent's declared model when it is
/// usable, else the first model a user can run right now.
///
/// `None` means no model is selectable at all (no provider holds a credential);
/// the turn then reports the usual pick-a-model error.
pub fn seed_model_ref(resolved: &ResolvedConfig, agent_id: &str) -> Option<String> {
    resolved
        .model_for_agent(agent_id)
        .map(|model| model.reference.clone())
        .or_else(|| resolved.fallback_model_ref())
}

/// Keep a session runnable before its next turn reads it.
///
/// A session's `model_id` is the only model fact a turn resolves (see
/// [`resolve_session_llm`]), so an empty one (agent had none when the session
/// was opened), a lost one (model dropped from the catalog) or an unusable one
/// (provider key removed) is replaced in place — the catalog is the source of
/// truth and the stored ref is derived state. The agent's declared model wins
/// over the generic fallback when it is usable.
///
/// This writes `sessions.db`, which is not turn-guarded: unlike settings writes
/// (global DB), it succeeds while another session is running. Returns the model
/// the session should run with, or `None` when nothing is selectable.
pub fn ensure_session_model(
    resolved: &ResolvedConfig,
    sessions: &SessionManager,
    session_id: &str,
    agent_id: &str,
) -> Option<String> {
    let current = sessions.session_model_id(session_id);
    if current
        .as_deref()
        .is_some_and(|reference| resolved.model_for_agent_ref(reference).is_some())
    {
        return current;
    }
    let model_ref = seed_model_ref(resolved, agent_id)?;
    if current.as_deref() != Some(model_ref.as_str()) {
        match sessions.set_session_model_id(session_id, Some(model_ref.clone())) {
            Ok(()) => tracing::info!(
                session_id,
                model_ref = %model_ref,
                "auto-assigned a runnable model to a session whose model could not run"
            ),
            Err(error) => tracing::warn!(
                session_id,
                model_ref = %model_ref,
                %error,
                "could not persist the auto-assigned session model"
            ),
        }
    }
    Some(model_ref)
}

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
            "model '{model_ref}' is not in this build's provider catalog. {MISSING_MODEL_HINT}"
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

/// Read the producer of each item from durable request headers.
///
/// One definition shared by the request builder and the switch preview, so a
/// preview and the next turn cannot disagree about who produced what.
pub(crate) fn producers_from_origins(
    origins: Vec<(crate::session::event::Seq, serde_json::Value)>,
    item_seqs: &[Option<crate::session::event::Seq>],
) -> Vec<Option<crate::llm::ItemProducer>> {
    let headers = origins
        .into_iter()
        .map(|(seq, body)| {
            let field = |key: &str| {
                body.get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            (
                seq,
                crate::llm::ItemProducer {
                    provider_id: field("provider_id").unwrap_or_default(),
                    endpoint_type: field("endpoint_type"),
                    model_ref: field("model_ref"),
                },
            )
        })
        .collect::<Vec<_>>();
    crate::llm::producers_for_seqs(item_seqs, &headers)
}

/// What a model switch would cost, computed before the switch is persisted.
#[derive(Debug, Clone)]
pub(crate) struct SwitchPreview {
    pub report: crate::llm::port::PortReport,
    /// Highest durable seq at preview time. Later items did not exist when the
    /// user switched and are not part of this report.
    pub at_seq: crate::session::event::Seq,
}

/// Port the current committed view to `model_ref` without sending anything.
///
/// This is the same policy function the next turn runs; only the input is a
/// preview (no turn-specific padding or reminders). A blocked target must not
/// be persisted as the session model.
pub(crate) fn preview_session_model(
    resolved: &ResolvedConfig,
    sessions: &SessionManager,
    session_id: &str,
    model_ref: &str,
) -> Result<SwitchPreview> {
    let target = resolved
        .catalog()
        .model(model_ref)
        .cloned()
        .ok_or_else(|| {
            LitecodeError::Config(format!(
                "model '{model_ref}' is not in this build's provider catalog"
            ))
        })?;
    let rows = sessions.data().working_set_blocking(session_id)?;
    let seqs: Vec<Option<crate::session::event::Seq>> =
        rows.iter().map(|row| row.log_seq).collect();
    let mut items: Vec<Item> = rows.into_iter().map(|row| row.item).collect();
    let origins = sessions.request_origins(session_id).unwrap_or_default();
    let producers = producers_from_origins(origins, &seqs);
    let rebased = vec![false; items.len()];
    let projected = project_llm_input_for_model(&mut items, target.as_ref());
    let mut report = crate::llm::port::port(
        &mut items,
        &producers,
        &rebased,
        target.as_ref(),
        crate::platform_knobs::ThinkingSpec::Tier(
            sessions.thinking_tier(session_id).unwrap_or_default(),
        ),
        // The agent loop offers tools; a target that cannot take them cannot
        // be a session model. This is the same predicate the request uses.
        true,
    );
    report.projected_parts = projected;
    Ok(SwitchPreview {
        report,
        at_seq: sessions.entry_wire_seq_cursor(session_id).0.max(0) as u64,
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
pub fn project_llm_input_for_model(items: &mut [Item], model: &ResolvedModel) -> u32 {
    let mut projected = 0u32;
    for item in items.iter_mut() {
        match item {
            Item::Message(MessageItem::Input(message)) => {
                for part in message.content.iter_mut() {
                    if let Some(text) = omit_unsupported_part(part, model) {
                        *part = InputContent::InputText(InputTextContent { text });
                        projected += 1;
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
                        projected += 1;
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
    projected
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
            Some(omit_note(
                &model.reference,
                "image",
                "image",
                location.as_str(),
            ))
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
mod tests {}
