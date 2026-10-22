//! Replaying session history to a model.
//!
//! 1. Provider item `id`s never go back on the wire, except a summary-replay
//!    host's own reasoning id. Codecs omit every other id. `call_id` is a
//!    pairing key, not an identity, and always survives.
//! 2. Reasoning ciphertext (`encrypted_content`) is readable only by the producer
//!    that minted it. Producer identity is the provider, the protocol, and the
//!    model when the origin recorded a `model_ref`. A row with no `endpoint_type`,
//!    or no `model_ref`, keeps the previous rule: it returns to the same provider
//!    on Responses and Chat, and is dropped for Messages.
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

use crate::authority::responses::Item;

/// Who produced one input item, read from the newest `request/header` at or
/// before its seq.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProducer {
    pub provider_id: String,
    /// Absent when the origin row predates protocol recording.
    pub endpoint_type: Option<String>,
    /// Catalog reference `{provider}/{model}`. Absent on older rows.
    pub model_ref: Option<String>,
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

/// Ciphertext belongs to this request when the provider, protocol, and model
/// match. A missing protocol, or a missing model on a non-Messages row, keeps
/// the rule from before those fields were recorded.
fn ciphertext_is_own(
    producer: &ItemProducer,
    provider_id: &str,
    endpoint_type: &str,
    model_ref: &str,
) -> bool {
    if producer.provider_id != provider_id {
        return false;
    }
    match producer.endpoint_type.as_deref() {
        None => endpoint_type != "messages",
        Some(kind) if kind != endpoint_type => false,
        Some(_) => match producer.model_ref.as_deref() {
            Some(minted) => minted == model_ref,
            None => endpoint_type != "messages",
        },
    }
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
                ciphertext_is_own(producer, provider_id, endpoint_type, model_ref)
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
            strip_foreign_ciphertext(&mut items, &producers, "p", "responses", "p/m")
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
    fn messages_signature_does_not_cross_models() {
        let mut items = vec![reasoning(Some("opus-sig"))];
        let producers = vec![Some(produced(
            "opencode",
            Some("messages"),
            Some("opencode/claude-opus-5-5"),
        ))];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut items,
                &producers,
                "opencode",
                "messages",
                "opencode/claude-sonnet-5-5",
            )
            .ciphertexts_stripped,
            1
        );
        assert!(matches!(
            &items[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));
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

    #[test]
    fn same_provider_ciphertext_does_not_cross_models() {
        let minted = vec![reasoning(Some("gpt-cipher"))];
        let producers = vec![Some(produced(
            "opencode",
            Some("responses"),
            Some("opencode/gpt-5.6-sol"),
        ))];
        let mut to_grok = minted.clone();
        assert_eq!(
            strip_foreign_ciphertext(
                &mut to_grok,
                &producers,
                "opencode",
                "responses",
                "opencode/grok-4.7",
            )
            .ciphertexts_stripped,
            1
        );
        assert!(matches!(
            &to_grok[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));
        assert!(matches!(
            &minted[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("gpt-cipher")
        ));
        let mut back = minted;
        assert_eq!(
            strip_foreign_ciphertext(
                &mut back,
                &producers,
                "opencode",
                "responses",
                "opencode/gpt-5.6-sol",
            )
            .ciphertexts_stripped,
            0
        );
        assert!(matches!(
            &back[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("gpt-cipher")
        ));
    }

    #[test]
    fn legacy_origin_without_model_still_replays_to_responses() {
        let mut items = vec![reasoning(Some("old"))];
        let producers = vec![Some(produced("p", Some("responses"), None))];
        assert_eq!(
            strip_foreign_ciphertext(&mut items, &producers, "p", "responses", "p/other")
                .ciphertexts_stripped,
            0
        );
        assert!(matches!(
            &items[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("old")
        ));
    }

    #[test]
    fn legacy_origin_without_protocol_still_replays_to_responses() {
        let mut responses = vec![reasoning(Some("old"))];
        let producers = vec![Some(produced("p", None, None))];
        assert_eq!(
            strip_foreign_ciphertext(&mut responses, &producers, "p", "responses", "p/m")
                .ciphertexts_stripped,
            0
        );
        assert!(matches!(
            &responses[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.as_deref() == Some("old")
        ));

        let mut messages = vec![reasoning(Some("old"))];
        assert_eq!(
            strip_foreign_ciphertext(
                &mut messages,
                &producers,
                "p",
                "messages",
                "p/claude-sonnet-5-5",
            )
            .ciphertexts_stripped,
            1
        );
    }

    #[test]
    fn messages_foreign_strip_counts_as_wholly_omitted() {
        let mut items = vec![reasoning(Some("opus-sig"))];
        let producers = vec![Some(produced(
            "opencode",
            Some("messages"),
            Some("opencode/claude-opus-5-5"),
        ))];
        let stats = strip_foreign_ciphertext(
            &mut items,
            &producers,
            "opencode",
            "messages",
            "opencode/claude-sonnet-5-5",
        );
        assert_eq!(stats.ciphertexts_stripped, 1);
        // Signature gone → Messages encode drops the whole thinking block.
        assert_eq!(stats.reasoning_wholly_omitted, 1);
        assert_eq!(crate::types::item_text_preview(&items[0]), "think");
    }

    #[test]
    fn responses_foreign_strip_keeps_text_not_wholly_omitted() {
        let mut items = vec![reasoning(Some("foreign"))];
        let producers = vec![Some(produced("q", Some("responses"), Some("q/m")))];
        let stats = strip_foreign_ciphertext(&mut items, &producers, "p", "responses", "p/m");
        assert_eq!(stats.ciphertexts_stripped, 1);
        // Text remains on the Item; Responses-with-text-replay can still use it.
        assert_eq!(stats.reasoning_wholly_omitted, 0);
        assert_eq!(crate::types::item_text_preview(&items[0]), "think");
    }
}
