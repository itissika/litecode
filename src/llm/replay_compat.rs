//! Replaying session history to a model.
//!
//! 1. Provider item `id`s never go back on the wire, except a summary-replay
//!    host's own reasoning id. Codecs omit every other id. `call_id` is a
//!    pairing key, not an identity, and always survives.
//! 2. Reasoning ciphertext (`encrypted_content`) is readable only by the provider
//!    that produced it, so it goes back only to that provider.
//! 3. Reasoning text is never dropped by the ciphertext strip. Each codec writes
//!    it in its own dialect; a provider declaring `reasoning_replay` receives it
//!    on every turn.
//! 4. Summary replay keeps only reasoning this provider produced. Foreign
//!    reasoning has no id that host minted, and its input schema requires that
//!    id, so the item is removed on the request copy.
//!
//! The session log is never rewritten. Rules 2 and 4 run on the per-request copy.

use crate::authority::responses::Item;

/// The provider that produced each item: the provider of the newest
/// `request/header` row at or before the item's seq. An item without a seq, or
/// older than every header, has no provable producer.
///
/// `headers` is `(seq, provider_id)` in ascending seq order.
pub fn producers_for_seqs(seqs: &[Option<u64>], headers: &[(u64, String)]) -> Vec<Option<String>> {
    seqs.iter()
        .map(|seq| {
            let seq = (*seq)?;
            let after = headers.partition_point(|(header, _)| *header <= seq);
            after.checked_sub(1).map(|index| headers[index].1.clone())
        })
        .collect()
}

/// Rule 2 on the request copy: remove reasoning ciphertext the target provider did
/// not produce. Text and summary stay. Returns how many ciphertexts were removed.
pub fn strip_foreign_ciphertext(
    items: &mut [Item],
    producers: &[Option<String>],
    provider_id: &str,
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
            .and_then(Option::as_deref)
            .is_some_and(|producer| producer == provider_id);
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
    producers: &[Option<String>],
    provider_id: &str,
) -> usize {
    let mut dropped = 0;
    let mut kept = Vec::with_capacity(items.len());
    for (index, item) in items.drain(..).enumerate() {
        let foreign = matches!(item, Item::Reasoning(_))
            && !producers
                .get(index)
                .and_then(Option::as_deref)
                .is_some_and(|producer| producer == provider_id);
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
        let headers = vec![(5, "a".to_string()), (12, "b".to_string())];
        assert_eq!(
            producers_for_seqs(&[None, Some(3), Some(5), Some(10), Some(14)], &headers),
            vec![
                None,
                None,
                Some("a".into()),
                Some("a".into()),
                Some("b".into())
            ]
        );
    }

    #[test]
    fn ciphertext_goes_back_only_to_its_producer_and_text_always_stays() {
        let mut items = vec![
            reasoning(Some("own")),
            reasoning(Some("foreign")),
            reasoning(Some("unknown")),
        ];
        let producers = vec![Some("p".to_string()), Some("q".to_string()), None];
        assert_eq!(strip_foreign_ciphertext(&mut items, &producers, "p"), 2);
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
            Some("aliyun-token".to_string()),
            Some("aliyun-token".to_string()),
            Some("openai".to_string()),
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
}
