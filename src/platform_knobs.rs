//! Platform semantics for thinking intensity and context window mode.
//!
//! See `docs/platform-knobs.md`. UI / session persist platform enums; the
//! provider catalog maps them onto vendor wire literals.

use serde::{Deserialize, Serialize};

use crate::provider_catalog::ResolvedModel;

/// Platform thinking intent on [`crate::llm::ModelRequest`].
///
/// A codec maps this onto the resolved model's `reasoning.tiers`. `Off` is
/// compaction (and any other caller that must not think): it is **not**
/// `ThinkingTier::Low`, and it sends the model's declared `tiers.off` literal
/// when the vendor has one, no reasoning control at all otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSpec {
    Off,
    Tier(ThinkingTier),
}

impl Default for ThinkingSpec {
    fn default() -> Self {
        Self::Tier(ThinkingTier::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ThinkingTier {
    Low,
    #[default]
    Medium,
    High,
}

impl ThinkingTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ContextMode {
    #[default]
    Standard,
    Max,
}

impl ContextMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "standard" => Some(Self::Standard),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

/// Context budget for one turn.
///
/// Both numbers are catalog facts: `context_window` is the standard budget and
/// `context_window_max` the ceiling the user opts into with
/// [`ContextMode::Max`]. Nothing is clamped or guessed at request time.
pub fn effective_context_window(model: &ResolvedModel, mode: ContextMode) -> usize {
    match mode {
        ContextMode::Standard => model.context_window,
        ContextMode::Max => model.context_window_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_catalog::ProviderCatalog;
    use std::path::Path;
    use std::sync::Arc;

    fn model(entry: &str) -> Arc<ResolvedModel> {
        let text = format!(
            "version = 1\n[[providers]]\nid = \"p\"\nname = \"P\"\nendpoint = \"https://x.example/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"p\"\n{entry}\n"
        );
        let catalog = ProviderCatalog::parse(&text, Path::new("t.toml")).unwrap();
        Arc::clone(catalog.model("p/m").unwrap())
    }

    #[test]
    fn standard_and_max_are_catalog_facts() {
        let resolved = model("context_window = 128000\ncontext_window_max = 1000000\n");
        assert_eq!(
            effective_context_window(&resolved, ContextMode::Standard),
            128_000
        );
        assert_eq!(
            effective_context_window(&resolved, ContextMode::Max),
            1_000_000
        );
    }

    #[test]
    fn max_defaults_to_the_standard_budget() {
        let resolved = model("context_window = 128000\n");
        assert_eq!(
            effective_context_window(&resolved, ContextMode::Max),
            128_000
        );
    }

    #[test]
    fn thinking_tiers_round_trip_through_strings() {
        for tier in [ThinkingTier::Low, ThinkingTier::Medium, ThinkingTier::High] {
            assert_eq!(ThinkingTier::parse(tier.as_str()), Some(tier));
        }
        assert_eq!(ThinkingTier::parse("max"), None);
        assert_eq!(ContextMode::parse("max"), Some(ContextMode::Max));
        assert_eq!(
            ThinkingSpec::default(),
            ThinkingSpec::Tier(ThinkingTier::Medium)
        );
    }
}
