//! Replaying session history to a model.
//!
//! 1. Provider item `id`s never go back on the wire, except a summary-replay
//!    host's own reasoning id. Codecs omit every other id. `call_id` is a
//!    pairing key, not an identity, and always survives.
//! 2. Reasoning ciphertext (`encrypted_content`) is readable only by the producer
//!    that minted it. Producer identity is the provider, the protocol, the
//!    model when the origin recorded a `model_ref`, and the fixed prefix
//!    (instructions and tool list) when the origin recorded those digests: the
//!    same model under a different system prompt or tool set rejects the old
//!    signature. A row with no `endpoint_type`, or no `model_ref`, keeps the
//!    previous rule: it returns to the same provider on Responses and Chat, and
//!    is dropped for Messages. A row without prefix digests keeps the rule from
//!    before the prefix was recorded.
//! 3. Reasoning text is never dropped by the ciphertext strip. Each codec writes
//!    it in its own dialect; a target declaring `reasoning.replay = "text"`
//!    receives readable text from any producer.
//! 4. Summary replay keeps only reasoning this provider produced. Foreign
//!    reasoning has no id that host minted, and its input schema requires that
//!    id, so the item is removed on the request copy.
//! 5. A Messages signature stays valid while this request still sends the
//!    prefix it was signed against. Compaction or a stale-media note marks
//!    the reasoning item minted while the old content was still in that
//!    prefix, and only that signature is dropped. Later thinking was signed
//!    against the rewritten prefix and stays. Gateways do not reliably honor
//!    `drop_block`, so this cannot be left to the API.
//!
//! The session log is never rewritten. Rules 2, 4 and 5 run on the per-request
//! copy.

use sha2::{Digest, Sha256};

use crate::authority::responses::Item;
use crate::llm::request::ToolDef;

/// Who produced one input item, read from the newest `request/header` at or
/// before its seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProducer {
    pub provider_id: String,
    /// Absent when the origin row predates protocol recording.
    pub endpoint_type: Option<String>,
    /// Catalog reference `{provider}/{model}`. Absent on older rows.
    pub model_ref: Option<String>,
    /// Digest of the instructions the producing request sent. Absent on rows
    /// that predate prefix recording.
    pub instructions_digest: Option<String>,
    /// Digest of the tool list the producing request sent. Absent on rows that
    /// predate prefix recording.
    pub tools_digest: Option<String>,
}

/// The fixed prefix a request sends before any item: its instructions and its
/// tool list. A reasoning signature is only valid against the exact prefix that
/// minted it — the same model under a different system prompt or tool set is a
/// different request to the provider, and replaying the old signature is a 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedPrefix<'a> {
    pub instructions_digest: &'a str,
    pub tools_digest: &'a str,
}

/// SHA-256 digests of a request's fixed prefix, written to `request/header` and
/// compared on later turns. Durable, so it must not depend on process state.
/// The tool list is hashed in wire order: a reordered list is a different
/// prefix, even when it holds the same tools.
pub fn fixed_prefix_digests(instructions: &str, tools: &[ToolDef]) -> (String, String) {
    let mut hasher = Sha256::new();
    hasher.update(instructions.as_bytes());
    let instructions_digest = format!("{:x}", hasher.finalize());

    let mut hasher = Sha256::new();
    for tool in tools {
        hasher.update(tool.name.as_bytes());
        hasher.update([0]);
        hasher.update(tool.description.as_bytes());
        hasher.update([0]);
        hasher.update(tool.input_schema.to_string().as_bytes());
        hasher.update([0]);
    }
    let tools_digest = format!("{:x}", hasher.finalize());
    (instructions_digest, tools_digest)
}

/// The producer of each item: the newest `request/header` row at or before the
/// item's seq. An item without a seq, or older than every header, has no
/// provable producer.
///
/// `headers` is `(seq, producer)` in ascending seq order.
pub fn producers_for_seqs(
    seqs: &[Option<u64>],
    headers: &[(u64, ItemProducer)],
) -> Vec<Option<ItemProducer>> {
    seqs.iter()
        .map(|seq| {
            let seq = (*seq)?;
            let after = headers.partition_point(|(header, _)| *header <= seq);
            after.checked_sub(1).map(|index| headers[index].1.clone())
        })
        .collect()
}

/// Ciphertext belongs to this request when the provider, protocol, model, and
/// fixed prefix match. A missing protocol, or a missing model on a non-Messages
/// row, keeps the rule from before those fields were recorded; a row without
/// prefix digests keeps the rule from before the prefix was recorded.
fn ciphertext_is_own(
    producer: &ItemProducer,
    provider_id: &str,
    endpoint_type: &str,
    model_ref: &str,
    prefix: Option<FixedPrefix<'_>>,
) -> bool {
    if producer.provider_id != provider_id {
        return false;
    }
    let same_model = match producer.endpoint_type.as_deref() {
        None => endpoint_type != "messages",
        Some(kind) if kind != endpoint_type => return false,
        Some(_) => match producer.model_ref.as_deref() {
            Some(minted) => minted == model_ref,
            None => endpoint_type != "messages",
        },
    };
    if !same_model {
        return false;
    }
    let Some(prefix) = prefix else {
        return true;
    };
    let same_prefix = producer
        .instructions_digest
        .as_deref()
        .is_none_or(|minted| minted == prefix.instructions_digest)
        && producer
            .tools_digest
            .as_deref()
            .is_none_or(|minted| minted == prefix.tools_digest);
    same_prefix
}

/// Read-only counters from [`strip_foreign_ciphertext`]. Strip behaviour is
/// unchanged: ciphertext is cleared, text and summary stay on the Item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StripCiphertextStats {
    /// How many `encrypted_content` blobs were cleared.
    pub ciphertexts_stripped: usize,
    /// How many of those clearings leave no wire-usable thinking for this
    /// endpoint: Messages needs a signature (encode drops the block without
    /// one), and any endpoint with neither ciphertext nor retained text has
    /// nothing left to send. Does not change what strip mutates.
    pub reasoning_wholly_omitted: usize,
}

fn reasoning_has_text(reasoning: &crate::authority::responses::ReasoningItem) -> bool {
    use crate::authority::responses::{ReasoningItemContent, SummaryPart};
    let from_content = reasoning.content.as_ref().is_some_and(|parts| {
        parts.iter().any(|part| match part {
            ReasoningItemContent::ReasoningText(text) => !text.text.trim().is_empty(),
        })
    });
    if from_content {
        return true;
    }
    reasoning.summary.iter().any(|part| match part {
        SummaryPart::SummaryText(text) => !text.text.trim().is_empty(),
    })
}

/// Rule 2 on the request copy: remove reasoning ciphertext this request cannot
/// read. Text and summary stay. Returns strip counts only — never changes
/// keep/drop policy.
pub fn strip_foreign_ciphertext(
    items: &mut [Item],
    producers: &[Option<ItemProducer>],
    provider_id: &str,
    endpoint_type: &str,
    model_ref: &str,
    prefix: Option<FixedPrefix<'_>>,
) -> StripCiphertextStats {
    let mut stats = StripCiphertextStats::default();
    for (index, item) in items.iter_mut().enumerate() {
        let Item::Reasoning(reasoning) = item else {
            continue;
        };
        if reasoning.encrypted_content.is_none() {
            continue;
        }
        let own = producers
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|producer| {
                ciphertext_is_own(producer, provider_id, endpoint_type, model_ref, prefix)
            });
        if !own {
            reasoning.encrypted_content = None;
            stats.ciphertexts_stripped += 1;
            // Messages encode requires a signature; without it the whole
            // thinking block is omitted even when text remains on the Item.
            // Elsewhere, wholly-omitted means no text/summary left either.
            if endpoint_type == "messages" || !reasoning_has_text(reasoning) {
                stats.reasoning_wholly_omitted += 1;
            }
        }
    }
    stats
}

/// Rule 5 on the request copy: on a Messages target, remove the signature from
/// reasoning this request has marked. A mark means that item was minted while
/// the old prefix was still present. Later reasoning is left alone. `rebased`
/// is parallel to `items`. Returns how many signatures were removed.
pub fn strip_rebased_signatures(
    items: &mut [Item],
    rebased: &[bool],
    endpoint_type: &str,
) -> usize {
    if endpoint_type != "messages" {
        return 0;
    }
    let mut stripped = 0;
    for (item, rebased) in items.iter_mut().zip(rebased) {
        if !*rebased {
            continue;
        }
        let Item::Reasoning(reasoning) = item else {
            continue;
        };
        if reasoning.encrypted_content.take().is_some() {
            stripped += 1;
        }
    }
    stripped
}

/// Rule 4 on the request copy: drop reasoning this provider did not produce.
/// An unknown producer is foreign. Non-reasoning items stay. Returns how many
/// reasoning items were removed.
pub fn retain_own_reasoning(
    items: &mut Vec<Item>,
    producers: &[Option<ItemProducer>],
    provider_id: &str,
) -> usize {
    let mut dropped = 0;
    let mut kept = Vec::with_capacity(items.len());
    for (index, item) in items.drain(..).enumerate() {
        let foreign = matches!(item, Item::Reasoning(_))
            && !producers
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|producer| producer.provider_id == provider_id);
        if foreign {
            dropped += 1;
            continue;
        }
        kept.push(item);
    }
    *items = kept;
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{ReasoningItem, ReasoningItemContent, ReasoningTextContent};

    fn produced(provider: &str, endpoint: Option<&str>, model: Option<&str>) -> ItemProducer {
        ItemProducer {
            provider_id: provider.to_string(),
            endpoint_type: endpoint.map(str::to_string),
            model_ref: model.map(str::to_string),
            instructions_digest: None,
            tools_digest: None,
        }
    }

    fn tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    fn reasoning(encrypted: Option<&str>) -> Item {
        Item::Reasoning(ReasoningItem {
            id: Some("cc_rs_1".into()),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent {
                    text: "think".into(),
                },
            )]),
            encrypted_content: encrypted.map(str::to_string),
            status: None,
        })
    }

    #[test]
    fn producer_is_the_newest_header_at_or_before_the_item() {
        let headers = vec![
            (5, produced("a", Some("responses"), Some("a/m"))),
            (12, produced("b", Some("messages"), Some("b/claude"))),
        ];
        let producers = producers_for_seqs(&[None, Some(3), Some(5), Some(10), Some(14)], &headers);
        let ids: Vec<_> = producers
            .iter()
            .map(|producer| {
                producer
                    .as_ref()
                    .map(|producer| producer.provider_id.as_str())
            })
            .collect();
        assert_eq!(ids, vec![None, None, Some("a"), Some("a"), Some("b")]);
        assert_eq!(
            producers[4]
                .as_ref()
                .and_then(|producer| producer.model_ref.as_deref()),
            Some("b/claude")
        );
    }

    #[test]
    fn ciphertext_goes_back_only_to_its_producer_and_text_always_stays() {
        let mut items = vec![
            reasoning(Some("own")),
            reasoning(Some("foreign")),
            reasoning(Some("unknown")),
        ];
        let producers = vec![
            Some(produced("p", Some("responses"), Some("p/m"))),
            Some(produced("q", Some("responses"), Some("q/m"))),
            None,
        ];
        assert_eq!(
            strip_foreign_ciphertext(&mut items, &producers, "p", "responses", "p/m", None)
                .ciphertexts_stripped,
            2
        );
        let ciphertexts: Vec<_> = items
            .iter()
            .map(|item| match item {
                Item::Reasoning(r) => r.encrypted_content.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ciphertexts, vec![Some("own".into()), None, None]);
        assert!(
            items
                .iter()
                .all(|item| crate::types::item_text_preview(item) == "think")
        );
    }

    #[test]
    fn summary_replay_keeps_only_this_providers_reasoning() {
        let mut items = vec![
            reasoning(Some("own")),
            crate::types::user_text("stay"),
            reasoning(Some("foreign")),
            reasoning(None),
        ];
        let producers = vec![
            Some(produced(
                "aliyun-token",
                Some("responses"),
                Some("aliyun-token/qwen"),
            )),
            Some(produced(
                "aliyun-token",
                Some("responses"),
                Some("aliyun-token/qwen"),
            )),
            Some(produced("openai", Some("responses"), Some("openai/gpt"))),
            None,
        ];
        assert_eq!(
            retain_own_reasoning(&mut items, &producers, "aliyun-token"),
            2
        );
        assert_eq!(items.len(), 2);
        match &items[0] {
            Item::Reasoning(reasoning) => {
                assert_eq!(reasoning.encrypted_content.as_deref(), Some("own"));
            }
            other => panic!("expected own reasoning, got {other:?}"),
        }
        assert_eq!(crate::types::item_text_preview(&items[1]), "stay");
    }

    #[test]
    fn ciphertext_does_not_cross_protocols_on_the_same_provider() {
        let mut to_messages = vec![reasoning(Some("gpt-cipher")), reasoning(Some("claude-sig"))];
        let producers = vec![
            Some(produced(
                "opencode",
                Some("responses"),
                Some("opencode/gpt-6-luna"),
            )),
            Some(produced(
                "opencode",
                Some("messages"),
                Some("opencode/claude-sonnet-5-5"),
            )),
        ];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut to_messages,
                &producers,
                "opencode",
                "messages",
                "opencode/claude-sonnet-5-5",
                None,
            )
            .ciphertexts_stripped,
            1
        );
        assert!(matches!(
            &to_messages[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));
        assert!(matches!(
            &to_messages[1],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("claude-sig")
        ));

        let mut to_responses = vec![reasoning(Some("claude-sig"))];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut to_responses,
                &producers[1..],
                "opencode",
                "responses",
                "opencode/gpt-6-luna",
                None,
            )
            .ciphertexts_stripped,
            1
        );
        assert!(matches!(
            &to_responses[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));
    }

    #[test]
    fn the_fixed_prefix_decides_whether_a_signature_is_still_valid() {
        let base = vec![tool("read"), tool("grep")];
        let (instructions_digest, tools_digest) = fixed_prefix_digests("sys", &base);
        let mut producer = produced("p", Some("responses"), Some("p/m"));
        producer.instructions_digest = Some(instructions_digest.clone());
        producer.tools_digest = Some(tools_digest.clone());

        // Same provider, protocol, model and prefix: the signature still holds.
        let mut same = vec![reasoning(Some("sig"))];
        let prefix = FixedPrefix {
            instructions_digest: &instructions_digest,
            tools_digest: &tools_digest,
        };
        assert_eq!(
            strip_foreign_ciphertext(
                &mut same,
                &[Some(producer.clone())],
                "p",
                "responses",
                "p/m",
                Some(prefix),
            )
            .ciphertexts_stripped,
            0
        );

        // One more tool under the same model is a different prefix.
        let widened = vec![tool("read"), tool("grep"), tool("write")];
        let (_, widened_digest) = fixed_prefix_digests("sys", &widened);
        let mut tools_changed = vec![reasoning(Some("sig"))];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut tools_changed,
                &[Some(producer.clone())],
                "p",
                "responses",
                "p/m",
                Some(FixedPrefix {
                    instructions_digest: &instructions_digest,
                    tools_digest: &widened_digest,
                }),
            )
            .ciphertexts_stripped,
            1
        );
        assert!(matches!(
            &tools_changed[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));

        // Rewritten instructions are a different prefix too.
        let (edited_instructions, _) = fixed_prefix_digests("sys v2", &base);
        let mut instructions_changed = vec![reasoning(Some("sig"))];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut instructions_changed,
                &[Some(producer.clone())],
                "p",
                "responses",
                "p/m",
                Some(FixedPrefix {
                    instructions_digest: &edited_instructions,
                    tools_digest: &tools_digest,
                }),
            )
            .ciphertexts_stripped,
            1
        );
    }

    #[test]
    fn a_row_without_prefix_digests_keeps_the_rule_from_before_recording() {
        let legacy = produced("p", Some("responses"), Some("p/m"));
        let mut items = vec![reasoning(Some("sig"))];
        let (instructions_digest, tools_digest) = fixed_prefix_digests("sys", &[tool("read")]);
        assert_eq!(
            strip_foreign_ciphertext(
                &mut items,
                &[Some(legacy)],
                "p",
                "responses",
                "p/m",
                Some(FixedPrefix {
                    instructions_digest: &instructions_digest,
                    tools_digest: &tools_digest,
                }),
            )
            .ciphertexts_stripped,
            0
        );
    }

    #[test]
    fn a_cross_model_switch_drops_the_old_signature() {
        let tools = vec![tool("read")];
        let (instructions_digest, tools_digest) = fixed_prefix_digests("sys", &tools);
        let mut producer = produced("p", Some("responses"), Some("p/old"));
        producer.instructions_digest = Some(instructions_digest.clone());
        producer.tools_digest = Some(tools_digest.clone());
        let mut items = vec![reasoning(Some("sig"))];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut items,
                &[Some(producer)],
                "p",
                "responses",
                "p/new",
                Some(FixedPrefix {
                    instructions_digest: &instructions_digest,
                    tools_digest: &tools_digest,
                }),
            )
            .ciphertexts_stripped,
            1
        );
    }

    #[test]
    fn messages_signature_does_not_survive_a_rewritten_prefix() {
        let mut items = vec![
            reasoning(Some("before-compaction")),
            crate::types::user_text("stay"),
            reasoning(Some("after-compaction")),
        ];
        let rebased = [true, true, false];
        assert_eq!(
            strip_rebased_signatures(&mut items, &rebased, "messages"),
            1
        );
        assert!(matches!(
            &items[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));
        assert_eq!(crate::types::item_text_preview(&items[0]), "think");
        // Signed after the rewrite. The flag on the user message does not
        // take it with the earlier block.
        assert!(matches!(
            &items[2],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("after-compaction")
        ));
        assert_eq!(crate::types::item_text_preview(&items[2]), "think");

        let mut responses = vec![reasoning(Some("gpt-cipher"))];
        assert_eq!(
            strip_rebased_signatures(&mut responses, &[true], "responses"),
            0
        );
        assert!(matches!(
            &responses[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("gpt-cipher")
        ));
    }
}
