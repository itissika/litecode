//! Replaying session history to a model: three rules, nothing else.
//!
//! 1. Provider item `id`s never go back on the wire. Codecs omit them; `call_id`
//!    is a pairing key, not an identity, and always survives.
//! 2. Reasoning ciphertext (`encrypted_content`) is readable only by the provider
//!    that produced it, so it goes back only to that provider.
//! 3. Reasoning text is never dropped here. Each codec writes it in its own
//!    dialect; a provider declaring `reasoning_replay` receives it on every turn.
//!
//! The session log is never rewritten: rule 2 runs on the per-request copy.

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
        let Item::Reasoning(reasoning) = item else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{ReasoningItem, ReasoningItemContent, ReasoningTextContent};

    fn reasoning(encrypted: Option<&str>) -> Item {
        Item::Reasoning(ReasoningItem {
            id: Some("cc_rs_1".into()),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(ReasoningTextContent {
                text: "think".into(),
            })]),
            encrypted_content: encrypted.map(str::to_string),
            status: None,
        })
    }

    #[test]
    fn producer_is_the_newest_header_at_or_before_the_item() {
        let headers = vec![(5, "a".to_string()), (12, "b".to_string())];
        assert_eq!(
            producers_for_seqs(&[None, Some(3), Some(5), Some(10), Some(14)], &headers),
            vec![None, None, Some("a".into()), Some("a".into()), Some("b".into())]
        );
    }

    #[test]
    fn ciphertext_goes_back_only_to_its_producer_and_text_always_stays() {
        let mut items = vec![reasoning(Some("own")), reasoning(Some("foreign")), reasoning(Some("unknown"))];
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
        assert!(items.iter().all(|item| crate::types::item_text_preview(item) == "think"));
    }
}
