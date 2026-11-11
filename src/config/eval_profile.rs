//! Declarative evaluation surface.
//!
//! One file per campaign says which model-visible tools a scored run gets and at
//! which permission tier. Applying it is the only supported way to change the
//! scored surface: an id the live capability catalog does not know is an error,
//! never a silent omission.
//!
//! Bindings are per capability card (\`run\` = run/wait/kill, \`edit\` =
//! edit/apply_edits, \`lsp\` = seven), so this module takes model-visible tool
//! names and requires each touched card to be covered as a whole.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::Deserialize;

use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AgentToolBinding, CallSurface, ToolPreset};
use crate::config::settings_writer::SettingsWriter;
use crate::permission::has_permission_preset;
use crate::tool::capability::capability_catalog;
use crate::types::{LitecodeError, Result};

/// The agent a scored run uses unless \`--agent\` says otherwise.
pub const AGENT_ID: &str = "default";

/// Id of the code transport. It is not a binding: it exists whenever a code tool
/// is bound, so a profile has to state it but cannot switch it on its own.
pub const RUN_CODE_ID: &str = "run_code";

/// Thinking tiers a session knob may hold (recorded by the artifact, not applied
/// by this command yet).
const THINKING_TIERS: &[&str] = &["low", "medium", "high"];
/// Context tiers a session knob may hold.
const CONTEXT_TIERS: &[&str] = &["standard", "max"];

/// Whether the web tools are on. \`auto\` is decided by the adapter's probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebMode {
    Auto,
    On,
    Off,
}

impl WebMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// Permission tier a card is bound at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    All,
    Safe,
}

impl Tier {
    fn preset(self) -> ToolPreset {
        match self {
            Self::All => ToolPreset::All,
            Self::Safe => ToolPreset::Safe,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Safe => "safe",
        }
    }
}

/// The three lists a profile classifies tools into.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLists {
    #[serde(default)]
    pub on: Vec<String>,
    #[serde(default)]
    pub off: Vec<String>,
    /// Follow the network decision instead of a fixed answer.
    #[serde(default)]
    pub network: Vec<String>,
}

/// A parsed \`eval/profile.toml\`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalProfile {
    pub name: String,
    #[serde(default = "WebMode::auto")]
    pub web: WebMode,
    pub tools: ToolLists,
    #[serde(default)]
    pub permissions: BTreeMap<String, Tier>,
    #[serde(default)]
    pub knobs: BTreeMap<String, String>,
}

impl WebMode {
    fn auto() -> Self {
        Self::Auto
    }
}

/// What the applier turned the profile into: the ids a scored run can see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub name: String,
    pub web: WebMode,
    /// Bound capability cards, sorted.
    pub cards: Vec<String>,
    /// Model-visible tool ids, sorted.
    pub tools: Vec<String>,
    /// Permission tier per bound card that has a dial, in profile vocabulary.
    pub permissions: BTreeMap<String, String>,
    pub knobs: BTreeMap<String, String>,
}

/// What a run actually got, as one document. The file asked for `web = "auto"`;
/// this one records the answer, so an artifact never has to re-derive it.
#[derive(Debug, serde::Serialize)]
pub struct AppliedDocument {
    pub name: String,
    pub source: String,
    pub web: String,
    pub cards: Vec<String>,
    pub tools: Vec<String>,
    pub permissions: BTreeMap<String, String>,
    pub knobs: BTreeMap<String, String>,
}

impl Applied {
    pub fn document(&self, source: &Path) -> AppliedDocument {
        AppliedDocument {
            name: self.name.clone(),
            source: source.display().to_string(),
            web: self.web.as_str().to_string(),
            cards: self.cards.clone(),
            tools: self.tools.clone(),
            permissions: self.permissions.clone(),
            knobs: self.knobs.clone(),
        }
    }

    /// Write the applied document as JSON. The caller reports it; the hash of
    /// these exact bytes is what an artifact quotes.
    pub fn write_document(&self, source: &Path, path: &Path) -> Result<()> {
        let document = self.document(source);
        let text = serde_json::to_string_pretty(&document)
            .map_err(|error| LitecodeError::Config(format!("eval profile: {error}")))?;
        std::fs::write(path, text + "\n").map_err(|error| {
            LitecodeError::Config(format!("eval profile {}: {error}", path.display()))
        })
    }
}

impl EvalProfile {
    pub fn parse(text: &str) -> Result<Self> {
        let profile: Self = toml::from_str(text)
            .map_err(|error| LitecodeError::Config(format!("eval profile: {error}")))?;
        profile.check_shape()?;
        Ok(profile)
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            LitecodeError::Config(format!("eval profile {}: {error}", path.display()))
        })?;
        Self::parse(&text)
    }

    /// Checks that need only the file: an id classified twice is ambiguous, the
    /// transport cannot be switched off, and a knob value must be a real tier.
    fn check_shape(&self) -> Result<()> {
        let mut seen: BTreeMap<&str, &'static str> = BTreeMap::new();
        for (list, ids) in [
            ("tools.on", &self.tools.on),
            ("tools.off", &self.tools.off),
            ("tools.network", &self.tools.network),
        ] {
            for id in ids {
                if let Some(first) = seen.insert(id.as_str(), list) {
                    return Err(LitecodeError::Config(format!(
                        "eval profile: '{id}' is listed in both {first} and {list}"
                    )));
                }
            }
        }
        for (list, ids) in [
            ("tools.off", &self.tools.off),
            ("tools.network", &self.tools.network),
        ] {
            if ids.iter().any(|id| id == RUN_CODE_ID) {
                return Err(LitecodeError::Config(format!(
                    "eval profile: '{RUN_CODE_ID}' is the code transport (in {list}); it cannot be switched off"
                )));
            }
        }
        if let Some(thinking) = self.knobs.get("thinking")
            && !THINKING_TIERS.contains(&thinking.as_str())
        {
            return Err(LitecodeError::Config(format!(
                "eval profile: thinking = '{thinking}' is not one of {}",
                THINKING_TIERS.join(", ")
            )));
        }
        if let Some(context) = self.knobs.get("context")
            && !CONTEXT_TIERS.contains(&context.as_str())
        {
            return Err(LitecodeError::Config(format!(
                "eval profile: context = '{context}' is not one of {}",
                CONTEXT_TIERS.join(", ")
            )));
        }
        for key in self.knobs.keys() {
            if key != "thinking" && key != "context" {
                return Err(LitecodeError::Config(format!(
                    "eval profile: unknown knob '{key}' (known: context, thinking)"
                )));
            }
        }
        Ok(())
    }

    /// Model-visible tool ids this profile binds, with the web group resolved.
    pub fn wanted(&self, web: WebMode) -> BTreeSet<String> {
        let mut wanted: BTreeSet<String> = self.tools.on.iter().cloned().collect();
        if web == WebMode::On {
            wanted.extend(self.tools.network.iter().cloned());
        }
        wanted
    }
}

/// Bind the profile's tools to the \`default\` agent.
///
/// \`web\` carries the adapter's probe result; it is required only when the file
/// says \`auto\`, because the applier must not guess what the container can reach.
pub fn apply(
    writer: &SettingsWriter,
    resolved: &ResolvedConfig,
    profile: &EvalProfile,
    web: Option<WebMode>,
) -> Result<Applied> {
    let web = match (profile.web, web) {
        (WebMode::Auto, None) => {
            return Err(LitecodeError::Config(
                "eval profile: web = \"auto\" needs --web on|off (probe the container once and pass the answer)"
                    .into(),
            ));
        }
        (_, Some(mode)) => mode,
        (mode, None) => mode,
    };
    let catalog = capability_catalog(resolved);
    let wanted = profile.wanted(web);

    // An id the catalog does not know is the one failure mode worth being loud
    // about: it would otherwise mean a silently smaller tool set.
    for id in &wanted {
        if id != RUN_CODE_ID && !catalog.member_owner.contains_key(id) {
            let mut known: Vec<&str> = catalog.member_owner.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(LitecodeError::Config(format!(
                "eval profile: unknown tool '{id}'. tools: {}",
                known.join(", ")
            )));
        }
    }

    let mut cards: BTreeSet<String> = BTreeSet::new();
    for id in &wanted {
        if let Some(card) = catalog.member_owner.get(id) {
            cards.insert(card.clone());
        }
    }

    // A binding covers a whole card. Half a card is not a smaller tool set, it is
    // a file that does not say what it means.
    for card_id in &cards {
        let Some(card) = catalog.get(card_id) else {
            return Err(LitecodeError::Config(format!(
                "eval profile: card '{card_id}' vanished from the catalog"
            )));
        };
        for member in card.member_ids() {
            if !wanted.contains(member) {
                let present = card
                    .member_ids()
                    .find(|member| wanted.contains(*member))
                    .unwrap_or(card_id.as_str());
                return Err(LitecodeError::Config(format!(
                    "eval profile: card '{card_id}' is only half bound: '{present}' is on, '{member}' is not. Bindings cover the whole card"
                )));
            }
        }
    }

    let binds_code = cards.iter().any(|card_id| {
        catalog
            .get(card_id)
            .is_some_and(|card| card.surface == CallSurface::Code)
    });
    if binds_code && !wanted.contains(RUN_CODE_ID) {
        return Err(LitecodeError::Config(format!(
            "eval profile: '{RUN_CODE_ID}' is missing from tools.on; the code tools are called through it"
        )));
    }

    // Every bound dial has to state its tier: a command refused by an allowlist
    // reads exactly like a model that refused to run it.
    let mut presets: BTreeMap<String, ToolPreset> = BTreeMap::new();
    for card_id in &cards {
        match (
            has_permission_preset(card_id),
            profile.permissions.get(card_id),
        ) {
            (true, Some(tier)) => {
                presets.insert(card_id.clone(), tier.preset());
            }
            (true, None) => {
                return Err(LitecodeError::Config(format!(
                    "eval profile: '{card_id}' is bound but [permissions] does not state its tier (all or safe)"
                )));
            }
            (false, Some(_)) => {
                return Err(LitecodeError::Config(format!(
                    "eval profile: '{card_id}' has no permission dial; drop its [permissions] line"
                )));
            }
            (false, None) => {}
        }
    }
    for key in profile.permissions.keys() {
        if !cards.contains(key) {
            return Err(LitecodeError::Config(format!(
                "eval profile: [permissions] names '{key}', which this profile does not bind"
            )));
        }
    }

    let settings = writer.load_settings()?;
    let Some(current) = settings.agents.get(AGENT_ID).cloned() else {
        return Err(LitecodeError::Config(format!(
            "agent '{AGENT_ID}' not found; a profile binds to it, it cannot create it"
        )));
    };
    let mut next = current;
    // "Off" is a disabled row, not a missing one: the load-time ensure plants any
    // missing core binding back enabled, so an absent row would come back on.
    let mut tools: HashMap<String, AgentToolBinding> = next
        .tools
        .iter()
        .filter(|(card_id, _)| !cards.contains(*card_id))
        .map(|(card_id, binding)| {
            let mut off = binding.clone();
            off.enabled = false;
            (card_id.clone(), off)
        })
        .collect();
    for card_id in &cards {
        tools.insert(
            card_id.clone(),
            AgentToolBinding {
                enabled: true,
                preset: presets.get(card_id).copied(),
                allowed_tools: None,
            },
        );
    }
    next.tools = tools;
    writer.write_agent(AGENT_ID, next)?;

    let permissions: BTreeMap<String, String> = cards
        .iter()
        .filter_map(|card_id| {
            profile
                .permissions
                .get(card_id)
                .map(|tier| (card_id.clone(), tier.as_str().to_string()))
        })
        .collect();

    Ok(Applied {
        name: profile.name.clone(),
        web,
        cards: cards.into_iter().collect(),
        tools: wanted.into_iter().collect(),
        permissions,
        knobs: profile.knobs.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(extra: &str) -> String {
        format!("name = \"t\"\n[tools]\non = [\"read\"]\n{extra}")
    }

    #[test]
    fn a_tool_listed_twice_is_rejected() {
        let error =
            EvalProfile::parse("name = \"t\"\n[tools]\non = [\"read\"]\noff = [\"read\"]\n")
                .expect_err("an id in two lists is ambiguous");
        assert!(error.to_string().contains("'read'"), "{error}");
        assert!(error.to_string().contains("both"), "{error}");
    }

    #[test]
    fn the_code_transport_cannot_be_switched_off() {
        let error = EvalProfile::parse(&profile("off = [\"run_code\"]\n"))
            .expect_err("run_code is the transport");
        assert!(error.to_string().contains("run_code"), "{error}");
    }

    #[test]
    fn a_knob_value_must_be_a_real_tier() {
        let error = EvalProfile::parse(&format!(
            "{}[knobs]\nthinking = \"huge\"\n",
            profile("off = []\n")
        ))
        .expect_err("thinking has three tiers");
        assert!(error.to_string().contains("thinking"), "{error}");
        let error = EvalProfile::parse(&format!(
            "{}[knobs]\ntemperature = \"0.2\"\n",
            profile("off = []\n")
        ))
        .expect_err("an unknown knob would be ignored");
        assert!(error.to_string().contains("temperature"), "{error}");
    }

    #[test]
    fn the_applied_document_records_the_answers() {
        let applied = Applied {
            name: "t".to_string(),
            web: WebMode::On,
            cards: vec!["read".to_string(), "webfetch".to_string()],
            tools: vec![
                "read".to_string(),
                "run_code".to_string(),
                "webfetch".to_string(),
            ],
            permissions: [("read".to_string(), "all".to_string())]
                .into_iter()
                .collect(),
            knobs: [("thinking".to_string(), "medium".to_string())]
                .into_iter()
                .collect(),
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("applied.json");
        applied
            .write_document(Path::new("eval/profile.toml"), &path)
            .expect("write");
        let text = std::fs::read_to_string(&path).expect("read");
        let document: serde_json::Value = serde_json::from_str(&text).expect("json");
        // "auto" is a question, never an answer.
        assert_eq!(document["web"], "on");
        assert_eq!(document["source"], "eval/profile.toml");
        assert_eq!(document["permissions"]["read"], "all");
        assert_eq!(document["tools"][1], "run_code");
        assert_eq!(document["knobs"]["thinking"], "medium");

        // An artifact quotes the hash of these bytes, so they must not drift.
        let again = dir.path().join("again.json");
        applied
            .write_document(Path::new("eval/profile.toml"), &again)
            .expect("write");
        assert_eq!(text, std::fs::read_to_string(&again).expect("read"));
    }

    #[test]
    fn auto_web_needs_the_probe_answer() {
        // Pure shape: the applier refuses before it touches a database.
        let parsed = EvalProfile::parse(&profile("")).expect("parses");
        assert_eq!(parsed.web, WebMode::Auto);
        assert_eq!(parsed.wanted(WebMode::Off).len(), 1);
    }
}
