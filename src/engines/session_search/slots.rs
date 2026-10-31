//! Session semantic slots — the first cut over the session corpus.
//!
//! The underlying goal of session retrieval in one line:
//! **when, who, how they thought / said it, what they did, how it went.**
//!
//! The corpus is cut by data type first (classify → trim → tag); dedup comes
//! later, as a consequence.
//!
//! # The final corpus policy (WP-128 / WP-130 whitelist)
//!
//! Lane admission (new kinds default **out** unless listed):
//!
//! | Lane | Admits |
//! |---|---|
//! | **Dense** | Who / Said / Thought only (user, assistant message, thinking) |
//! | **Sparse** | Dense three + Did (`item/tool_call`) |
//!
//! Outcome (`tool_result`), Summary, When, Other, reminder/control, unknown —
//! **not** searchable on either lane. Per-tool envelopes stay archived.
//!
//! Why the earlier per-tool machinery (`slot-trim` / `clean` / `clean-prose` and
//! their noise collapse) was archived: it cost coverage (the self-referential
//! tool-row rules silently dropped truth rows), and its complexity served one
//! tool family at a time — the wrong trade for a production lane.
//!
//! `V0Prod` remains as the frozen **baseline** (what production embeds today),
//! not as a selectable scheme.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Role of one durable session row, in the user-facing question it answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// 时间 — `turn/start` / `turn/end` anchors. Metadata; not embedded by default.
    When,
    /// 谁 — `item/user`: the human's intent, asks, constraints.
    Who,
    /// 怎么想 — `item/assistant` reasoning.
    Thought,
    /// 说了 — `item/assistant` visible text.
    Said,
    /// 干了什么 — `item/tool_call` actions.
    Did,
    /// 干得怎么样 — `item/tool_result` outcome / evidence.
    Outcome,
    /// compacted summaries (model-distilled; excluded from the final corpus).
    Summary,
    /// `reminder/*`, `plan/execute`, unknown kinds.
    Other,
}

impl Slot {
    pub fn as_str(self) -> &'static str {
        match self {
            Slot::When => "when",
            Slot::Who => "who",
            Slot::Thought => "thought",
            Slot::Said => "said",
            Slot::Did => "did",
            Slot::Outcome => "outcome",
            Slot::Summary => "summary",
            Slot::Other => "other",
        }
    }

    /// Slots that carry retrieval value as text. `When` stays metadata.
    pub fn default_indexed(self) -> bool {
        !matches!(self, Slot::When)
    }

    /// Chunking applies to text that can be long: the three intent slots
    /// (人话 + 工具调用). Tool results stay one document each, so a hit on them
    /// resolves to the event, not to a slice of output.
    pub fn chunks(self) -> bool {
        matches!(self, Slot::Who | Slot::Said | Slot::Thought | Slot::Did)
    }
}

/// Classify one durable row: `transcript_items.kind` + `item_type`.
pub fn classify(kind: &str, item_type: &str) -> Slot {
    match (kind, item_type) {
        ("item/user", _) => Slot::Who,
        ("item/assistant", "reasoning") => Slot::Thought,
        ("item/assistant", _) => Slot::Said,
        ("item/tool_call", _) => Slot::Did,
        ("item/tool_result", _) => Slot::Outcome,
        ("compacted", _) => Slot::Summary,
        ("turn/start" | "turn/end", _) => Slot::When,
        _ => Slot::Other,
    }
}

/// Dense lane whitelist: user / assistant message / thinking only.
pub fn admits_dense(slot: Slot) -> bool {
    matches!(slot, Slot::Who | Slot::Said | Slot::Thought)
}

/// Sparse lane whitelist: dense three + tool_call intent (`Did`).
pub fn admits_sparse(slot: Slot) -> bool {
    matches!(slot, Slot::Who | Slot::Said | Slot::Thought | Slot::Did)
}

/// Corpus policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    /// Frozen baseline: the five searchable kinds, raw item text, one row = one
    /// chunk, no trimming, no filter. Must stay byte-identical to what
    /// production embeds today; kept for comparison, never selected.
    V0Prod,
    /// Locked product corpus (see module doc). Dense: three-state prose;
    /// sparse admits Did separately via [`admits_sparse`]. Outcome excluded.
    Final,
}

impl Policy {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "v0" | "v0-prod" | "prod" | "baseline" => Ok(Policy::V0Prod),
            "final" | "v1" | "clean" => Ok(Policy::Final),
            other => bail!(
                "unknown session corpus policy `{other}` (v0-prod | final); \
                 the archived policies slot-trim / clean-prose / chunk are not selectable"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Policy::V0Prod => "v0-prod",
            Policy::Final => "final",
        }
    }
}

/// Trimming knobs. `budget_tokens` mirrors the product embed window
/// (`EMBED_MAX_LENGTH = 512`); characters are used as the cheap proxy.
#[derive(Debug, Clone, Copy)]
pub struct SlotCfg {
    pub budget_tokens: usize,
    /// Include `turn/*` anchors as documents.
    pub index_when: bool,
}

impl Default for SlotCfg {
    fn default() -> Self {
        Self {
            budget_tokens: 512,
            index_when: false,
        }
    }
}

impl SlotCfg {
    /// Mixed CJK/Latin roughly costs 3 chars per token for this tokenizer.
    pub fn chars_budget(&self) -> usize {
        (self.budget_tokens * 3).max(120)
    }

    /// Coarse kind gate for callers that do not yet pick a lane.
    /// Prefer [`admits_dense`] / [`admits_sparse`] on the classified [`Slot`].
    /// New kinds default **out** (unknown → false).
    pub fn kind_included(&self, kind: &str) -> bool {
        match kind {
            "item/user" | "item/assistant" | "item/tool_call" => true,
            "turn/start" | "turn/end" => self.index_when,
            // tool_result / compacted / reminder / plan / unknown: not searchable
            _ => false,
        }
    }
}

/// Projection text under the product whitelist, or `None` when the slot is out.
///
/// Dense callers only keep Who/Said/Thought via [`admits_dense`]. Did text is
/// still produced here for sparse-adjacent helpers; Outcome is always `None`.
pub fn row_text_final(slot: Slot, text: &str, cfg: &SlotCfg) -> Option<String> {
    let budget = cfg.chars_budget();
    match slot {
        Slot::Summary | Slot::When | Slot::Other | Slot::Outcome => None,
        Slot::Who | Slot::Said | Slot::Thought => Some(trim_head_tail(text.trim(), budget)),
        // Intent call: keep signature + args; budget trim only (never envelope).
        Slot::Did => Some(trim_head_tail(text.trim(), budget * 3 / 2)),
    }
}

/// Tool name of a `function_call` row (`name({...})`), for outcome labelling.
pub fn tool_name_of(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let open = trimmed.find('(')?;
    let name = trimmed[..open].trim();
    if name.is_empty() || name.contains(char::is_whitespace) || name.len() > 40 {
        return None;
    }
    Some(name)
}

/// Keep head + tail, drop the middle — long text is usually boilerplate-headed
/// and evidence-tailed.
pub fn trim_head_tail(text: &str, max_chars: usize) -> String {
    let n = text.chars().count();
    if n <= max_chars {
        return text.to_string();
    }
    let head_len = max_chars * 2 / 3;
    let tail_len = max_chars - head_len;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text.chars().skip(n - tail_len).collect();
    format!("{head} …[省略 {} 字符]… {tail}", n - max_chars)
}

/// `bash({"command":"…"})` → `bash(command="…")`; long string args keep a real
/// head instead of a size marker, because that head is where the identifier, the
/// path or the command lives.
pub fn summarize_tool_call(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let Some(open) = trimmed.find('(') else {
        return trim_head_tail(trimmed, max_chars);
    };
    let name = trimmed[..open].trim();
    let args = trimmed[open + 1..].trim_end().trim_end_matches(')');
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(args) else {
        return trim_head_tail(&format!("{name}({})", args.trim()), max_chars);
    };
    let mut parts = Vec::new();
    for (key, value) in map.iter() {
        parts.push(match value {
            Value::String(s) => {
                let n = s.chars().count();
                if n <= 160 {
                    format!("{key}={s:?}")
                } else {
                    let head: String = s.chars().take(160).collect();
                    format!("{key}={head:?}…(+{})", n - 160)
                }
            }
            Value::Array(a) => format!("{key}=<array:{}>", a.len()),
            Value::Object(o) => format!("{key}=<obj:{}>", o.len()),
            other => format!("{key}={other}"),
        });
        if parts.len() >= 14 {
            break;
        }
    }
    trim_head_tail(&format!("{name}({})", parts.join(", ")), max_chars)
}

#[cfg(test)]
mod tests {}
