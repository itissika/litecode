//! Transcript token estimates for budget / row `token_estimate`.
//!
//! Text is counted with **tiktoken-rs cl100k_base**. Media costs come from
//! [`crate::session::media_tokens`] (same helpers as media_budget trim).
//!
//! **Forbidden:** `item_text_preview` or character-length heuristics as budget truth.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::authority::responses::{
    FunctionCallOutput, InputContent, Item, MessageItem, OutputMessageContent,
    ReasoningItemContent, SummaryPart,
};
use crate::session::media_tokens::input_content_media_tokens;

/// Per-tool cl100k slice (schema / call args / output). Not billed usage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolTokenRow {
    pub name: String,
    pub schema: usize,
    pub call: usize,
    pub output: usize,
}

/// Local cl100k buckets for occupancy mix (not provider ring truth).
///
/// Item text: `tool_call` / `tool_output` / `conversation`. Prompt overhead:
/// `system` (instructions) and `tool_schema` (tool JSON). JSON envelope and
/// tokenizer gap stay out — the UI residual (`used − classified_sum`) absorbs them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemTokenBreakdown {
    pub system: usize,
    pub tool_schema: usize,
    pub tool_call: usize,
    pub tool_output: usize,
    pub conversation: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub per_tool: Vec<ToolTokenRow>,
}

impl ItemTokenBreakdown {
    pub fn classified_sum(&self) -> usize {
        self.system
            .saturating_add(self.tool_schema)
            .saturating_add(self.tool_call)
            .saturating_add(self.tool_output)
            .saturating_add(self.conversation)
    }
}

/// Count tokens for a UTF-8 string with cl100k_base (OpenAI gpt-4 / 3.5 family encoding).
pub fn count_text_tokens(text: &str) -> usize {
    let bpe = tiktoken_rs::cl100k_base_singleton();
    bpe.encode_with_special_tokens(text).len()
}

fn add_input_content_tokens(content: &InputContent, total: &mut usize) {
    match content {
        InputContent::InputText(t) => *total += count_text_tokens(&t.text),
        InputContent::InputImage(_) | InputContent::InputFile(_) => {
            *total += input_content_media_tokens(content);
        }
    }
}

pub(crate) fn item_token_estimate(item: &Item) -> usize {
    match item {
        Item::Message(MessageItem::Input(msg)) => {
            let mut n = 0usize;
            for c in &msg.content {
                add_input_content_tokens(c, &mut n);
            }
            n
        }
        Item::Message(MessageItem::Output(msg)) => {
            let mut n = 0usize;
            for c in &msg.content {
                match c {
                    OutputMessageContent::OutputText(t) => n += count_text_tokens(&t.text),
                    OutputMessageContent::Refusal(r) => n += count_text_tokens(&r.refusal),
                }
            }
            n
        }
        Item::Reasoning(r) => {
            let mut n = 0usize;
            for part in &r.summary {
                let SummaryPart::SummaryText(t) = part;
                n += count_text_tokens(&t.text);
            }
            if let Some(content) = &r.content {
                for part in content {
                    let ReasoningItemContent::ReasoningText(t) = part;
                    n += count_text_tokens(&t.text);
                }
            }
            n
        }
        Item::FunctionCall(fc) => count_text_tokens(&fc.name) + count_text_tokens(&fc.arguments),
        Item::FunctionCallOutput(out) => match &out.output {
            FunctionCallOutput::Text(s) => count_text_tokens(s),
            FunctionCallOutput::Content(parts) => {
                let mut n = 0usize;
                for c in parts {
                    add_input_content_tokens(c, &mut n);
                }
                n
            }
        },
        // Other Item variants (tool search, computer, …) are rare in the agent transcript;
        // serialize body and tokenize as a last-resort lower bound so budget still moves.
        other => serde_json::to_string(other)
            .map(|s| count_text_tokens(&s))
            .unwrap_or(1),
    }
}

/// Structured Item token estimate (cl100k_base text + shared media helpers).
pub fn compute_token_estimate(items: &[Item]) -> usize {
    if items.is_empty() {
        return 0;
    }
    items.iter().map(item_token_estimate).sum::<usize>().max(1)
}

/// Same units as [`compute_token_estimate`], split by Item kind for the occupancy bar.
pub fn compute_token_breakdown(items: &[Item]) -> ItemTokenBreakdown {
    let mut bd = ItemTokenBreakdown::default();
    let mut call_names: HashMap<String, String> = HashMap::new();
    let mut per_tool: HashMap<String, ToolTokenRow> = HashMap::new();

    for item in items {
        if let Item::FunctionCall(fc) = item {
            if !fc.call_id.is_empty() {
                call_names.insert(fc.call_id.clone(), fc.name.clone());
            }
        }
    }

    for item in items {
        let n = item_token_estimate(item);
        match item {
            Item::FunctionCall(fc) => {
                bd.tool_call = bd.tool_call.saturating_add(n);
                let row = per_tool
                    .entry(fc.name.clone())
                    .or_insert_with(|| ToolTokenRow {
                        name: fc.name.clone(),
                        ..ToolTokenRow::default()
                    });
                row.call = row.call.saturating_add(n);
            }
            Item::FunctionCallOutput(out) => {
                bd.tool_output = bd.tool_output.saturating_add(n);
                let name = call_names
                    .get(&out.call_id)
                    .cloned()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "unknown".into());
                let row = per_tool
                    .entry(name.clone())
                    .or_insert_with(|| ToolTokenRow {
                        name,
                        ..ToolTokenRow::default()
                    });
                row.output = row.output.saturating_add(n);
            }
            _ => bd.conversation = bd.conversation.saturating_add(n),
        }
    }

    let mut rows: Vec<ToolTokenRow> = per_tool.into_values().collect();
    rows.sort_by(|a, b| {
        b.call
            .saturating_add(b.output)
            .cmp(&a.call.saturating_add(a.output))
            .then_with(|| a.name.cmp(&b.name))
    });
    bd.per_tool = rows;
    bd
}

/// Fold instructions + per-tool schema JSON into an item breakdown (same cl100k units).
pub fn apply_prompt_overhead(
    bd: &mut ItemTokenBreakdown,
    instructions: &str,
    tool_schemas: &[(String, usize)],
) {
    bd.system = count_text_tokens(instructions);
    bd.tool_schema = 0;
    for (name, n) in tool_schemas {
        bd.tool_schema = bd.tool_schema.saturating_add(*n);
        if let Some(row) = bd.per_tool.iter_mut().find(|r| r.name == *name) {
            row.schema = *n;
        } else if *n > 0 {
            bd.per_tool.push(ToolTokenRow {
                name: name.clone(),
                schema: *n,
                call: 0,
                output: 0,
            });
        }
    }
    bd.per_tool.sort_by(|a, b| {
        b.call
            .saturating_add(b.output)
            .cmp(&a.call.saturating_add(a.output))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Autocompact fires when estimated tokens exceed this fraction of the model context window.
///
/// `0.8` leaves headroom for the model reply / tool schemas while still compacting before
/// hard context overflow. Threshold and [`compute_token_estimate`] share the same token units.
pub fn autocompact_threshold(context_window: usize) -> usize {
    (context_window as f64 * 0.8) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, InputImageContent,
        InputTextContent,
    };
    use crate::session::media_tokens::IMAGE_FALLBACK_TOKENS;
    use crate::types::user_text;

    #[test]
    fn cl100k_gold_sample_hello_world() {
        // Documented gold: cl100k_base encodes "hello world" as exactly 2 tokens.
        assert_eq!(count_text_tokens("hello world"), 2);
    }

    #[test]
    fn text_token_count_handles_utf8_and_grows_monotonically() {
        let ascii = count_text_tokens("needle");
        let chinese = count_text_tokens("你好，世界");
        assert!(ascii > 0 && chinese > 0);
        assert!(count_text_tokens("needle needle") > ascii);
    }

    #[test]
    fn estimate_grows_with_text() {
        let short = vec![user_text("hi")];
        let long = vec![user_text("x".repeat(400))];
        assert!(compute_token_estimate(&long) > compute_token_estimate(&short));
    }

    #[test]
    fn media_adds_tokens_beyond_text_only() {
        let text_only = vec![Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Content(vec![InputContent::InputText(InputTextContent {
                text: "caption".into(),
            })]),
            id: None,
            status: None,
        })];
        let with_image = vec![Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Content(vec![
                InputContent::InputText(InputTextContent {
                    text: "caption".into(),
                }),
                InputContent::InputImage(InputImageContent {
                    detail: Default::default(),
                    file_id: None,
                    image_url: Some("https://example.com/a.png".into()),
                }),
            ]),
            id: None,
            status: None,
        })];
        let text_n = compute_token_estimate(&text_only);
        let media_n = compute_token_estimate(&with_image);
        assert!(media_n >= text_n + IMAGE_FALLBACK_TOKENS);
    }

    #[test]
    fn empty_transcript_is_zero() {
        assert_eq!(compute_token_estimate(&[]), 0);
    }

    #[test]
    fn breakdown_splits_tool_output_from_conversation() {
        let items = vec![
            user_text("hello world"),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: "c1".into(),
                output: FunctionCallOutput::Text("x".repeat(400)),
                id: None,
                status: None,
            }),
        ];
        let bd = compute_token_breakdown(&items);
        assert!(bd.tool_output > bd.conversation);
        assert_eq!(bd.classified_sum(), compute_token_estimate(&items));
    }

    #[test]
    fn breakdown_groups_output_by_tool_name() {
        let items = vec![
            Item::FunctionCall(FunctionToolCall {
                arguments: r#"{"cmd":"ls"}"#.into(),
                call_id: "c1".into(),
                name: "bash".into(),
                namespace: None,
                id: None,
                status: None,
            }),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: "c1".into(),
                output: FunctionCallOutput::Text("x".repeat(400)),
                id: None,
                status: None,
            }),
        ];
        let bd = compute_token_breakdown(&items);
        assert!(bd.tool_output > bd.tool_call);
        assert_eq!(bd.per_tool.len(), 1);
        assert_eq!(bd.per_tool[0].name, "bash");
        assert_eq!(bd.per_tool[0].call, bd.tool_call);
        assert_eq!(bd.per_tool[0].output, bd.tool_output);
    }

    #[test]
    fn prompt_overhead_fills_system_and_schema() {
        let mut bd = compute_token_breakdown(&[]);
        apply_prompt_overhead(
            &mut bd,
            "you are a helpful assistant",
            &[("bash".into(), 12)],
        );
        assert!(bd.system > 0);
        assert_eq!(bd.tool_schema, 12);
        assert_eq!(bd.per_tool[0].name, "bash");
        assert_eq!(bd.per_tool[0].schema, 12);
    }

    #[test]
    fn breakdown_empty_is_zero() {
        assert_eq!(compute_token_breakdown(&[]), ItemTokenBreakdown::default());
    }
}
