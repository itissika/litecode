# WP-131 / IDE：引擎进度 code 侧 + 两库合并（seed）

接口与 GM 拍板见 WP-131-backend.md（冻结，不得改键名）。

## 范围
1. code IndexingProgress 增加 chunks_total 与 scanning phase（先切块得精确 total）。
2. 增量刷新持续上报（现状 files_total=0 只报一次）。
3. status_view.rs 合并 code+session 两语料，state 优先级 failed>needs_rebuild>refreshing>ready；Σ done/total/pending；任一 scanning ⇒ total.chunks_total=null；total.error=首个 failed。
4. 共享进度记录类型（session 复用）——**先落类型/签名并告知 Session**。
5. 合并单测（优先级 + Σ + scanning null）。
6. 删除估算：index_status.rs ~349-357 sync_pct_from_pending。
7. 模型缺失/引擎不可用/已停止 → failed + error{engine,reason,message}。
8. 工具 sync% 由新 total 计算。
9. 旧字段暂保留 deprecated，WP-131 封包前删除。

## 约束
不 commit 未批；LSP 不动；轮询；Warming 引擎级；不按语言特化；448/RRF 不动。

## done_when
code 语料 done≤total 单调、完成相等；运行中 total=0 ≤ 一个上报周期；合并单测绿；估算删除；云端绿。

---

## Delivery (IDE, WP-134) — 2026-10-14, not committed

### Final signatures (already sent to Session and Tools)
- `src/engines/progress.rs` (new, `pub mod progress;` at engines/mod.rs:10): CorpusState/CorpusPhase/ProgressEngine/FailReason/CorpusError/CorpusProgress/RetrievalProgress/merge_total; tools and the UI both use `CorpusProgress::percent()`.
- `src/engines/status_view.rs:204` `pub fn retrieval_progress_for_workspace(engines: &WorkspaceEngines, root: &Path) -> RetrievalProgress` (re-exported at mod.rs:13).
- JSON: `total` / `code` / `session` added as new keys on the existing retrieval object (status_view.rs:173); desired/state/usable/error/index are unchanged.

### Changes (code side + shared types only)
- index_status.rs: job file gains `chunks{done,total}` (null total = scanning); `update_chunk_progress` :238; `code_corpus_progress` :514 (model_missing → engine_unavailable → engine_stopped, only when desired=true → index_failed); `sync_pct_from_pending` estimate deleted (Ready/Stale with pending → 99 "syncing", an in-flight cap, not an estimate).
- build.rs: two passes. Pass 1 :61 chunks everything for an exact total (over 256 MB, :19, it counts only and re-chunks in pass 2 with deterministic ids); pass 2 reports after each EMBED_BATCH; done == total pinned at the end, :157.
- code_search/mod.rs:716 update_files: incremental denominator = whole index.
- **Precise tick signal (GM acceptance item)**: new `CodeQueue{queued, embed_dirty}` (`.litecode/code-index/queue.json`, index_status.rs:287) + `write_code_queue` :300 / `read_code_queue` :314. Every worker spot that changes the queue (mod.rs flush/refresh, fs_notify, reconcile) writes it explicitly; `index_work_from_disk` (:112, used by code_index_tick) and `chunks_pending` read it instead of `read_pending_hint`. Deleting pending_hint.json does not affect the tick.
- Old fields index.progress / sync_pct / indexed_chunks marked deprecated, **not deleted**.

### Known deviations (must read)
1. **`chunks_pending` actually counts files**: it is `embed_dirty`, the number of queued files needing embedding, not chunks. A per-file chunk count needs the worker to pre-chunk the queue; left for follow-up.
2. **Incremental refresh total drifts until the end**: total = indexed_chunks before the refresh + new chunks from the pre-chunked dirty files; the old chunks of re-indexed or deleted files are not subtracted, so total runs high mid-refresh (percent runs low) and is corrected to the real index size at the end. A full build is exact.
3. The session side is still the temporary adapter `session_corpus_progress` (status_view.rs:238), which only reports ready / needs_rebuild.

### For Session (WP-135)
The code side is covered. For the session tick signal I only provide the interface and did not touch semantic_index.rs or the session branch in engines/mod.rs: Session should, mirroring CodeQueue, persist an exact session queue (or report `CorpusProgress.chunks_pending`) and have `session_work_from_disk` read it instead of read_session_pending_hint, then replace session_corpus_progress.
### For Tools (WP-137)
When deleting pending_hint: test helper calls to `write_pending_hint(root, n)` must switch to `write_code_queue(root, &set)`. `write_pending_hint*` still mirrors into queue.json, so deleting the hint file alone does not break the tick.

### Evidence (measured; HOME=.tmp/home, timeout, </dev/null, target lock, -j2 --test-threads=1)
- `cargo test --lib engines` → `test result: ok. 300 passed; 0 failed; 1 ignored` (log .tmp/wp134/test_engines.log)
- `cargo test --lib tools` → `test result: ok. 448 passed; 0 failed; 0 ignored` (.tmp/wp134/test_tools.log)
- Acceptance tests with no pending_hint:
  - `engines::code_search::cool_tests::tick_drains_queue_without_pending_hint ... ok`: deletes pending_hint.json, enqueues added.rs → index_work_from_disk == Update{dirty:1} → refresh_index_incremental (the path the tick spawns) → queue.json cleared, Work::None, the new symbol is searchable.
  - `engines::tests::code_index_tick_sees_queue_without_pending_hint ... ok`: queue only, no hint → tick passes the Clean gate (in unit tests it stops at WorkerDown because no worker process runs).
- Others: progress::tests ×7, index_status::tests::{corpus_failed_reasons, corpus_job_scanning_then_embedding, estimate_helper_is_gone} all ok.
- Note: this run compiled together with other groups' (session/tools/web) uncommitted changes in the working tree.

---

## Rework (acceptance returned: incremental total ran high; chunks_pending was a file count) — 2026-10-14

### Changes
- `code_search/mod.rs:700` update_files is now two passes:
  - Pass 1 (:703, scanning, total=null): classify (deleted / non-indexable / missing / emptied → removal) and chunk every dirty file; done = kept = current index chunks − old chunks of removed and re-indexed files.
  - Pass 2 (:766, embedding): total = kept + new chunks (exact); done rises from kept, reported per file; at the end (:809) pinned to the real index size, done == total. A file whose embedding fails keeps its old chunks and the pin absorbs the difference.
  - refresh_index_incremental → sync_index_with_disk → flush → update_files goes through this same path.
- `progress.rs:91` CorpusProgress gains `files_pending: u64` (serde default 0; not in total; summed on merge); `with_files_pending()`, `has_pending()` :170; percent(): ready with pending files → 99, never 100.
- `code_corpus_progress`: files_pending = queue.json `queued`; chunks_pending is exact total−done only during embedding, otherwise 0. **Never a file count.**
- `.tmp/wp134_signature_final.md` updated to mark the new field.

### Evidence (measured)
- `cargo test --lib engines` → `test result: ok. 302 passed; 0 failed; 1 ignored`
  - `engines::code_search::cool_tests::incremental_refresh_total_exact_with_delete_and_reindex ... ok` (a.rs re-indexed and shrunk, b.rs deleted, d.rs added; trace[0]=(kept,None); first total == final index size; done monotonic, ≤ total, total stable; last=(n,Some(n)))
  - `engines::progress::tests::files_pending_sums_and_blocks_100 ... ok`
  - `tick_drains_queue_without_pending_hint`, `code_index_tick_sees_queue_without_pending_hint` still ok
- `cargo test --lib tools` → **compile failure (not caused by my code)**: `error[E0063]: missing field files_pending` at `src/tools/mod.rs:137` (Tools' uncommitted test helper `raw()` builds CorpusProgress as a struct literal). I cannot touch src/tools; Tools needs to add one line `files_pending: 0,` (or switch to the constructors).

### Status
Tools is not green, so **not delivered**; waiting for Tools to fix it, then rerun the tools tests → deliver → validate_all.

---

## Final verification (after Tools fixed raw(); 1.99 toolchain) — 2026-10-14

- Toolchain: `rustc 1.99.0 (b940084d7 2026-09-28)`, `cargo 1.99.0 (5f94df478 2026-08-27)`, `rustfmt 1.10.0-stable`; the ops default wrapper, nothing installed into ~/.rustup.
  - Note: my shell had stale RUSTUP_HOME/CARGO_HOME pointing at .tmp/home (an old 1.99 copy); unset them so the default wrapper applies. cargo 1.99 panics on a relative HOME (`... is not absolute`, manifest.rs:376), so HOME now uses the absolute path `$PWD/.tmp/home`.
- rustfmt: `rustfmt --check` only on the src/engines files I changed (progress.rs, status_view.rs, code_search/{index_status,build,mod,fs_notify,reconcile}.rs, engines/mod.rs); formatted per file via stdin only progress / index_status / build / code_search/mod (rustfmt recurses into child modules on a path, which would have hit Session's files). All 7 files clean. engines/mod.rs still has rustfmt diffs only in Session's WP-135 test `session_tick_drains_queue_without_pending_hint` (~L1729-1746), not mine, left untouched → for Session.
- `cargo test --lib tools` → `test result: ok. 450 passed; 0 failed; 0 ignored`
- `cargo test --lib engines` → `test result: ok. 306 passed; 0 failed; 1 ignored` (the ignored one = live_corpus_probe, needs a real workspace)
- Flaky test reported by Tools, `tools::bash::tests::live_small_echo_exit_code_first_file_on_disk_not_in_view`: passed in the full run, then 3 isolated runs `1 passed` ×3 (0.03s). Judgement (推断): intermittent, unrelated to progress — it is a bash tool live-process test that does not touch engines/progress/CorpusProgress; Tools' failure likely came from contention with concurrent cargo/process load. Recorded, not blocking.
- Evidence tests ok: incremental_refresh_total_exact_with_delete_and_reindex, files_pending_sums_and_blocks_100, tick_drains_queue_without_pending_hint, code_index_tick_sees_queue_without_pending_hint.

---

## Rework 2 (L3 / L5 / stuck Running / kept_chunks) — 2026-10-14, not delivered (per instruction), not committed

Toolchain: rustc 1.99.0 (b940084d7 2026-09-28), default ops wrapper; HOME=$PWD/.tmp/home, RUSTUP_HOME/CARGO_HOME unset, main target (on .tmp/cargo-target.lock), timeout, </dev/null.

### Changes
1. **L3** `engines/mod.rs` stop("code_search"): clears only the code side's `refresh_busy`, no longer touches `session_refresh_busy` (the session consume task clears its own flag when it ends). I made this change; test `engines::tests::stop_clears_only_its_own_busy_flag`.
2. **L5** `index_status.rs` `write_json`: **was not atomic** (direct `std::fs::write`), now writes a temp file in the same dir and then renames. Temp name = `.{name}.{pid}.{process-wide counter}.tmp`, so two writers in one process cannot collide. job.json / queue.json / pending_hint.json all go through it. Test `index_status::tests::job_and_queue_writes_are_atomic` (concurrent writes and reads, every read parses, no .tmp left behind).
3. **Stuck Running**
   - On failure, failed is written: the 3 error exits of build_full_index changed from clear_index_job to `mark_index_job_failed` (warmup_index / rebuild_index_in_runtime / spawn_scoped_refresh already wrote failed).
   - Per the mid-level ruling, stale-job cleanup **happens only in the worker load path**: `code_search_ipc/server.rs` initialize calls `reclaim_stale_index_job(root)` before warmup (no task of this worker exists yet). A stale Refreshing is cleared and re-queued (queue.json queued ≥ 1, so the next tick reconciles); a stale Building is just cleared (warmup rebuilds); Failed is kept. No pid, no file lock; the serve tick / start / status never clean up (an earlier version cleaned up in start() based on the in-process busy flag; removed).
   - Tests: `index_status::tests::stale_running_job_is_reclaimed_and_requeued`; `engines::tests::serve_tick_never_clears_worker_running_job` (job.json written by the worker, serve has no busy flag, 3 ticks, job still Refreshing).
4. **kept_chunks (aligned with Session)**: conclusion — **done would drop on the code side**: idle / begin_refreshing reported `indexed`, then pass 1 reported `kept = indexed − old chunks`. Fix:
   - `CodeQueue.kept_chunks: u64` (serde default 0), computed exactly by the worker from the loaded index at enqueue time (`CodeSearchRuntime::persist_queue`, called from fs_notify / reconcile / flush / refresh; it never reloads a cooled index; lock order pending→index); writers without index access keep the previous value, else fall back to the on-disk count.
   - While a queue exists, idle ready and begin_refreshing both report `done = kept_chunks`; refresh_index_incremental now reconciles first, then begin_refreshing, then flush, so the job starts already at the final kept.
   - Remaining boundary: done is fixed at enqueue; a new enqueue (more dirty files) may lower kept — that is a new enqueue event, consistent with Session.
   - Test `cool_tests::done_monotonic_from_enqueue_to_end` (re-index a, delete b, add d; from the idle view after enqueue through refresh and end, done never drops, ≤ total, job starts at kept, ends with done == total).

### fmt
Formatted only my files (per file via stdin, no recursion into Session modules); engines/mod.rs only my own hunk was changed by hand. Full-repo `cargo fmt --check`: **no diff** (whole repo).

### Tests (measured)
- `cargo test --lib engines` → `test result: ok. 317 passed; 0 failed; 1 ignored`
- `cargo test --lib tools` → `test result: ok. 450 passed; 0 failed; 0 ignored`

### Disk
`.tmp/wp134` 5.5G → 92K (deleted the separate target; logs kept). Main target is used from now on.

## WP-131 seal step: legacy field removal (IDE, 2026-10-14 ~01:30–02:00 HKT; ordered by backend_mid, ahead of WP-141)

### What was deleted
- engines detail (`status_view.rs` retrieval_detail): `index.status`, `index.progress`, `index.indexed_chunks`, `index.pending_updates`, `index.sync_pct`, `index.sync_label`, `index.session.pending_updates`. `index.created_at` kept (UI). The `"total"`/`"code"`/`"session"` CorpusProgress fields remain the only progress interface.
- `code_search/index_status.rs`: `SyncPct`, `sync_pct_from_view`, `sync_pct_from_progress` (the fixed 99 "syncing" branch), `PendingHintFile`, `pending_hint_path`, `write_pending_hint`, `write_pending_hint_from`, `write_pending_hint_counts`, `read_pending_hint`, `read_pending_embed_hint`, `read_pending_hint_file`. `resolve_index_view` Stale now reads `read_code_queue().queued`. Tests rewritten to `write_code_queue` (`queue_marks_stale_when_ready`, `queue_embed_dirty_keeps_deletes_on_update`); `removed_legacy_helpers_stay_gone` guards the names; the 3 SyncPct tests deleted.
- Callers moved to `write_code_queue`: code_search/mod.rs (warmup/rebuild/refresh/flush), reconcile.rs, fs_notify.rs, code_search_ipc/server.rs (:111, :260), engines/mod.rs tests.
- Session transition API `read_session_pending_hint` removed (Session files; diff in `.tmp/wp134/session_files.diff` for Session cross-review). Callers: code_search/mod.rs `has_pending_updates` → `read_session_queue().queued`; status_view key dropped; tests/code_search_ipc.rs → `read_session_queue(root).queued`.
- tests/optional_code_search.rs, tests/code_search_ipc.rs: `read_pending_hint(root)` → `read_code_queue(root).queued`. tests/settings_api.rs: `index.status` assertions → `retrieval.total.state` is string and `index.status` is null.
- src/tools: no references to removed symbols (its `sync_pct:` line is the WP-137 CorpusProgress format; untouched).

### rg evidence (after change)
```
$ rg -n "write_pending_hint|read_pending_hint|read_pending_embed_hint|PendingHintFile|pending_hint_path|read_session_pending_hint|write_session_pending_hint|SyncPct|sync_pct_from_view|sync_pct_from_progress|sync_pct_from_pending" src tests
exit=1   (no matches)
$ rg -n "\"(sync_pct|sync_label|indexed_chunks|pending_updates|progress)\":" src/engines/status_view.rs
exit=1   (no matches)
$ rg -n "pending_hint" src tests
only test names and absence assertions (pending_hint.json must not exist) + the dead-name guard's split literals:
src/tools/code_search.rs:580, src/engines/mod.rs:1726/1729/1754/1880/1882/1891, src/engines/code_search/mod.rs:1011/1014,
src/engines/code_search/index_status.rs:820/821, src/engines/session_search/semantic_index.rs:1999/2008
```

### Verification (rustc 1.99; HOME=$PWD/.tmp/home, RUSTUP_HOME/CARGO_HOME unset, main target under flock .tmp/cargo-target.lock, timeout, </dev/null, -j 2)
- `cargo test --lib -- engines:: tools:: workspace::watcher --test-threads=2`: 777 passed, 0 failed, 1 ignored.
- `cargo test --test code_search_ipc --test optional_code_search --test settings_api`: 11 + 19 + 48 passed, 0 failed.
- `cargo fmt --check` (whole repo): exit 0 (rustfmt run per file on our files only).
- Note: the working tree also carries the paused WP-141 WIP (stamps.v1 etc.); it compiles and its tests are inside the 777 above. Not committed.
