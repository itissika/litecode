//! Seq-only provenance helpers for compaction checkpoints.

use crate::session::event::Seq;

/// Per item: persisted before the checkpoint seq, so it was produced on the
/// history that checkpoint replaced. Identity is the checkpoint seq; summary
/// text never participates.
pub fn minted_before_summary(seqs: &[Option<Seq>], checkpoint_seq: Option<Seq>) -> Vec<bool> {
    let Some(checkpoint_seq) = checkpoint_seq else {
        return vec![false; seqs.len()];
    };
    seqs.iter()
        .map(|seq| seq.is_some_and(|seq| seq < checkpoint_seq))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_uses_checkpoint_seq_not_summary_text() {
        assert_eq!(
            minted_before_summary(&[Some(191), Some(136), None, Some(195)], Some(191)),
            vec![false, true, false, false]
        );
        assert_eq!(
            minted_before_summary(&[Some(1), Some(0)], None),
            vec![false, false]
        );
    }

    #[test]
    fn pending_rows_never_inherit_a_signature() {
        assert_eq!(
            minted_before_summary(&[Some(8), None, Some(9)], Some(8)),
            vec![false, false, false]
        );
    }
}
