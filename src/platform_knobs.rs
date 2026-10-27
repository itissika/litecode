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

/// Whether auto compaction may summarize in the background while the agent
/// keeps looping. Read once per runtime; a turn never changes mode mid-flight.
///
/// - `Off`: synchronous compaction at the 0.8 prepare line.
/// - `Shadow`: reserved preview mode; it follows the synchronous path today.
/// - `On`: an independent branch owns the summary and its commit. This is the
///   production default; set `LITECODE_COMPACT_PREFETCH=off` to opt out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompactionPrefetchMode {
    Off,
    Shadow,
    #[default]
    On,
}

impl CompactionPrefetchMode {
    pub const ENV: &'static str = "LITECODE_COMPACT_PREFETCH";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::On => "on",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "shadow" => Some(Self::Shadow),
            "on" => Some(Self::On),
            _ => None,
        }
    }

    /// Default is `On`: an unset or invalid env var uses the background branch.
    /// `LITECODE_COMPACT_PREFETCH=off` restores synchronous compaction.
    pub fn from_env() -> Self {
        std::env::var(Self::ENV)
            .ok()
            .and_then(|raw| Self::parse(&raw))
            .unwrap_or_default()
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
    use super::CompactionPrefetchMode;

    #[test]
    fn compaction_prefetch_mode_parses_known_values_only() {
        assert_eq!(
            CompactionPrefetchMode::parse("off"),
            Some(CompactionPrefetchMode::Off)
        );
        assert_eq!(
            CompactionPrefetchMode::parse(" SHADOW "),
            Some(CompactionPrefetchMode::Shadow)
        );
        assert_eq!(
            CompactionPrefetchMode::parse("On"),
            Some(CompactionPrefetchMode::On)
        );
        assert_eq!(CompactionPrefetchMode::parse("maybe"), None);
        assert_eq!(
            CompactionPrefetchMode::default(),
            CompactionPrefetchMode::On
        );
    }
}
