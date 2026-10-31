//! Transcript port: the one place that decides what a target model may receive.
//!
//! The kernel transcript is authority \`Item\`. Before a request, the runtime asks
//! this module one question: *given who produced each item and which model is
//! about to read it, what is still sendable?* The answer is a filtered copy plus
//! a [\`PortReport\`] — every removal is counted, nothing disappears silently.
//!
//! Layering contract:
//!
//! - **port** owns semantic policy: replay admissibility, thinking intent,
//!   capability blocks. It never knows a wire field name. (Media projection is
//!   the same policy but runs in the view builder, before media refs expand;
//!   its count flows back into [`PortReport::projected_parts`].)
//! - **codec** owns dialect shape: how allowed content is spelled. It must not
//!   decide that content is unwanted.
//! - [\`PortProfile\`] is the target's declared shape family, so the port can
//!   guarantee the codec's preconditions without naming a vendor.

use crate::authority::responses::{Item, ReasoningItem, ReasoningItemContent, SummaryPart};
use crate::llm::replay_compat::{ItemProducer, strip_foreign_ciphertext, strip_rebased_signatures};
use crate::platform_knobs::ThinkingSpec;
use crate::provider_catalog::{EndpointKind, ReasoningReplay, ResolvedModel};

/// How a target protocol carries prior reasoning back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningPort {
    /// OpenAI-family Responses: only the producer's own ciphertext replays.
    Ciphertext,
    /// Responses host that replays its own item id plus summary text.
    Summary,
    /// Responses host that accepts plain reasoning text (DeepSeek/MiMo).
    /// Readable text is portable; producer ownership is irrelevant.
    ReasoningText,
    /// Anthropic Messages: a signed thinking block; unsigned text is not sendable.
    SignedBlock,
    /// Chat Completions: reasoning goes into a vendor text key. Readable text
    /// is portable; producer ownership is irrelevant.
    VendorText,
}

/// The target's portability shape. Protocol-level, not vendor-level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PortProfile {
    pub reasoning: ReasoningPort,
}

/// Why a switch or a turn cannot be served by this target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortBlock {
    /// The agent loop sends tools; the target declares \`tool_call = false\`.
    ToolsUnsupported,
}

impl PortBlock {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::ToolsUnsupported => {
                "this model does not support tool calls, but the agent loop sends tools"
            }
        }
    }
}

/// What the port did to one request view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PortReport {
    pub items_in: u32,
    pub items_out: u32,
    pub reasoning_dropped: u32,
    pub ciphertext_stripped: u32,
    pub signatures_stripped: u32,
    /// Media parts the view already replaced with placeholders (counted by the
    /// pipeline before refs are expanded).
    pub projected_parts: u32,
    pub blocked: Option<PortBlock>,
}

impl PortReport {
    /// True when the target receives less than the transcript holds.
    pub(crate) fn degraded(&self) -> bool {
        self.reasoning_dropped > 0
            || self.ciphertext_stripped > 0
            || self.signatures_stripped > 0
            || self.projected_parts > 0
    }
}

/// The target's shape family. Derived from catalog facts, never a provider id.
pub(crate) fn profile_for(target: &ResolvedModel) -> PortProfile {
    let reasoning = match target.endpoint_type {
        EndpointKind::Responses => match target.reasoning_replay {
            ReasoningReplay::Text => ReasoningPort::ReasoningText,
            ReasoningReplay::Summary => ReasoningPort::Summary,
            ReasoningReplay::Ciphertext => ReasoningPort::Ciphertext,
        },
        EndpointKind::Messages => ReasoningPort::SignedBlock,
        EndpointKind::ChatCompletions => ReasoningPort::VendorText,
    };
    PortProfile { reasoning }
}

/// Filter one request view for \`target\`.
///
/// \`producers\` and \`rebased\` are parallel to \`items\` (same contract as the
/// replay rules). The function mutates \`items\` in place and returns what it
/// did. A blocked target leaves \`items\` untouched: the caller must abort.
pub(crate) fn port(
    items: &mut Vec<Item>,
    producers: &[Option<ItemProducer>],
    rebased: &[bool],
    target: &ResolvedModel,
    thinking: ThinkingSpec,
    has_tools: bool,
) -> PortReport {
    let mut report = PortReport {
        items_in: items.len() as u32,
        ..PortReport::default()
    };

    if has_tools && !target.tool_call {
        report.blocked = Some(PortBlock::ToolsUnsupported);
        report.items_out = report.items_in;
        return report;
    }

    let endpoint_type = target.endpoint_type.as_str();
    report.signatures_stripped = strip_rebased_signatures(items, rebased, endpoint_type) as u32;
    report.ciphertext_stripped = strip_foreign_ciphertext(
        items,
        producers,
        &target.provider_id,
        endpoint_type,
        &target.reference,
    )
    .ciphertexts_stripped as u32;

    let profile = profile_for(target);
    let provider = target.provider_id.as_str();
    let thinking_off = matches!(thinking, ThinkingSpec::Off);
    let mut kept = Vec::with_capacity(items.len());

    for (index, item) in items.drain(..).enumerate() {
        let Item::Reasoning(reasoning) = item else {
            kept.push(item);
            continue;
        };
        let own = producers
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|producer| producer.provider_id == provider);
        let signed = reasoning
            .encrypted_content
            .as_deref()
            .is_some_and(|signature| !signature.is_empty());

        let keep = match profile.reasoning {
            ReasoningPort::Ciphertext => signed,
            ReasoningPort::Summary => own && has_id(&reasoning) && has_replay_text(&reasoning),
            ReasoningPort::ReasoningText => has_replay_text(&reasoning),
            ReasoningPort::SignedBlock => !thinking_off && own && signed,
            ReasoningPort::VendorText => has_replay_text(&reasoning),
        };

        if keep {
            kept.push(Item::Reasoning(reasoning));
        } else {
            report.reasoning_dropped += 1;
        }
    }

    *items = kept;
    report.items_out = items.len() as u32;
    report
}

fn has_id(reasoning: &ReasoningItem) -> bool {
    reasoning
        .id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
}

fn has_replay_text(reasoning: &ReasoningItem) -> bool {
    let from_content = reasoning.content.as_ref().is_some_and(|parts| {
        parts.iter().any(|part| match part {
            ReasoningItemContent::ReasoningText(text) => !text.text.trim().is_empty(),
        })
    });
    from_content
        || reasoning.summary.iter().any(|part| match part {
            SummaryPart::SummaryText(text) => !text.text.trim().is_empty(),
        })
}

#[cfg(test)]
mod tests {}
