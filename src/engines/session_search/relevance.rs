//! The ranking judgment set: the fixed corpus and the queries whose *order* the
//! lane is expected to get right, with the metrics that say whether it did.
//!
//! Stage A of the ranking remediation. Every later change to a gate, a band or a
//! fusion weight has to keep this board green, which is the only reason the
//! numbers in `ranking::LAYERS` and `query_plan` are allowed to be numbers: they
//! are pinned against a corpus and a set of judgments, not against a feeling.
//!
//! The corpus is synthetic and fixed (session ids `S1`…`S7`, every row addressed
//! by `seq`), built through the real `sparse::build_index`, so it exercises the
//! real derivation — admission, roles, chunking — rather than a hand-written
//! index. The judgments are `query → (relevant rows, trap rows)`:
//!
//! * a **relevant** row is one a reader would have wanted;
//! * a **trap** is a row the old OR-everything ladder returned for that query and
//!   that a reader would not have wanted. Traps are what the gates exist for, so
//!   they are asserted to be *absent*, not merely ranked lower.
//!
//! Metrics are the standard ones: Recall@10, Precision@5, MRR and nDCG@10. They
//! are reported per query and asserted as floors, so a regression names the query
//! it broke instead of failing one aggregate number.

