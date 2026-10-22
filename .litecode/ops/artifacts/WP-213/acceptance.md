# WP-213 acceptance

## Tip / suite
- Branch `feat/tool-contract-ptc` (not merged to main).
- Full `cargo test -j 2`: **857 passed / 0 failed / 0 ignored** (see `cargo-full.txt`). Lib unit: 569.
- Flaky: none observed this run.

## What landed
1. **PRELUDE** rewritten to plan draft A (8 prose lines; 5402→1676 chars, −69%; 1303→379 cl100k tokens).
2. **`code` schema description** → plan draft B (134→40 tokens).
3. **Code-surface tool md** slimmed to ≤5 lines semantics + 1 example (18 files under `src/tools/descriptions/`).
4. **SDK fixture** refreshed: `run_code_sdk.txt` / `.sha256` (14126→7333 tokens).
5. **harness / system prompt: zero diff** — `src/config/global_db/prompts/harness/system.md`, `tools.md`, agent packs, `builtin_prompts` untouched.
6. **Runtime unchanged** — no edits under `src/tools/code/render.rs`, `src/ptc/`, ok/kill/rejection behavior.

## Protected / snapshot assertion changes
| item | change | why |
|---|---|---|
| `src/tool/fixtures/run_code_sdk.txt` + `.sha256` | full refresh | PRELUDE + tool md rewrite; `sdk_snapshot::phase0` compares exact text/hash |
| other protected string asserts | **none** | no test asserted on removed PRELUDE enum/truncation prose |

No new tests added (plan: do not pile tests).

## Tip unblock outside prompt scope (reported)
Commit `21286eb` (after WP-212 close) added `result_data: &[ToolResultData]` to `SessionManager::commit_turn_delta` but left `tests/working_set_identity.rs` on the 4-arg call. Tip would not compile the full suite. Minimal fix: pass `&[]` at the two call sites. Separately committed; not a product-behavior change.

## Leftovers for GM
- Ops board/log/`tool_traces.jsonl` dirty mixes from start/other agents — **not** committed here.
- Chat/Direct descriptions shortened via shared md (Q1 accepted); dual-track teaching out of scope.
