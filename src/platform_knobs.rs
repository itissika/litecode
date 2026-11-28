//! Platform semantics for thinking intensity and context window mode.
//!
//! See `knowledge/contracts/config/platform-knobs.md`. UI / session persist platform
//! enums; the provider catalog maps them onto vendor wire literals.

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

/// Context window selected by the product mode.
///
/// `Standard` uses the model's standard/default budget (normally 256K, or the
/// model maximum when smaller); `Max` uses the model's true maximum. The
/// physical prompt ceiling still uses the model maximum separately.
pub fn effective_context_window(model: &ResolvedModel, mode: ContextMode) -> usize {
    match mode {
        ContextMode::Standard => model.context_window,
        ContextMode::Max => model.context_window_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_catalog::{DEFAULT_CATALOG, ProviderCatalog};
    use crate::session::estimate::compaction_budget;
    use std::path::Path;

    /// The catalog is the only origin of these two numbers: the eval's 1M/256K
    /// comparison reads them, and the 80% prepare line is derived from them.
    #[test]
    fn cline_pass_exposes_256k_standard_and_1m_max() {
        let catalog = ProviderCatalog::parse(DEFAULT_CATALOG, Path::new("default_catalog.toml"))
            .expect("the embedded catalog is valid");
        let model = catalog
            .model("cline/cline-pass/deepseek-v4.1-flash-1017")
            .expect("cline-pass model is in the shipped catalog");

        let standard = effective_context_window(model, ContextMode::Standard);
        let max = effective_context_window(model, ContextMode::Max);
        assert_eq!(standard, 256_000);
        assert_eq!(max, 1_000_000);
        assert_eq!(compaction_budget(standard), 204_800);
        assert_eq!(compaction_budget(max), 800_000);
        // The prepare line stays under the hard prompt ceiling (the physical
        // window minus the output reserve), so `max` still compacts before it
        // stops sending.
        let hard_line = max - usize::try_from(model.max_output).expect("max_output fits");
        assert!(compaction_budget(max) < hard_line);
    }

    #[test]
    fn a_mode_is_the_only_spelling_a_session_row_can_hold() {
        assert_eq!(ContextMode::default(), ContextMode::Standard);
        assert_eq!(ContextMode::parse("max"), Some(ContextMode::Max));
        assert_eq!(
            ContextMode::parse(" standard "),
            Some(ContextMode::Standard)
        );
        assert_eq!(ContextMode::parse("1m"), None);
        assert_eq!(ContextMode::Max.as_str(), "max");
    }
}
