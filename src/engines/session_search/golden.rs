//! Stage A golden: the sparse lane's **derived artifact** and **query
//! contract**, pinned against one fixed corpus.
//!
//! This is the regression gate the lifecycle remediation (stages B–G) must keep
//! green. It freezes, in one place:
//!
//! * which source rows enter the index — sparse whitelist = 三态 + `tool_call`;
//!   `compacted`, session-echo results, and ordinary `tool_result` stay out;
//! * the chunk grid — `sid:seq` for a row that fits one chunk, `sid:seq#k`
//!   otherwise, tiling the row losslessly;
//! * the normalized text stored next to the original chunk text;
//! * the query ladder — exact substring 1.0, CJK substring via the trigram
//!   table, word-token match, and the exclusions that must stay unsearchable.
//!
//! The corpus uses synthetic session ids (`S1`, `S2`) and calls
//! `sparse::build_index` directly, so nothing here depends on the store's ULID
//! generation or on wall-clock ordering.

