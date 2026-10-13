# WP-141 / IDE — code index periodic disk reconcile + stamps.v1

## Follow-up: NotWarm serve-side scan (choice a) — 2026-10-14 01:53 HKT

Gap: `code_index_tick` returned `Skipped(NotWarm)` *before* `request_reconcile`, so a cold/NotWarm engine never wrote `queue.json` from disk changes.

Fix (a):
- New `code_search::queue_reconcile_from_stamps(root)` — pure-stat against `stamps.v1` in the serve process; writes `queue.json` only; no index load / embed. Missing stamps ⇒ no-op (leave prior queue).
- `code_index_tick`: always scan first (worker IPC if alive, else serve stamps scan); then if NotWarm return without spawning digest.
- Tests:
  - `engines::code_search::reconcile::tests::serve_scan_from_stamps_queues_without_runtime`
  - `engines::tests::code_index_tick_not_warm_queues_without_digest` (NotWarm + disk change ⇒ `files_pending>0`, state stays Warming, no worker)

Also confirmed: `status_view.rs` session block already serializes only `"work"` (lines 140–142); local `session_status` kept for `merge_retrieval_status` / `needs_rebuild`. No settings_api rerun (file untouched).

Checks: reconcile 10 ok / 1 ignored; code_index_tick* 4 ok; `cargo fmt --check` clean on touched files. No commit / no re-deliver.
