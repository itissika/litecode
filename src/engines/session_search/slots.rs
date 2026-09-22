//! Session semantic slots — the first cut over the session corpus.
//!
//! The underlying goal of session retrieval in one line:
//! **when, who, how they thought / said it, what they did, how it went.**
//!
//! The corpus is cut by data type first (classify → trim → tag); dedup comes
//! later, as a consequence.
//!
//! # The final corpus policy (locked 2026-09-21)
//!
//! Four row classes, one treatment each — no per-tool logic, ever:
//!
//! 1. **人话（who / said / thought）** — kept whole, trimmed by budget only;
//!    never noise-stripped.
//! 2. **工具调用（did）** — kept: the call is the intent (which file / which
//!    command / which query). Trimmed by budget only; **never** collapsed to an
//!    envelope and never dropped as noise.
//! 3. **工具产出（outcome）** — kept, plain text, budget trim only. The semantic
//!    lane is wide recall; literal evidence questions are the sparse lane's job.
//!    No envelope rewriting, no per-tool classification.
//! 4. **压缩总结（summary）** — **excluded**. It is not first-party speech: the
//!    model wrote it after the fact, its span crosses turns, so a hit there
//!    resolves to a coordinate that does not correspond to where things happened.
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

/// Corpus policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    /// Frozen baseline: the five searchable kinds, raw item text, one row = one
    /// chunk, no trimming, no filter. Must stay byte-identical to what
    /// production embeds today; kept for comparison, never selected.
    V0Prod,
    /// The locked final corpus (see the module doc). 人话全留、工具调用留、
    /// 工具产出原样（只按预算截断）、压缩总结剔掉。
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

    /// Rows admitted at all. The union of the two production kind lists (the FTS
    /// lane indexes 8 kinds, the dense lane only 5 — a known inconsistency).
    pub fn kind_included(&self, kind: &str) -> bool {
        match kind {
            "item/user" | "item/assistant" | "item/tool_call" | "item/tool_result" | "compacted" => true,
            "plan/execute" | "reminder/job_exit" | "reminder/plan" => true,
            "turn/start" | "turn/end" => self.index_when,
            _ => false,
        }
    }
}

/// One row's document text under [`Policy::Final`], or `None` when the row is not
/// admitted at all.
///
/// 人话全留（只按预算头尾截断）；工具调用留（只按预算截断，绝不折叠成信封）；
/// 工具产出原样（只按预算截断，不做分类、不做信封）；压缩总结剔除。
pub fn row_text_final(slot: Slot, text: &str, cfg: &SlotCfg) -> Option<String> {
    let budget = cfg.chars_budget();
    match slot {
        Slot::Summary | Slot::When | Slot::Other => None,
        Slot::Who | Slot::Said | Slot::Thought => Some(trim_head_tail(text.trim(), budget)),
        // The call is the intent: file paths, commands, query words. Keep the
        // signature and the arguments; the bulk is what the budget trim handles.
        Slot::Did => Some(trim_head_tail(text.trim(), budget * 3 / 2)),
        // The outcome is kept as-is for coverage; it is not rewritten and not
        // classified. Literal evidence questions are the sparse lane's job.
        Slot::Outcome => Some(trim_head_tail(text.trim(), budget)),
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
mod tests {
    use super::*;

    #[test]
    fn classify_covers_every_durable_kind() {
        assert_eq!(classify("item/user", "message"), Slot::Who);
        assert_eq!(classify("item/assistant", "reasoning"), Slot::Thought);
        assert_eq!(classify("item/assistant", "message"), Slot::Said);
        assert_eq!(classify("item/tool_call", "function_call"), Slot::Did);
        assert_eq!(classify("item/tool_result", "function_call_output"), Slot::Outcome);
        assert_eq!(classify("compacted", "compacted"), Slot::Summary);
        assert_eq!(classify("turn/start", "turn/end"), Slot::When);
        assert_eq!(classify("plan/execute", "message"), Slot::Other);
    }

    #[test]
    fn final_policy_keeps_calls_and_drops_summaries() {
        let cfg = SlotCfg::default();
        // 人话：全留（短文本原样）。
        assert_eq!(
            row_text_final(Slot::Who, "  帮我查一下这个报错  ", &cfg).as_deref(),
            Some("帮我查一下这个报错")
        );
        // 工具调用：留，且不被折叠成信封（路径/命令头仍在）。
        let call = r#"edit({"file_path":"src/a.rs","new_string":"x"})"#;
        let out = row_text_final(Slot::Did, call, &cfg).expect("kept");
        assert!(out.contains("src/a.rs"), "{out}");
        // 工具产出：原样保留，不做信封。
        let dump = "src/a.rs:1:x\nsrc/a.rs:2:y\n";
        let out = row_text_final(Slot::Outcome, dump, &cfg).expect("kept");
        assert!(out.contains("src/a.rs:2:y"), "{out}");
        // 压缩总结：剔除。
        assert!(row_text_final(Slot::Summary, "conversation summary", &cfg).is_none());
    }

    #[test]
    fn long_prose_keeps_head_and_tail() {
        let cfg = SlotCfg { budget_tokens: 10, index_when: false };
        let text: String = (0..100).map(|i| format!("line-{i:03}\n")).collect();
        let out = row_text_final(Slot::Said, &text, &cfg).expect("kept");
        assert!(out.contains("line-000"));
        assert!(out.contains("省略"));
        assert!(out.chars().count() <= cfg.chars_budget() + 32);
    }

    #[test]
    fn tool_call_summary_drops_bulk_payload() {
        let raw = r#"edit({"file_path":"src/a.rs","old_string":"x","new_string":"y"})"#;
        let out = summarize_tool_call(raw, 600);
        assert!(out.starts_with("edit("), "{out}");
        assert!(out.contains("old_string=\"x\""), "{out}");
    }
}
