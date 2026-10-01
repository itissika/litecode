//! Startup upgrade for a provider catalog this build cannot load.
//!
//! The strict contract in [super::schema] does not change. This module runs
//! only after that parse fails, and it does not translate old fields into new
//! ones:
//!
//! - a key or enum value this build does not know is dropped;
//! - a provider or model that still cannot load is replaced by the embedded
//!   seed entry with the same id, or dropped when the seed has no such id;
//! - a document that is TOML but still cannot load after that is replaced by
//!   the embedded catalog;
//! - text that is not TOML is left alone, and the original parse error stands.
//!
//! A file that already loads is never passed here, so comments on a valid
//! catalog survive.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use toml::Value;

use crate::types::Result;

use super::ProviderCatalog;
use super::schema::{Modality, ProviderQuirk, SUPPORTED_VERSION};
use super::store::DEFAULT_CATALOG;

const TOP_LEVEL: &[&str] = &["version", "providers", "models"];
const PROVIDER_KEYS: &[&str] = &[
    "id",
    "name",
    "visible",
    "endpoint",
    "endpoint_type",
    "auth",
    "tiers",
    "quirks",
    "headers",
];
const MODEL_KEYS: &[&str] = &[
    "id",
    "provider_id",
    "label",
    "endpoint",
    "endpoint_type",
    "quirks",
    "context_window",
    "context_window_max",
    "max_output",
    "modalities",
    "tool_call",
    "json_output",
    "stream_usage",
    "usage_patch",
    "reasoning",
    "extra_body",
];
const TIER_KEYS: &[&str] = &["off", "low", "medium", "high"];
const REASONING_KEYS: &[&str] = &["tiers", "summary", "key", "replay"];

/// A catalog this build can load, plus the file text to persist.
pub struct UpgradedCatalog {
    pub catalog: ProviderCatalog,
    pub text: String,
}

/// `None` when `text` is not TOML. Otherwise a catalog this build accepts.
pub fn upgrade(text: &str, path: &Path) -> Option<UpgradedCatalog> {
    let mut document: Value = match toml::from_str(text) {
        Ok(Value::Table(table)) => Value::Table(table),
        Ok(_) => return Some(embedded(path)),
        Err(_) => return None,
    };
    let seed = seed_document();
    let seed_providers = index_providers(array_of(seed, "providers"));
    let seed_models = index_models(array_of(seed, "models"));

    retain_keys(document.as_table_mut()?, TOP_LEVEL);
    let table = document.as_table_mut()?;
    table.insert(
        "version".to_string(),
        Value::Integer(i64::from(SUPPORTED_VERSION)),
    );
    if !is_entry_array(table, "providers") || !is_entry_array(table, "models") {
        tracing::warn!(
            "provider catalog providers/models were not arrays; replacing the file with the embedded default"
        );
        return Some(embedded(path));
    }

    let providers = take_entries(table, "providers");
    let mut kept_providers = Vec::new();
    let mut seen_providers = HashSet::new();
    for entry in providers {
        let Some(provider) = salvage_provider(entry, &seed_providers) else {
            continue;
        };
        let Some(id) = string_field(&provider, "id") else {
            continue;
        };
        if !seen_providers.insert(id.clone()) {
            tracing::warn!(provider_id = %id, "dropped duplicate provider during catalog upgrade");
            continue;
        }
        kept_providers.push(provider);
    }
    let providers_by_id: HashMap<String, Value> = kept_providers
        .iter()
        .filter_map(|provider| string_field(provider, "id").map(|id| (id, provider.clone())))
        .collect();

    let models = take_entries(table, "models");
    let mut kept_models = Vec::new();
    let mut seen_models = HashSet::new();
    for entry in models {
        let Some(model) = salvage_model(entry, &providers_by_id, &seed_models) else {
            continue;
        };
        let Some(reference) = model_reference(&model) else {
            continue;
        };
        if !seen_models.insert(reference.clone()) {
            tracing::warn!(model = %reference, "dropped duplicate model during catalog upgrade");
            continue;
        }
        kept_models.push(model);
    }

    table.insert("providers".to_string(), Value::Array(kept_providers));
    table.insert("models".to_string(), Value::Array(kept_models));

    match render(&document, path) {
        Ok(upgraded) => Some(upgraded),
        Err(error) => {
            tracing::warn!(
                %error,
                "provider catalog upgrade could not be loaded; replacing it with the embedded default"
            );
            Some(embedded(path))
        }
    }
}

fn salvage_provider(mut value: Value, seed: &HashMap<String, Value>) -> Option<Value> {
    let id = string_field(&value, "id");
    scrub_provider(&mut value);
    if provider_loads(&value) {
        return Some(value);
    }
    let id = match id {
        Some(id) => id,
        None => {
            tracing::warn!("dropped provider entry with no id during catalog upgrade");
            return None;
        }
    };
    let Some(replaced) = seed.get(&id).cloned() else {
        tracing::warn!(
            provider_id = %id,
            "dropped incompatible provider; this build has no default with that id"
        );
        return None;
    };
    if provider_loads(&replaced) {
        tracing::warn!(
            provider_id = %id,
            "replaced incompatible provider with the embedded default"
        );
        return Some(replaced);
    }
    tracing::warn!(
        provider_id = %id,
        "dropped provider; the embedded default did not load"
    );
    None
}

fn salvage_model(
    mut value: Value,
    providers: &HashMap<String, Value>,
    seed: &HashMap<String, Value>,
) -> Option<Value> {
    let reference = model_reference(&value);
    scrub_model(&mut value);
    if model_loads(&value, providers) {
        return Some(value);
    }
    let Some(reference) = reference else {
        tracing::warn!("dropped model entry with no provider_id/id during catalog upgrade");
        return None;
    };
    let Some(replaced) = seed.get(&reference).cloned() else {
        tracing::warn!(
            model = %reference,
            "dropped incompatible model; this build has no default with that id"
        );
        return None;
    };
    if model_loads(&replaced, providers) {
        tracing::warn!(
            model = %reference,
            "replaced incompatible model with the embedded default"
        );
        return Some(replaced);
    }
    tracing::warn!(
        model = %reference,
        "dropped model that does not load against its provider"
    );
    None
}

fn scrub_provider(value: &mut Value) {
    let Some(table) = value.as_table_mut() else {
        return;
    };
    retain_keys(table, PROVIDER_KEYS);
    if let Some(tiers) = table.get_mut("tiers") {
        scrub_tiers(tiers);
    }
    drop_unknown_names(table, "quirks", |name| ProviderQuirk::parse(name).is_some());
}

fn scrub_model(value: &mut Value) {
    let Some(table) = value.as_table_mut() else {
        return;
    };
    retain_keys(table, MODEL_KEYS);
    drop_unknown_names(table, "quirks", |name| ProviderQuirk::parse(name).is_some());
    drop_unknown_names(table, "modalities", |name| Modality::parse(name).is_some());
    if let Some(reasoning) = table.get_mut("reasoning") {
        scrub_reasoning(reasoning);
    }
}

fn scrub_reasoning(value: &mut Value) {
    let Some(table) = value.as_table_mut() else {
        return;
    };
    retain_keys(table, REASONING_KEYS);
    if let Some(tiers) = table.get_mut("tiers") {
        scrub_tiers(tiers);
    }
}

fn scrub_tiers(value: &mut Value) {
    let Some(table) = value.as_table_mut() else {
        return;
    };
    retain_keys(table, TIER_KEYS);
}

/// Drop enum values this build does not know. An array that becomes empty had
/// only unknown values, so the key is removed and the field's default applies.
/// An array that was already empty is kept: on a model, `quirks = []` clears
/// the inherited list.
fn drop_unknown_names(
    table: &mut toml::map::Map<String, Value>,
    key: &str,
    known: fn(&str) -> bool,
) {
    let remove = {
        let Some(value) = table.get_mut(key) else {
            return;
        };
        let Some(items) = value.as_array_mut() else {
            return;
        };
        let before = items.len();
        items.retain(|item| item.as_str().is_some_and(known));
        before > 0 && items.is_empty()
    };
    if remove {
        table.remove(key);
    }
}

fn retain_keys(table: &mut toml::map::Map<String, Value>, allowed: &[&str]) {
    table.retain(|key, _| allowed.contains(&key));
}

fn is_entry_array(table: &toml::map::Map<String, Value>, key: &str) -> bool {
    matches!(table.get(key), None | Some(Value::Array(_)))
}

fn take_entries(table: &mut toml::map::Map<String, Value>, key: &str) -> Vec<Value> {
    match table.remove(key) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

fn provider_loads(provider: &Value) -> bool {
    provider.is_table() && loads(std::slice::from_ref(provider), &[])
}

fn model_loads(model: &Value, providers: &HashMap<String, Value>) -> bool {
    let Some(provider_id) = string_field(model, "provider_id") else {
        return false;
    };
    let Some(provider) = providers.get(&provider_id) else {
        return false;
    };
    model.is_table() && loads(std::slice::from_ref(provider), std::slice::from_ref(model))
}

fn loads(providers: &[Value], models: &[Value]) -> bool {
    let mut table = toml::map::Map::new();
    table.insert(
        "version".to_string(),
        Value::Integer(i64::from(SUPPORTED_VERSION)),
    );
    table.insert("providers".to_string(), Value::Array(providers.to_vec()));
    table.insert("models".to_string(), Value::Array(models.to_vec()));
    render(
        &Value::Table(table),
        Path::new("<provider-catalog-upgrade>"),
    )
    .is_ok()
}

fn render(document: &Value, path: &Path) -> Result<UpgradedCatalog> {
    let text = toml::to_string(document).map_err(|error| {
        crate::types::LitecodeError::Config(format!(
            "provider catalog upgrade could not be serialized: {error}"
        ))
    })?;
    let catalog = ProviderCatalog::parse(&text, path)?;
    Ok(UpgradedCatalog { catalog, text })
}

fn embedded(path: &Path) -> UpgradedCatalog {
    let catalog =
        ProviderCatalog::parse(DEFAULT_CATALOG, path).expect("embedded provider catalog is valid");
    UpgradedCatalog {
        catalog,
        text: DEFAULT_CATALOG.to_string(),
    }
}

fn seed_document() -> &'static Value {
    static SEED: OnceLock<Value> = OnceLock::new();
    SEED.get_or_init(|| toml::from_str(DEFAULT_CATALOG).expect("embedded provider catalog is TOML"))
}

fn array_of<'a>(document: &'a Value, key: &str) -> &'a [Value] {
    document
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn index_providers(entries: &[Value]) -> HashMap<String, Value> {
    entries
        .iter()
        .filter_map(|entry| string_field(entry, "id").map(|id| (id, entry.clone())))
        .collect()
}

fn index_models(entries: &[Value]) -> HashMap<String, Value> {
    entries
        .iter()
        .filter_map(|entry| model_reference(entry).map(|reference| (reference, entry.clone())))
        .collect()
}

fn model_reference(value: &Value) -> Option<String> {
    let provider_id = string_field(value, "provider_id")?;
    let id = string_field(value, "id")?;
    if provider_id.is_empty() || id.is_empty() {
        return None;
    }
    Some(ProviderCatalog::reference_of(&provider_id, &id))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .as_table()?
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::schema::ProviderQuirk;
    use super::upgrade;

    fn upgraded(text: &str) -> super::UpgradedCatalog {
        upgrade(text, Path::new("upgrade-test.toml")).expect("toml should upgrade")
    }

    #[test]
    fn syntax_errors_are_not_rewritten() {
        assert!(upgrade("version = [\n", Path::new("x.toml")).is_none());
        assert!(upgrade("42", Path::new("x.toml")).is_none());
    }

    #[test]
    fn unknown_fields_and_quirks_are_dropped_and_edits_stay() {
        let text = r#"
version = 1
note = "ignore me"

[[providers]]
id = "deepseek"
name = "DeepSeek"
endpoint = "https://proxy.example/v1"
endpoint_type = "responses"
auth = "bearer"
quirks = ["omit_temperature_when_thinking", "reasoning_replay"]

[[providers]]
id = "mine"
name = "Mine"
endpoint = "https://example.com/v1"
endpoint_type = "responses"

[[models]]
id = "deepseek-flash"
provider_id = "deepseek"
label = "My Flash"
temperature = false
modalities = ["text", "telepathy"]
quirks = ["omit_temperature_when_thinking"]
"#;
        let upgraded = upgraded(text);
        let deepseek = upgraded.catalog.provider("deepseek").unwrap();
        assert_eq!(deepseek.endpoint, "https://proxy.example/v1");
        assert!(deepseek.quirks == vec![ProviderQuirk::ReasoningReplay]);
        assert!(upgraded.catalog.provider("mine").is_some());
        let model = upgraded.catalog.model("deepseek/deepseek-flash").unwrap();
        assert_eq!(model.label, "My Flash");
        assert!(model.has_quirk(ProviderQuirk::ReasoningReplay));
        assert!(!model.supports(super::super::schema::Modality::Image));
        assert!(!upgraded.text.contains("omit_temperature"));
        assert!(!upgraded.text.contains("temperature"));
        assert!(!upgraded.text.contains("telepathy"));
        assert!(!upgraded.text.contains("note"));
    }

    #[test]
    fn summary_replay_survives_an_upgrade() {
        let text = r#"
version = 1

[[providers]]
id = "p"
name = "P"
endpoint = "https://x.example/v1"
endpoint_type = "responses"

[[models]]
id = "m"
provider_id = "p"
reasoning = { replay = "summary", tiers = { low = "low", medium = "medium", high = "high" }, dropped = true }
"#;
        let upgraded = upgraded(text);
        let model = upgraded.catalog.model("p/m").unwrap();
        assert_eq!(
            model.reasoning_replay,
            super::super::schema::ReasoningReplay::Summary
        );
        assert!(upgraded.text.contains("replay"));
        assert!(!upgraded.text.contains("dropped"));
    }

    #[test]
    fn an_explicit_empty_quirk_list_still_clears_inheritance() {
        let text = r#"
version = 1

[[providers]]
id = "deepseek"
name = "DeepSeek"
endpoint = "https://api.deepseek.com"
endpoint_type = "responses"
quirks = ["reasoning_replay"]

[[models]]
id = "deepseek-flash"
provider_id = "deepseek"
quirks = []
"#;
        let upgraded = upgraded(text);
        let model = upgraded.catalog.model("deepseek/deepseek-flash").unwrap();
        assert!(model.quirks.is_empty());
    }

    #[test]
    fn an_incompatible_known_id_is_replaced_by_the_seed() {
        let text = r#"
version = 1

[[providers]]
id = "deepseek"
name = "DeepSeek"
endpoint = "not a url"
endpoint_type = "responses"

[[models]]
id = "deepseek-flash"
provider_id = "deepseek"
label = "gone"
context_window = 0

[[models]]
id = "custom-only"
provider_id = "deepseek"
context_window = 0
"#;
        let upgraded = upgraded(text);
        assert_eq!(
            upgraded.catalog.provider("deepseek").unwrap().endpoint,
            "https://api.deepseek.com"
        );
        assert_eq!(
            upgraded
                .catalog
                .model("deepseek/deepseek-flash")
                .unwrap()
                .label,
            "DeepSeek Flash"
        );
        assert!(upgraded.catalog.model("deepseek/custom-only").is_none());
    }

    #[test]
    fn a_document_whose_tables_are_not_arrays_is_replaced_by_the_embedded_catalog() {
        let upgraded = upgraded("providers = 1\n");
        assert!(upgraded.catalog.provider("openai").is_some());
        assert!(upgraded.catalog.model("deepseek/deepseek-flash").is_some());
        assert!(upgraded.text.contains("# LiteCode provider catalog"));
    }
}
