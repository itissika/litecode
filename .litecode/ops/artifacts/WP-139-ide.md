# WP-139-ide：code 定期扫盘对账 + 印章持久化（IDE 执行）
上级：WP-140（backend_mid 批）。方案、文件边界、顺序、硬门、验收见 WP-139-backend.md。
- Phase 0：只出方案（不改码），写清接 WP-134 progress.rs / 新 pending 信号的位置、印章格式与失效规则、冷卸载策略选型、测试清单（含“只 stat”证明）。
- 编码须等 WP-134 验收后由 backend_mid 通知再 start。

---
# WP-141 delivery (IDE, 2026-10-14 HKT). Plan: .tmp/wp141_plan.md; rulings 1–5 by backend_mid applied.

## Changes (working tree, not committed)
- NEW `src/engines/code_search/stamps.rs`: `stamps.v1` in `.litecode/index/`. Header JSON {v=1, pipeline_version, embedder_id, canonical root, index_created_at=meta.created_at}; lines `mtime_ns\tlen\tpath` or `?\t?\tpath` (indexed, unstamped). Invalid → file deleted, every path unstamped (full read). Invalid = bad header/line, v≠1, pipeline/embedder/root differ, created_at≠meta, meta missing/needs_rebuild. Rebuild start discards it. Atomic temp+rename in the same dir. Written after every index commit (index first, stamps second).
- `code_search/reconcile.rs`: scan is stat-only. Removed the IndexCold early return (old :23). Loaded index: known = indexed paths ∪ stamped; cooled index: known = stamps.v1 keys (no reload); cooled + invalid stamps → in-memory `full_reconcile_on_load` flag, full reconcile at the next `ensure_index` load (no new state). `diff_against_stamps`: known−disk ⇒ deleted; new or mtime/len differ ⇒ modified; equal ⇒ skipped unread. Queue → same `pending_updates` → `queue.json` (WP-134 CodeQueue) → CorpusProgress.
- `code_search/build.rs`: `scan_stat_candidates` = Search walk with `skip_binary=false` + path filter + stat (size cap) — the binary sniff (opens files) moved to `update_files`; full build takes stamps before each read and discards stamps.v1 at start.
- `code_search/mod.rs`: FileStamp ms→ns; runtime seeds stamps from build or stamps.v1; `persist_stamps`; `update_files` pass 1 compares content (ruling 2): identical chunks ⇒ stamp refresh, zero embeds; empty / binary / too-large existing files are stamped (no churn every pass); pre-read stamp recorded on success.
- `code_search/store.rs`: build_stamps hand-off; `paths_with_identical_chunks` (one pass over the store).
- `engines/mod.rs` `code_index_tick`: calls `request_reconcile` (worker scan) before reading queue.json — same 30 s tick, no timer, no config. NotWarm/Stopped: no worker to scan; the worker's warmup reconcile covers it on start (ruling 5: cooled ⇒ scan+queue only, digest after load via the existing refresh path).
- `workspace/watcher.rs` + `workspace/service.rs`: notify Err no longer dropped (`notify_signal`), events with Rescan flag (inotify Q_OVERFLOW / FSEvents MustScanSubDirs / Windows overflow) → `rescan_tx` broadcast (no paths; UI change stream untouched).
- `serve/router.rs` (wiring only, ruling 1 — **for Session cross-review**, full diff `.tmp/wp141/ide/router.diff`, +27/−1):
  1. code tick: `engines.code_index_tick()` → `spawn_blocking(move || engines.code_index_tick()).await` (scan is blocking IPC; next tick waits).
  2. new task: `workspace.subscribe_rescan()` → on signal (or its own Lagged) `spawn_blocking { code_search().request_reconcile(); path_index().request_rescan(); }` — same entry as the existing IPC Lagged arm (router.rs:156-166, unchanged).

## Tests (new / changed)
reconcile.rs: index_cold_scan_queues_without_reload (cooled: queued exactly, index stays unloaded, queue.json (2,1), CorpusProgress files_pending=2, chunks_pending=0; digest after load); index_cold_without_stamps_reconciles_on_load; restart_uses_persisted_stamps_and_catches_offline_changes (offline add/modify/delete; **stat-only proof**: an unchanged chmod-000 file is not queued — any read would fail); touch_without_content_change_does_not_reembed (**embed calls = 0**, stamp refreshed); files_without_chunks_are_accounted_not_requeued (empty + binary); diff_is_stat_only_and_generic (no extension rules); crash_between_index_commit_and_stamps_rereads_never_misses (stamps discarded, 3 re-checked, 1 embed). stamps.rs: round trip; invalidation matrix (bad header, version, embedder, created_at, pipeline, meta gone, bad line) + root change. watcher.rs: rescan_flag_and_overflow_request_full_rescan (+ notify Err ⇒ rescan signal). service.rs: rescan_signal_reaches_subscribers. Lagged: existing router arm → request_reconcile → worker reconcile_disk → queue_reconcile_dirty (full scan); no new unit test for the router task itself (gap, noted).

## Verification (rustc 1.99; HOME=$PWD/.tmp/home, RUSTUP_HOME/CARGO_HOME unset, main target under flock .tmp/cargo-target.lock, timeout, </dev/null, -j2)
- `cargo test --lib -- engines:: tools:: workspace:: serve::`: 951 passed, 0 failed, 2 ignored (one = the manual bench).
- `--test code_search_ipc / optional_code_search / settings_api`: 11 / 19 / 48 passed.
- `cargo fmt --check`: exit 0.
- 448/RRF: `git diff -U0 -- src | rg "^[-+].*(448|RRF_K|SPARSE_WEIGHT|SEMANTIC_WEIGHT|DENSE_CHUNK_TOKENS|CHUNK_TOKENS)"` → exit 1 (no changed lines). Constants: sparse.rs:1639 CHUNK_TOKENS=448, semantic_index.rs:35 DENSE_CHUNK_TOKENS=448, ranking.rs:372 RRF_K=60.0, retrieve.rs:21 RRF_K=60, session_search/mod.rs:281/285 weights 1.0.
- No pending_hint in WP-141 code (rg exit 1).

## Measurement (pure stat scan, no index/model/reads)
- Repo: torvalds/linux shallow clone (depth 1) at `.tmp/wp141/linux`, commit 24a19b953e2a3d0ea5a8c4951a0ab8f32720c907 (Linux 7.3-rc7, 2026-10-11). **Clone deleted after the run.**
- Harness: `#[ignore] wp141_bench_stat_scan` (scan_stat_candidates + diff_against_stamps). **Debug test binary** (no release target on the box; numbers are an upper bound). Limits: `.tmp/wp102/run_limited.sh` only launches `litecode serve`, so `.tmp/wp141/ide/run_scan_limited.sh` applies the same rules (refuse if cargo/rustc running or MemAvailable<6GiB; prlimit AS=4GiB; taskset 0-3; RSS watchdog 3.5GiB). Cold = `sync; echo 3 > /proc/sys/vm/drop_caches` (sudo) before each of 5 processes; hot = warm-up then 5 runs in one process.
- Files scanned: 95,921 (after gitignore/excludes).
- Cold ms: 3795.9, 3523.4, 3592.7, 3845.4, 3603.1 → median 3603.1, max 3845.4.
- Hot ms: 3100.4, 3096.7, 2958.0, 2895.9, 2917.3 → median 2958.0, max 3100.4 (dirty=0 every run).
- Peak RSS (VmHWM): 61.9–62.6 MB cold, 76.2 MB hot process.
