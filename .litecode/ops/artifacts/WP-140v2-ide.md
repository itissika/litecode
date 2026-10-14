# WP-143 / IDE — code first build / rebuild folded into the incremental path + save per batch + atomic writes (WP-140 v2 WP-A)

## Changes (uncommitted working tree)
- **New `src/engines/code_search/atomic.rs`** (v3 signature, shared with Session WP-144):
  - Functions: `AtomicPart`, `atomic_write_set`, `atomic_write_bytes`, `append_durable(data, committed_len, bytes) -> u64`, `append_fsync`, `truncate_to_committed`, `sweep_atomic_tmp`. Temp files are named `.<name>.tmp-<pid>-<seq>`.
  - **Round 3 addition:** when `append_durable`/`append_fsync` create a new file, they fsync the parent directory (test `append_syncs_parent_dir_only_when_creating_the_file`).
- **store.rs → append-only store:**
  - vectors.usearch is the snapshot. vectors.log holds `[id u64 LE][kind u8][384×f32]` records (kind 0 = delete). chunks.jsonl is append-only with `{"del":id}` tombstones.
  - Per batch: `append_durable` the logs → atomically write checkpoint.json `{vectors_log_len, chunks_len, snapshot_len, next_id}` → meta.
  - Compaction (save_full) runs when log bytes > max(snapshot, 1 MiB). Chunk lines are streamed through CountingSink and never buffered whole.
  - On load: sweep tmp files → cut the logs to the checkpoint → replay → R1 intersection repair (a chunk without a vector drops its whole file; orphan vectors rebuild the ANN).
- **mod.rs:**
  - `update_files` commits once per EMBED_BATCH (32). A file that spans batches is stamped only after its last batch (R2). Pass-1 text is held up to PASS1_HOLD_BYTES (256 MB); beyond that, files are re-chunked in pass 2.
  - `start_fresh_index` and `rebuild_index_in_runtime` = fresh index + `refresh_index_incremental`. warmup only loads or creates an empty index; the first build runs as a refresh job (status refreshing).
  - stamps.v1 is appended to (with `-\t-\tpath` tombstones), and created_at stays stable.
- **build.rs:** the full-build path is deleted.
- **index_status.rs:** the JobKind::Building writers `begin_building`/`update_build_progress` are deleted. JobKind::Building is kept so old job.json files can still be read and reclaimed; tests write a legacy job via `write_legacy_building_job`. IndexStatus::Building still has a live producer (Warming + Absent/NeedsRebuild), so it is kept, with no new or removed states.
- **engines/mod.rs:** Rebuild goes through begin_refreshing.
- **code_search_ipc/server.rs:** warmup always calls queue_reconcile_dirty.
- **Tests:** semantic_engine/retrieve/reconcile/fs_notify and both integration test files switched to `index_workspace_blocking` (the production path).
- **src/tools:** no changes here. **tools.diff and tools_v2.diff have been landed by Tools** (Tools ran its tests: 436 passed; the `chunks 4/8` assertion on the Warming path passes).
  - tools.diff: the Building-job assertions in the src/tools/code_search.rs test now use begin_refreshing + update_chunk_progress.
  - tools_v2.diff: `!msg.contains("4/8")` → `msg.contains("chunks 4/8")` (sync_pct contract v2: the exact chunk counts are on the sync line).

## The 34k gap: a real bug, fixed
- **Symptom:** the first full-repo run died with an allocation failure in the middle of a compaction. The resume then embedded 0 chunks, and the index stopped at 833,651 chunks / 91,934 files. A fresh build gives 867,698 / 95,874. A Python recount of chunk_file over the indexed files also gives 867,698, with 0 mismatched files. So the 34,047-chunk / 3,940-file gap was real loss.
- **Cause:** the crash happened between the compaction's renames. chunks.jsonl had already been replaced by the new, complete file, while checkpoint.json was still the old generation, so its chunks_len was shorter than the new file.
  1. Load cut the new file to the old length, losing the tail chunks.
  2. Their vectors became orphans; the ANN was rebuilt, but those paths could not be named.
  3. Their stamps were kept, so reconcile treated the files as clean and never re-embedded them.
- **Fix (store.rs load):**
  - `ckpt.snapshot_len != size of the actual snapshot` means another generation, so the load is marked suspect and nothing is truncated.
  - When the load is suspect or has orphans, stamps.v1 is discarded (the next reconcile re-reads every file; the content check keeps intact files at 0 embeds) and stamps_reset is set.
  - Language-generic, no new states, no estimates.
- **Test:** `crash_between_compaction_renames_keeps_chunks_and_vectors_one_to_one` for k=0..4, plus 30 extra files, asserting `more29.rs` is indexed after the crash. It fails on the old code and passes on the new.
- **Full-repo resume verification:** done after AS was raised to 6GiB with approval (section below): embed count equals the remaining chunks exactly, final 867,698. On the full repo, a fresh build matches the recount exactly (867,698).

## Measurements (Linux 24a19b953, 95,978 files; release, -j2 build, taskset 0-3, prlimit AS=4GiB, RSS watchdog 3.5GiB)
| Run | Embedder | Time | Embedded | Committed | Peak RSS | Notes |
|---|---|---|---|---|---|---|
| Subset kernel+mm, first build | hash | 8.8 s | 17,183 | 17,183 / 952 files | 145 MB | 329 saves, 8 full writes, 56 MB appended |
| Subset kernel+mm, first build | **prod (granite-97m ORT, 4 threads)** | 2,380 s (≈7.2 chunks/s) | 17,183 | 17,183 / 952 files | 2.70 GB | 342 saves, 8 full writes, 55 MB appended. RSS after load was only 0.26 GB; the growth comes from the ORT inference arena during embedding (VmSize 4.08 GB, close to the AS ceiling). |
| Full repo, first build | hash | ≈1,700 s (incl. a final reload that failed) | 867,698 | 867,698 / 95,874 files | 2.89 GB | |
| Full repo, killed at batch 13,500 | hash | 591 s | 432,000 | 431,998 / 47,474 files | 1.65 GB | 10,517 saves, 15 full writes, 1.54 GB appended |
| Resume #1 (after the kill) | hash | aborted at 06:10 (~19 min) | — | job reached 794,491 | 2.74 GB | SIGABRT: allocation failed under AS |
| Resume #2/#3 | hash | 527 s / ~11 min | 0 | — | 2.76 / 2.81 GB | the BM25 mmap returns ENOMEM under AS, ensure_index fails, nothing gets done |

(Superseded by the AS=6GiB full-repo verification below.) Earlier: the resume's "embed count == remaining chunks" could not be verified on the full repo, because under AS=4GiB a resume cannot load the full index plus BM25 (see below). It is covered by the unit tests `kill_resume_first_build_embeds_only_remaining` / `kill_resume_rebuild_embeds_only_remaining`, which assert that embed calls after a resume equal the remaining chunks exactly.

## Full-repo kill → resume verification (AS=6GiB, approved by 后端中层 08:08; RSS watchdog 3.5GiB unchanged; release build, -j2, hash embedder)
Clone again at 24a19b953 (95,978 files).
| Step | Time | Embedded | Committed | Peak RSS |
|---|---|---|---|---|
| First build, kill at batch 13,500 | 568 s | 432,000 | 431,997 / 47,325 files | 1.51 GB |
| **Resume** | **1,234 s** | **433,579** | 865,576 / 95,709 files | **2.87 GB** (VmSize 4.39 GB) |
| Delete stamps.v1 → full re-read (content check) | **724 s** | **0** | 865,576 (unchanged) | 2.86 GB |
| Restore the repo-root marker `.git` → resume | 671 s | 2,122 | **867,698 / 95,874 files** | 2.88 GB |

- **Embed count vs remaining chunks:** the resume embedded 433,579 = this corpus's total of 865,576 − the 431,997 committed at the kill. **Exact match, no repeats.** The kill run embedded 432,000 but committed 431,997. The 3 extra chunks belonged to a file that spans batches (R2): it is stamped only after its last batch, so those 3 are not counted as committed and are embedded again on resume. That is by design.
- **Why the corpus was 865,576 instead of 867,698:** after cloning I deleted `linux/.git`. The walker (ignore crate) then treated linux/ as a subdirectory of the parent repo /workspace/litecode and applied that repo's `.gitignore` (`target`, `*.log`, `dev`, …). Compared with the fresh build, that excluded 166 more files and 2,122 more chunks: 129 under `*/target/`, 24 `*.log`, `drivers/net/can/dev/*`, and so on. This was caused by my measurement setup, not by the resume. Evidence:
  1. The full re-read with stamps discarded found 0 changes, so the resumed index matched the corpus the scanner saw exactly.
  2. After restoring the `.git` marker, one more resume embedded exactly 2,122 chunks, and the final count was **867,698 chunks / 95,874 files**, identical to the fresh-build reference and to the Python recount.
- **Final count == 867,698: ✓**
- **Full re-read cost** (answering 后端中层): with a complete index and all stamps discarded, one full re-read takes 724 s (load ≈ 200 s + BM25 open + stat/read/chunk/content-compare of 95,756 files), embeds 0, with peak RSS 2.86 GB. This is the one-time cost after a suspect load. A normal resume or reconcile does not take this path.
- **Resume memory, measured again:** peak RSS 2.87 GB ≤ first build's 2.89 GB. Address space (VmSize) reached 4.39 GB, which is why AS=4GiB could not run it (see the analysis above). With AS=6GiB, all runs finished under the 3.5GiB RSS watchdog.

## Resume memory analysis (data from the bench's /proc/self/status phase marks)
| Phase (resume #3) | VmRSS | VmHWM | VmSize | VmPeak |
|---|---|---|---|---|
| After load | 2.36 GB | 2.36 GB | 2.98 GB | 3.05 GB |
| After opening BM25 (fails) | 2.59 GB | 2.65 GB | **4.16 GB** | 4.19 GB |
| End | 2.71 GB | 2.75 GB | 4.17 GB | 4.19 GB |

- **The resume's peak RSS is not higher than the first build's.** Resume 2.74–2.81 GB vs first build 2.89 GB.
- **The failure is about how AS counts memory:** prlimit AS limits virtual address space. Opening BM25 (tantivy MmapDirectory maps the 315 MB of segment files, plus reader/merge thread stacks and malloc arenas) adds +1.18 GB of VmSize but only +0.23 GB of RSS, hitting the 4 GiB ceiling, so mmap returns ENOMEM. Resume #1's SIGABRT is the same ceiling (a Rust allocation failure aborts).
- **Why the first build got through:** it starts from an empty index, and the ANN/chunks grow together with BM25. When the first build finished, it also failed the final reload ("load full index + open BM25") under the same AS limit. That is the same phenomenon.
- **The load does not read the whole index in one go:** chunks.jsonl (1.5 GB) is streamed line by line. Only vectors.usearch (579 MB) and vectors.log (≤ the snapshot) are read into a buffer transiently and freed right after the replay (VmPeak 3.05 GB vs VmSize 2.98 GB after load). Resident memory is set by the index size itself (ANN + chunk map ≈ 2.3 GB).
- **Not loosened:** the memory guard was left as is. To finish the full-repo resume verification, one option is AS=6GiB with the RSS watchdog kept at 3.5GiB. This needs the backend mid-level's approval.
- **Side finding (follow-up issue):** after a failed BM25 open, `ensure_bm25_open` runs a full resync rebuild on every call (about 2 minutes each at 794k chunks), and the errors from reconcile/update are swallowed (`let _ = update_files`). This takes up most of resume #2/#3's time. It is out of scope for this package; I suggest opening a separate ISSUE.

## Tests
- `cargo test --lib -- code_search`: 141 passed, 0 failed, 2 ignored (the bench is ignored)
- `--test optional_code_search` / `--test code_search_ipc`: 11 + 19 passed
- Death-list (build_full_index / begin_building / update_build_progress, etc. must not appear): workspace_hotswitch_death_list 2/2; `rg build_full_index|begin_building|update_build_progress` in src/tests = 0
- `rustfmt --check` on IDE files only; whole-repo `cargo fmt --check`: exit 0
- Full-repo lib suite (earlier): 8 failures in unrelated modules (lsp/git/path/terminal/registry), outside this package.

## Constraints
No commit, no checkout/stash of whole files; no Session or src/tools files touched; cargo always -j2, wrapper + flock + MemAvailable ≥ 6 GiB; the linux clone and other regenerable artifacts cleaned up when finished (only logs kept).
