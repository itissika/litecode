//! Equivalence gate: **a reconciled index must equal a clean rebuild**.
//!
//! The index is never told what changed. Every prepare diffs the final source
//! key set against what the index holds and repairs the difference, and a clean
//! rebuild derives the same rows from the same corpus. The moment those two
//! paths can disagree, a missed diff silently corrupts the index and the only
//! way back is a full rebuild — so the equivalence is pinned here, field by
//! field, on the derived artifact itself (`rows` and `source_state`).
//!
//! Both sides go through the same `derive_rows`; what differs is only *how much*
//! of the corpus each call sees. That is the real risk: a row that is skipped as
//! "unchanged" must be skipped because its derived bytes matched, never because
//! the incremental path forgot it existed. Revert, delete-tail, settle, and
//! mid-flight rows all land in this file for that reason.


