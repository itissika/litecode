//! Explicit retrieval layers: **what** produced a hit, how well it matched the
//! query's intent, and the one key the final ordering is allowed to sort on.
//!
//! # Why this exists
//!
//! The lane used to hand every mechanism the same single `f64` and let the last
//! stage sort by it. That works only while the numbers mean the same thing, and
//! they never do: an exact substring, a `NEAR` window, a BM25 rank and an n-gram
//! overlap are four different measurements, and no amount of tuning makes them
//! comparable across queries. Worse, once a hit is a bare score the reason it was
//! returned is gone — nothing above the lane can tell "the literal is here" from
//! "one common word happens to be here", so nothing above can rank the second
//! below the first.
//!
//! So a hit carries its provenance. [`HitEvidence`] is the machine-readable
//! answer to "why is this row in the list", [`RankKey`] is the ordering derived
//! from it, and [`LAYERS`] is the declarative statement of what each layer is
//! for — its goal, its priority, when it is allowed to run, what it must prove
//! before a hit counts, and how deep it may look.
//!
//! Three rules hold the model together:
//!
//! 1. **A native score never leaves its layer.** BM25, RRF and coverage are all
//!    `native` values; the only cross-layer currency is [`RankKey`].
//! 2. **A layer's priority is a product decision, not a score.** It is written
//!    down in [`LAYERS`], not encoded into a float band.
//! 3. **Provenance survives.** A hit keeps every layer that found it, so the
//!    fusion stage can reward agreement instead of guessing at it.
//!
//! The types here are internal. Nothing in this module reaches the agent-facing
//! view: that view carries prose and line numbers only, and a test pins it.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::slots::Slot;

/// Which mechanism produced a hit.
///
/// The first four are the sparse lane's leaf layers, in the order the product
/// prefers them (`Exact` > `Proximity` > `Lexical` > `Fuzzy`). `Semantic` is the
/// ANN lane, which has no rank relative to the sparse leaves on its own — it is
/// fused with `Lexical`. The last two name the composite layers, so a hit can
/// report "the sparse full layer produced this" as easily as a leaf can.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum LayerId {
    /// The query's normalized literal, present as a substring.
    #[default]
    Exact,
    /// The query's informative terms, inside one proximity window.
    Proximity,
    /// The query's terms, matched with enough coverage to count as intent.
    Lexical,
    /// Low-coverage n-gram overlap: typo tolerance, never a peer of the above.
    Fuzzy,
    /// Dense ANN over the session corpus.
    Semantic,
    /// The sparse lane's own fused output.
    SparseFull,
    /// The session-wide fused output (sparse + semantic).
    SessionFull,
}

impl LayerId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Proximity => "proximity",
            Self::Lexical => "lexical",
            Self::Fuzzy => "fuzzy",
            Self::Semantic => "semantic",
            Self::SparseFull => "sparse_full",
            Self::SessionFull => "session_full",
        }
    }
}

/// Ordering band of the evidence behind a hit.
///
/// A band is a **product contract**: a hit carrying only lower-band evidence is
/// never ranked above one carrying higher-band evidence, whatever their scores
/// say. `Fusion` is the band where heterogeneous methods (sparse lexical and
/// dense ANN) are merged by rank, because their native scores are not comparable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum RankBand {
    /// Literal substring evidence.
    #[default]
    Exact,
    /// Proximity evidence.
    Proximity,
    /// Rank-fused loose recall (lexical and/or semantic).
    Fusion,
    /// n-gram overlap only: last resort.
    Fuzzy,
}

impl RankBand {
    pub fn from_layer(layer: LayerId) -> Self {
        match layer {
            LayerId::Exact => Self::Exact,
            LayerId::Proximity => Self::Proximity,
            LayerId::Lexical | LayerId::Semantic | LayerId::SparseFull | LayerId::SessionFull => {
                Self::Fusion
            }
            LayerId::Fuzzy => Self::Fuzzy,
        }
    }

    fn index(self) -> u8 {
        self as u8
    }
}

/// Retrieval-facing role of a row: what question its text answers.
///
/// The tiers, in the order the product prefers them: **what was said** (human
/// and assistant alike) → **what the model thought** → **what it did** → **what
/// came back**. Nothing is filtered by role — a tool result is still fully
/// searchable.
///
/// A role is a *preference inside a band*, not a band of its own: see
/// [`ContentRole::weigh`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ContentRole {
    /// `item/user`, `item/assistant` — what was said, by either side.
    Conversation,
    /// `item/assistant` reasoning — the model's own working.
    Reasoning,
    /// `item/tool_call` — what was done.
    Action,
    /// `item/tool_result` — how it went.
    Outcome,
    /// A row whose kind is unknown to the index.
    #[default]
    Unknown,
}

impl ContentRole {
    /// Classify from the durable kind, the way the corpus already does.
    pub fn from_slot(slot: Slot) -> Self {
        match slot {
            Slot::Who | Slot::Said => Self::Conversation,
            Slot::Thought => Self::Reasoning,
            Slot::Did => Self::Action,
            Slot::Outcome => Self::Outcome,
            Slot::Summary | Slot::When | Slot::Other => Self::Unknown,
        }
    }

    /// Classify from `item_type` alone, for lanes that carry no `kind`
    /// (the semantic corpus keeps only the item type). Every one of the four
    /// tiers is unambiguous from the item type alone, so this agrees with
    /// [`Self::from_slot`] exactly.
    pub fn from_item_type(item_type: &str) -> Self {
        match item_type {
            "message" => Self::Conversation,
            "reasoning" => Self::Reasoning,
            "function_call" => Self::Action,
            "function_call_output" => Self::Outcome,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Reasoning => "reasoning",
            Self::Action => "action",
            Self::Outcome => "outcome",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "conversation" => Self::Conversation,
            "reasoning" => Self::Reasoning,
            "action" => Self::Action,
            "outcome" => Self::Outcome,
            _ => Self::Unknown,
        }
    }

    /// Ranking preference, lower is better: what was said, then the model's own
    /// working, then the call, then its output, then everything unclassified.
    pub fn preference(self) -> u8 {
        match self {
            Self::Conversation => 0,
            Self::Reasoning => 1,
            Self::Action => 2,
            Self::Outcome => 3,
            Self::Unknown => 4,
        }
    }

    /// The tier's preference as a permille scale on a band's strength.
    ///
    /// The steps are small on purpose. A role is a *preference*, not a band: it
    /// reorders hits whose match quality is close, and loses to any hit whose
    /// quality is visibly better. One more matched term of a three-term query is
    /// 33% of coverage, so a step of 5% can never stand in for a match — while
    /// at equal coverage, or at equal occurrence count, it is the difference.
    ///
    /// This is why the lane keeps exactly one number per band: tuning the
    /// product's taste means editing this table, not adding a comparison.
    pub fn weight_permille(self) -> u32 {
        match self {
            Self::Conversation => 1000,
            Self::Reasoning => 950,
            Self::Action => 900,
            Self::Outcome => 850,
            Self::Unknown => 800,
        }
    }

    /// Apply this role's preference to a within-band strength.
    ///
    /// The single place a role reaches the ordering. `u64` because the fused
    /// scale is near `u32::MAX` and must not wrap.
    pub fn weigh(self, strength: u32) -> u32 {
        (u64::from(strength) * u64::from(self.weight_permille()) / 1000) as u32
    }
}

/// One layer's reason for returning one hit.
///
/// `matched`/`total` are **counts of intent**, not scores: how many of the
/// branch's query terms (or grams) this hit actually contains, out of how many
/// the branch asked for. That is what lets the gate below reject "one common
/// word happened to appear" without a threshold on an absolute score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitEvidence {
    pub layer: LayerId,
    /// 0-based index of the `|` branch that produced this hit.
    #[serde(default)]
    pub branch: usize,
    /// Query terms (or grams) of the branch that this hit matched.
    #[serde(default)]
    pub matched: usize,
    /// Query terms (or grams) the branch asked for.
    #[serde(default)]
    pub total: usize,
    /// 0-based rank inside its own layer, before any fusion.
    #[serde(default)]
    pub local_rank: usize,
    /// layer-local diagnostic value (BM25 for a `MATCH` layer, RRF for a fused
    /// layer, occurrence count for `Exact`). **Never** compared across layers,
    /// and the Exact count is not a rank signal.
    #[serde(default)]
    pub native: f64,
}

impl HitEvidence {
    pub fn new(layer: LayerId, branch: usize, matched: usize, total: usize) -> Self {
        Self {
            layer,
            branch,
            matched,
            total,
            local_rank: 0,
            native: 0.0,
        }
    }

    /// Fraction of the branch's intent this hit covers. A hit with no counted
    /// terms (an exact literal match) counts as full coverage.
    pub fn coverage(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.matched as f64 / self.total as f64
        }
    }

    pub fn coverage_permille(&self) -> u32 {
        (self.coverage() * 1000.0).round() as u32
    }

    /// Does this evidence clear a layer's minimum? Counted, never scored.
    pub fn clears(&self, min_matches: usize) -> bool {
        self.matched >= min_matches || self.total == 0
    }

    pub fn band(&self) -> RankBand {
        RankBand::from_layer(self.layer)
    }
}

/// The single key the final ordering sorts on.
///
/// Integers only, on purpose: a float here would invite the very thing this
/// module exists to stop — comparing two different measurements. `strength` is a
/// *within-band* ranking derived from one layer's own evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RankKey {
    pub band: RankBand,
    /// Within-band strength, greater is better. `Exact`: presence only — how
    /// many times the literal repeats stays on [`HitEvidence::native`] and is
    /// not compared. `Proximity`/`Fuzzy`: coverage in permille. `Fusion`: the
    /// fused RRF sum as an integer (`rrf × 1_000_000`). Non-exact scales are
    /// scaled once more by the hit's [`ContentRole`] weight.
    pub strength: u32,
    /// Speaker tier on an Exact hit ([`speaker_tier`]); [`ContentRole::preference`]
    /// on every other band. Lower is better.
    pub role: u8,
    /// Rank inside the layer that produced the hit. The last relevance
    /// tie-break outside Exact; Exact ignores it so update time can break the tie.
    pub local_rank: u32,
}

/// Order two rank keys, best first.
///
/// Band first — a product contract. Inside Exact, every row already contains
/// the literal, so neither repetition nor a strength number orders them: the
/// speaker tier is the whole key. Equals fall through to the caller, which
/// breaks them on session update time. A reasoning row that quotes the person
/// ten times stays under the person, and does not climb past another reasoning
/// row that says it once. Every other band keeps strength first, so a partial
/// mention still loses to a fuller one.
pub fn cmp_rank(a: &RankKey, b: &RankKey) -> Ordering {
    let band = a.band.index().cmp(&b.band.index());
    if band != Ordering::Equal {
        return band;
    }
    if a.band == RankBand::Exact {
        return a.role.cmp(&b.role);
    }
    b.strength
        .cmp(&a.strength)
        .then_with(|| a.role.cmp(&b.role))
        .then_with(|| a.local_rank.cmp(&b.local_rank))
}

/// Who produced the text, for Exact ordering. Lower is better.
///
/// Human speech, then what the assistant said, then its reasoning, then the
/// call, then the tool result. [`ContentRole`] keeps human and assistant
/// speech together; this split is ranking-only and is not part of the view.
/// A missing kind falls back to the stored role, which cannot tell the two
/// speakers apart — the live index always has the kind.
pub fn speaker_tier(kind: Option<&str>, item_type: &str, role: ContentRole) -> u8 {
    match kind {
        Some("item/user") => 0,
        Some("item/assistant") if item_type == "reasoning" => 2,
        Some("item/assistant") => 1,
        Some("item/tool_call") => 3,
        Some("item/tool_result") => 4,
        _ => match role {
            ContentRole::Conversation => 0,
            ContentRole::Reasoning => 2,
            ContentRole::Action => 3,
            ContentRole::Outcome => 4,
            ContentRole::Unknown => 5,
        },
    }
}

/// RRF constant. Ranks, not scores: a document's contribution decays with its
/// position in each list, so lists whose scores share no scale can still be
/// merged. 60 is the value the original RRF work settled on and the default
/// every engine that ships RRF uses.
pub const RRF_K: f64 = 60.0;

/// One list's contribution to a fused score, from a 0-based rank.
pub fn rrf_score(local_rank: usize, weight: f64) -> f64 {
    weight / (RRF_K + local_rank as f64 + 1.0)
}

/// How a layer may be entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    /// Runs on every query.
    Always,
    /// Runs only when the query has more than one informative term.
    MultiTerm,
    /// Runs only when the layers above it did not return enough distinct rows.
    RecallShortfall,
}

/// A layer's declared semantics: what it is for, how much it outranks the next
/// one, when it may run, what a hit must prove, and how deep it may look.
///
/// This table is the retrieval policy. Nothing else in the code base is allowed
/// to imply an ordering between mechanisms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerSemantics {
    pub id: LayerId,
    /// The layer's semantic goal, in one line.
    pub goal: &'static str,
    /// Product priority; lower is better. Ordering of the leaf layers.
    pub priority: u8,
    pub activation: Activation,
    /// Minimum fraction of the branch's intent a hit must cover to count.
    ///
    /// For the word path the concrete floor is
    /// [`super::query_plan::QueryBranch::min_word_matches`], which is a count
    /// rule and not a ratio; this is the ratio the trigram path uses.
    pub min_coverage: f64,
    /// For a layer that demands a contiguous fragment (the fuzzy one): the
    /// fraction of the branch the fragment must span. A typo leaves most of the
    /// string intact, so this is what separates "a near miss" from "a different
    /// string that shares a word".
    pub min_run_fraction: f64,
    /// Candidate pool for this layer, as a multiple of the caller's limit.
    pub depth_factor: usize,
    /// Hard cap on that pool, so a common term cannot pull the corpus into
    /// memory just because the caller asked for more results.
    pub depth_cap: usize,
}

impl LayerSemantics {
    /// This layer's candidate depth for one query.
    pub fn depth(&self, limit: usize) -> usize {
        limit.saturating_mul(self.depth_factor).min(self.depth_cap)
    }

    /// Minimum matched terms/grams a hit must show.
    pub fn min_matches(&self, total: usize) -> usize {
        if total == 0 {
            return 0;
        }
        ((total as f64 * self.min_coverage).ceil() as usize).clamp(1, total)
    }
}

/// The four sparse leaf layers, in priority order.
pub const LAYERS: [LayerSemantics; 4] = [
    LayerSemantics {
        id: LayerId::Exact,
        goal: "the query's normalized literal is present, verbatim",
        priority: 0,
        activation: Activation::Always,
        // A literal match is the strongest possible evidence: full coverage.
        min_coverage: 1.0,
        min_run_fraction: 0.0,
        depth_factor: 1,
        depth_cap: 400,
    },
    LayerSemantics {
        id: LayerId::Proximity,
        goal: "the query's informative terms share one window, in any order",
        priority: 1,
        activation: Activation::MultiTerm,
        // `NEAR` already proves the terms are together; coverage only has to
        // confirm that the window was not satisfied by a subset.
        min_coverage: 0.6,
        min_run_fraction: 0.0,
        depth_factor: 2,
        depth_cap: 400,
    },
    LayerSemantics {
        id: LayerId::Lexical,
        goal: "enough of the query's words are present to count as its intent",
        priority: 2,
        activation: Activation::Always,
        // The gate that stops one common word from filling the list. The floor
        // itself is a count rule, not this ratio: `min_word_matches` for Latin,
        // `token_floor` plus `cjk_char_min` for Chinese — a ratio over a long
        // query is what used to make natural language unfindable. The value
        // below is the layer's declared coverage for the eval lanes.
        min_coverage: 0.6,
        min_run_fraction: 0.0,
        depth_factor: 2,
        depth_cap: 400,
    },
    LayerSemantics {
        id: LayerId::Fuzzy,
        goal: "typo tolerance: overlapping n-grams, never a peer of the above",
        priority: 3,
        activation: Activation::RecallShortfall,
        min_coverage: 0.35,
        // Three quarters of the string, unbroken. A typo keeps that much; a row
        // that merely shares a word — even a word that runs into the query's
        // tail across a space — does not.
        min_run_fraction: 3.0 / 4.0,
        depth_factor: 1,
        depth_cap: 200,
    },
];

/// The declared semantics of one layer, by id.
pub fn semantics(id: LayerId) -> &'static LayerSemantics {
    LAYERS.iter().find(|l| l.id == id).unwrap_or(&LAYERS[0])
}

/// What a layer did on one query. Diagnostics only: it is logged and asserted on
/// in tests, and it never reaches the agent-facing view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerTrace {
    pub layer: LayerId,
    /// Hits the layer's SQL returned.
    pub produced: usize,
    /// Hits the coverage gate rejected.
    pub rejected: usize,
    /// Hits that survived the gate.
    pub accepted: usize,
    pub stop: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The layer ran and its output was used.
    Ran,
    /// The layer could not be entered for this query (single term, CJK, ...).
    NotApplicable,
    /// Layers above already answered well enough; this one was not entered.
    EnoughHighConfidence,
    /// Higher layers were thin, so this one was entered to widen recall.
    RecallShortfall,
}

#[cfg(test)]
mod tests {}
