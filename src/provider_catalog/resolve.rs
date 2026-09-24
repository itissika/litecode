//! Validated catalog: immutable indexes the runtime resolves against.
//!
//! Built once per process from [super::schema::RawCatalog]. Inheritance
//! (endpoint, protocol, tiers) and default landing happen here and nowhere
//! else, so no runtime path re-derives a value or matches on a provider id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Map;

use crate::types::{LitecodeError, Result};

use super::schema::{
    AuthKind, EndpointKind, Modality, ProviderQuirk, RESERVED_BODY_KEYS, RESERVED_HEADER_NAMES,
    RawCatalog, RawModel, RawProvider, ReasoningKey, ReasoningTiers, SESSION_ID_PLACEHOLDER,
    SUPPORTED_VERSION, UsagePatch,
};

/// A provider after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub endpoint: String,
    pub endpoint_type: EndpointKind,
    pub auth: AuthKind,
    pub tiers: Option<ReasoningTiers>,
    pub quirks: Vec<ProviderQuirk>,
    pub headers: Vec<(String, String)>,
}

/// A model after validation and inheritance - everything a codec needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModel {
    pub provider_id: String,
    pub provider_name: String,
    /// Wire model id sent to the vendor.
    pub id: String,
    /// Stable reference: `{provider_id}/{id}`.
    pub reference: String,
    pub label: String,
    pub endpoint: String,
    pub request_url: String,
    pub endpoint_type: EndpointKind,
    pub auth: AuthKind,
    pub quirks: Vec<ProviderQuirk>,
    pub headers: Vec<(String, String)>,
    pub context_window: usize,
    pub context_window_max: usize,
    pub max_output: u32,
    pub modalities: Vec<Modality>,
    pub tool_call: bool,
    pub json_output: bool,
    pub temperature: bool,
    pub stream_usage: bool,
    pub usage_patch: UsagePatch,
    pub reasoning: Option<ReasoningTiers>,
    /// Literal that means "thinking off" for this model, when the vendor has one.
    pub reasoning_off: Option<String>,
    /// Vendor literal that opts into reasoning summaries, when declared.
    pub reasoning_summary: Option<String>,
    pub reasoning_key: ReasoningKey,
    pub extra_body: Map<String, serde_json::Value>,
}

impl ResolvedModel {
    pub fn supports(&self, modality: Modality) -> bool {
        self.modalities.contains(&modality)
    }

    pub fn has_quirk(&self, quirk: ProviderQuirk) -> bool {
        self.quirks.contains(&quirk)
    }

    /// Display label, falling back to the wire id.
    pub fn display_label(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// The catalog: raw providers/models plus the runtime indexes.
///
/// Immutable after construction; every component shares one `Arc<ProviderCatalog>`.
#[derive(Debug, Clone)]
pub struct ProviderCatalog {
    path: PathBuf,
    providers: Vec<Arc<ResolvedProvider>>,
    models: Vec<Arc<ResolvedModel>>,
    providers_by_id: HashMap<String, Arc<ResolvedProvider>>,
    models_by_ref: HashMap<String, Arc<ResolvedModel>>,
}

impl ProviderCatalog {
    /// Parse and validate catalog text. `path` is used for diagnostics only.
    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        let raw: RawCatalog = toml::from_str(text).map_err(|error| {
            LitecodeError::Config(format!(
                "provider catalog {} is not valid TOML:\n{error}",
                path.display()
            ))
        })?;
        Self::from_raw(raw, path)
    }

    /// Validate an already-deserialized catalog.
    pub fn from_raw(raw: RawCatalog, path: &Path) -> Result<Self> {
        let at = |message: String| {
            LitecodeError::Config(format!("provider catalog {}: {message}", path.display()))
        };

        if raw.version != SUPPORTED_VERSION {
            return Err(at(format!(
                "version {} is not supported by this build (expected {SUPPORTED_VERSION})",
                raw.version
            )));
        }

        let mut providers = Vec::with_capacity(raw.providers.len());
        let mut providers_by_id: HashMap<String, Arc<ResolvedProvider>> = HashMap::new();
        for (index, provider) in raw.providers.iter().enumerate() {
            validate_provider(provider).map_err(|message| {
                at(format!("providers[{index}] ({}): {message}", provider.id))
            })?;
            if providers_by_id.contains_key(&provider.id) {
                return Err(at(format!(
                    "providers[{index}]: duplicate provider id '{}'",
                    provider.id
                )));
            }
            let resolved = Arc::new(resolve_provider(provider));
            providers_by_id.insert(resolved.id.clone(), Arc::clone(&resolved));
            providers.push(resolved);
        }

        let mut models = Vec::with_capacity(raw.models.len());
        let mut models_by_ref: HashMap<String, Arc<ResolvedModel>> = HashMap::new();
        for (index, model) in raw.models.iter().enumerate() {
            let identity = format!("{}/{}", model.provider_id, model.id);
            let provider = providers_by_id.get(&model.provider_id).ok_or_else(|| {
                at(format!(
                    "models[{index}] ({identity}): provider_id '{}' does not exist",
                    model.provider_id
                ))
            })?;
            let resolved = Arc::new(
                resolve_model(model, provider, &identity)
                    .map_err(|message| at(format!("models[{index}] ({identity}): {message}")))?,
            );
            if models_by_ref.contains_key(&resolved.reference) {
                return Err(at(format!(
                    "models[{index}]: duplicate model reference '{}'",
                    resolved.reference
                )));
            }
            models_by_ref.insert(resolved.reference.clone(), Arc::clone(&resolved));
            models.push(resolved);
        }

        Ok(Self {
            path: path.to_path_buf(),
            providers,
            models,
            providers_by_id,
            models_by_ref,
        })
    }

    /// Path this catalog was loaded from (diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Providers in file order.
    pub fn providers(&self) -> &[Arc<ResolvedProvider>] {
        &self.providers
    }

    /// Models in file order.
    pub fn models(&self) -> &[Arc<ResolvedModel>] {
        &self.models
    }

    pub fn provider(&self, provider_id: &str) -> Option<&Arc<ResolvedProvider>> {
        self.providers_by_id.get(provider_id)
    }

    /// Look up by stable reference `{provider_id}/{model_id}`.
    pub fn model(&self, reference: &str) -> Option<&Arc<ResolvedModel>> {
        self.models_by_ref.get(reference)
    }

    /// Models of one provider, in file order.
    pub fn models_of(&self, provider_id: &str) -> Vec<Arc<ResolvedModel>> {
        self.models
            .iter()
            .filter(|model| model.provider_id == provider_id)
            .cloned()
            .collect()
    }

    /// Split a reference at its first `/` (model ids may contain `/`).
    pub fn split_reference(reference: &str) -> Option<(&str, &str)> {
        let (provider, model) = reference.split_once('/')?;
        if provider.is_empty() || model.is_empty() {
            return None;
        }
        Some((provider, model))
    }

    /// Build a reference from a provider id and a wire model id.
    pub fn reference_of(provider_id: &str, model_id: &str) -> String {
        format!("{provider_id}/{model_id}")
    }
}

fn resolve_provider(provider: &RawProvider) -> ResolvedProvider {
    ResolvedProvider {
        id: provider.id.clone(),
        name: provider.name.clone(),
        visible: provider.visible,
        endpoint: normalize_endpoint(&provider.endpoint),
        endpoint_type: provider.endpoint_type,
        auth: provider.auth,
        tiers: provider.tiers.clone(),
        quirks: provider.quirks.clone(),
        headers: provider
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    }
}

fn resolve_model(
    model: &RawModel,
    provider: &ResolvedProvider,
    reference: &str,
) -> std::result::Result<ResolvedModel, String> {
    if model.id.trim().is_empty() {
        return Err("id must not be empty".into());
    }
    let endpoint = match &model.endpoint {
        Some(endpoint) => normalize_endpoint(endpoint),
        None => provider.endpoint.clone(),
    };
    let endpoint_type = model.endpoint_type.unwrap_or(provider.endpoint_type);

    if model.context_window == 0 {
        return Err("context_window must be > 0".into());
    }
    if model.max_output == 0 {
        return Err("max_output must be > 0".into());
    }
    let context_window_max = model.context_window_max.unwrap_or(model.context_window);
    if context_window_max < model.context_window {
        return Err(format!(
            "context_window_max ({context_window_max}) must not be smaller than context_window ({})",
            model.context_window
        ));
    }
    if !model.modalities.contains(&Modality::Text) {
        return Err("modalities must include 'text'".into());
    }
    for modality in &model.modalities {
        if !endpoint_type.supports_modality(*modality) {
            return Err(format!(
                "modality '{}' is not implemented by the {} codec",
                modality.as_str(),
                endpoint_type.as_str()
            ));
        }
    }
    for key in model.extra_body.keys() {
        if RESERVED_BODY_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "extra_body key '{key}' is owned by the {} codec",
                endpoint_type.as_str()
            ));
        }
    }

    let reasoning = match &model.reasoning {
        Some(raw) => raw.tiers.clone().or_else(|| provider.tiers.clone()),
        None => provider.tiers.clone(),
    };
    if let Some(tiers) = &reasoning {
        validate_tiers(tiers)?;
    }
    let reasoning_off = reasoning.as_ref().and_then(|tiers| tiers.off.clone());
    let reasoning_summary = model.reasoning.as_ref().and_then(|raw| raw.summary.clone());
    if let Some(summary) = &reasoning_summary {
        if summary.trim().is_empty() {
            return Err("reasoning.summary must not be empty when present".into());
        }
        if reasoning.is_none() {
            return Err(
                "reasoning.summary requires a reasoning tier mapping (model or provider)".into(),
            );
        }
    }

    Ok(ResolvedModel {
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        id: model.id.clone(),
        reference: reference.to_string(),
        label: model.label.clone().unwrap_or_default(),
        endpoint: endpoint.clone(),
        request_url: endpoint_type.request_url(&endpoint),
        endpoint_type,
        auth: provider.auth,
        quirks: provider.quirks.clone(),
        headers: provider.headers.clone(),
        context_window: model.context_window,
        context_window_max,
        max_output: model.max_output,
        modalities: model.modalities.clone(),
        tool_call: model.tool_call,
        json_output: model.json_output,
        temperature: model.temperature,
        stream_usage: model.stream_usage,
        usage_patch: model.usage_patch,
        reasoning,
        reasoning_off,
        reasoning_summary,
        reasoning_key: model
            .reasoning
            .as_ref()
            .map(|raw| raw.key)
            .unwrap_or_default(),
        extra_body: model.extra_body.clone(),
    })
}

/// Everything a provider must satisfy, independent of its models.
pub fn validate_provider(provider: &RawProvider) -> std::result::Result<(), String> {
    if !is_valid_provider_id(&provider.id) {
        return Err(format!(
            "id '{}' must match [a-z0-9][a-z0-9_-]* and must not contain '/'",
            provider.id
        ));
    }
    if provider.name.trim().is_empty() {
        return Err("name must not be empty".into());
    }
    validate_endpoint(&provider.endpoint)?;
    if let Some(tiers) = &provider.tiers {
        validate_tiers(tiers)?;
    }
    validate_headers(&provider.headers)?;
    if provider.quirks.contains(&ProviderQuirk::ThinkingTypeSwitch)
        && provider.endpoint_type != EndpointKind::Responses
    {
        return Err(format!(
            "quirk '{}' is only implemented by the responses codec",
            "thinking_type_switch"
        ));
    }
    Ok(())
}

/// Provider ids are the left half of a model reference, so `/` is illegal.
pub fn is_valid_provider_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Absolute http(s) URL, no query/fragment, and a bare base path.
pub fn validate_endpoint(endpoint: &str) -> std::result::Result<(), String> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err("endpoint must not be empty".into());
    }
    let url = url::Url::parse(trimmed)
        .map_err(|error| format!("endpoint '{trimmed}' is not a URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "endpoint '{trimmed}' must use http or https (got '{}')",
            url.scheme()
        ));
    }
    if url.host_str().is_none() {
        return Err(format!("endpoint '{trimmed}' has no host"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "endpoint '{trimmed}' must not carry a query or fragment"
        ));
    }
    let path = url.path().trim_end_matches('/');
    for suffix in ["/responses", "/chat/completions"] {
        if path.ends_with(suffix) {
            return Err(format!(
                "endpoint '{trimmed}' must be the API base without '{suffix}'"
            ));
        }
    }
    Ok(())
}

fn validate_tiers(tiers: &ReasoningTiers) -> std::result::Result<(), String> {
    if let Some(off) = &tiers.off {
        if off.trim().is_empty() {
            return Err("tiers.off must not be empty when present".into());
        }
    }
    for (slot, literal) in [
        ("low", &tiers.low),
        ("medium", &tiers.medium),
        ("high", &tiers.high),
    ] {
        if literal.trim().is_empty() {
            return Err(format!("tiers.{slot} must not be empty"));
        }
    }
    Ok(())
}

/// Header names must be valid tokens; values may only use `{{session_id}}`.
pub fn validate_headers(
    headers: &std::collections::BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    for (name, value) in headers {
        if name.trim().is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err(format!("header name '{name}' is not a valid HTTP token"));
        }
        if RESERVED_HEADER_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(format!("header '{name}' is owned by the codec"));
        }
        let mut rest = value.as_str();
        while let Some(index) = rest.find("{{") {
            let tail = &rest[index..];
            let Some(end) = tail.find("}}") else {
                return Err(format!("header '{name}' has an unterminated template"));
            };
            let placeholder = &tail[..end + 2];
            if placeholder != SESSION_ID_PLACEHOLDER {
                return Err(format!(
                    "header '{name}' uses unsupported placeholder '{placeholder}' \
                     (only {SESSION_ID_PLACEHOLDER} is allowed)"
                ));
            }
            rest = &tail[end + 2..];
        }
    }
    Ok(())
}

fn normalize_endpoint(endpoint: &str) -> String {
    endpoint.trim().trim_end_matches('/').to_string()
}
