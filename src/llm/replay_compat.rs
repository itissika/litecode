//! Replaying session history to a model.
//!
//! 1. Provider item `id`s never go back on the wire, except a summary-replay
//!    host's own reasoning id. Codecs omit every other id. `call_id` is a
//!    pairing key, not an identity, and always survives.
//! 2. Reasoning ciphertext (`encrypted_content`) is readable only by the producer
//!    that minted it. Producer identity is the provider plus the protocol. A
//!    Messages signature is also bound to the model, so a different model on the
//!    same provider does not receive it. An origin row written before the
//!    protocol was recorded has no `endpoint_type`; that ciphertext still returns
//!    to the same provider on Responses and Chat, and is dropped for Messages.
//! 3. Reasoning text is never dropped by the ciphertext strip. Each codec writes
//!    it in its own dialect; a provider declaring `reasoning_replay` receives it
//!    on every turn.
//! 4. Summary replay keeps only reasoning this provider produced. Foreign
//!    reasoning has no id that host minted, and its input schema requires that
//!    id, so the item is removed on the request copy.
//!
//! The session log is never rewritten. Rules 2 and 4 run on the per-request copy.

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

/// Ciphertext belongs to this request when the provider and protocol match.
/// Messages also requires the same model. A missing protocol matches only a
/// non-Messages target, which is how rows written before this field existed
/// keep their previous behavior.
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
        Some(_) if endpoint_type == "messages" => producer.model_ref.as_deref() == Some(model_ref),
        Some(_) => true,
    }
}

/// Rule 2 on the request copy: remove reasoning ciphertext this request cannot
/// read. Text and summary stay. Returns how many ciphertexts were removed.
pub fn strip_foreign_ciphertext(
    items: &mut [Item],
    producers: &[Option<ItemProducer>],
    provider_id: &str,
    endpoint_type: &str,
    model_ref: &str,
) -> usize {
    let mut stripped = 0;
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
            strip_foreign_ciphertext(&mut items, &producers, "p", "responses", "p/m"),
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
            ),
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
            ),
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
            ),
            1
        );
        assert!(matches!(
            &items[0],
            Item::Reasoning(reasoning) if reasoning.encrypted_content.is_none()
        ));
    }

    #[test]
    fn legacy_origin_without_protocol_still_replays_to_responses() {
        let mut responses = vec![reasoning(Some("old"))];
        let producers = vec![Some(produced("p", None, None))];
        assert_eq!(
            strip_foreign_ciphertext(&mut responses, &producers, "p", "responses", "p/m"),
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
            ),
            1
        );
    }
}
