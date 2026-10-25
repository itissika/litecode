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
mod tests {}
