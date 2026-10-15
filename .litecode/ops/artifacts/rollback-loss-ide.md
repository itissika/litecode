# Rollback loss check — IDE domain (src/{workspace,terminal,lsp,engines})

**Checked:** 2026-10-15, about 01:45 HKT, by ide.

**Method, read-only throughout:**
- **Board:** read the board and wp.mjs, only through read commands.
- **git (read-only commands only):**
  - `log`, `log -S`, `grep HEAD`, `show --stat`, `diff --stat HEAD`, `status --short`
  - `apply --check` and `apply --check -R`, which write nothing
- **Nothing was written to src**, nothing was compiled, and no diff was applied.

## Bottom line
- **The IDE domain lost nothing.**
- **Work tree matches HEAD:** `git diff --stat HEAD -- src tests` is empty. HEAD = 3ddc643 (2026-10-14 15:59 +0800).
- **No in-flight code WPs:** of the WPs owned by ide on the board, none is in `review` or `in_progress` with src changes. Every one that changed src is `done`, and its commit is in HEAD.
- **Symbols checked:** every symbol named in each WP's artifact either exists in the work tree, or was deliberately removed or replaced by a later commit that is in HEAD. The "Symbols not found" column in the table explains each one.
- **Remaining diffs:** every diff that can still be found under `.tmp` is either already applied (`apply --check -R` passes) or superseded by later code in HEAD. None is needed for restoring.

## Table
| WP | Status | In HEAD | Lost in work tree | Symbols not found, and why | Diff usable for restoring (path, complete?, apply --check) | How to rebuild if there is no diff |
|---|---|---|---|---|---|---|
| WP-143 (WP-140 v2: first build/rebuild merged into incremental, per-batch saves, atomic writes) | done | Yes: af0963e "WP-139/140 v2 … atomic checkpoint". Its follow-ups 3ddc643 and 4a69eca also touch code_search and are in HEAD | **No.** `src/engines/code_search/atomic.rs` exists; `append_durable` is at atomic.rs:119; `checkpoint.json` code is at store.rs:36/355; `begin_refreshing` and `update_chunk_progress` are in use | `begin_building` and `update_build_progress` were **removed by WP-143 itself**. `tests/code_index_tick_death_list.rs:46/69` in HEAD asserts they must not reappear. `git log -S` shows the last change in af0963e | `.tmp/wp143/tools.diff` (src/tools/code_search.rs, test assertions only), complete: **apply --check fails forward and reverse (superseded).** HEAD code_search.rs:541-552 already contains the final form of both diffs (`begin_refreshing`, `update_chunk_progress(root, 4, Some(8))`, `chunks 4/8`). `.tmp/wp143/tools_v2.diff` (same file), same result: **superseded, already in HEAD in its later form.** Not needed | Not needed |
| WP-141 (WP-139: periodic disk scan + stamps persisted) | done | Yes: 7d737e0, af0963e | **No.** `queue_reconcile_from_stamps` (reconcile.rs:124), `code_index_tick` stat scan (engines/mod.rs:934), rescan subscriber (router.rs:230-249) | None | `.tmp/wp141/ide/router.diff` (src/serve/router.rs), complete: **forward check fails, reverse passes, so it is already applied** | Not needed |
| WP-134 (WP-131: engine progress, code side + merging the two stores) | done | Yes: 7d737e0 | **No** | `pending_hint_path`, `write_pending_hint*`, `read_pending_hint*`, `sync_pct_from_view/progress/pending` are the **legacy symbols the artifact lists for removal**. `git log -S` shows their last change in 7d737e0 (the removal) | `.tmp/wp134/session_files.diff` (session_search/semantic_index.rs, mod.rs; Session's files), complete: **reverse passes, already applied** | Not needed |
| WP-130 (WP-129/128: whitelist admission) | done | Yes: bd57f34 | No | None | None | Not needed |
| WP-079 (WP-077/076: 30 s silent incremental refresh for code + split code/session plans) | done | Yes: dc3db7b | **No.** `CODE_INDEX_TICK` (router.rs:35), `code_index_tick` (engines/mod.rs:934); test `code_index_tick_syncs_without_refresh` is in HEAD (tests/optional_code_search.rs) | `refresh_rebuilds_when_dirty_too_large` and similar old tests were removed on purpose (DirtyTooLarge abolished, dc3db7b/4f051a4). The catalog test names were renamed or rewritten later | None | Not needed |
| WP-074 (WP-071/070-045: language servers launched by their original path) | done | Yes: 466acad | **No** | `is_rustup_shim` and `rust_analyzer_runnable` were **removed by this WP** (the generic original-path launch replaced the Rust special case); last change 466acad. `which_in` and `src/lsp.rs`, `src/rust.rs`, `src/lsp_store.rs` are **Zed source references** in the artifact, not paths in this repo | None | Not needed |
| WP-073 (WP-071/070: deps probe with stdin null + hub −32801) | done | Yes: 466acad ("ContentModified copied from VS Code; probe stdin null") | **No.** `run_version_probe` uses `stdin(Stdio::null())` (deps.rs:423-); `LSP_CONTENT_MODIFIED` is at conn.rs:24 and the VS Code-style handling at conn.rs:51 / hub.rs:64/703/779 | `is_retryable_read_method` and `request_retry_modified` (the retry design in the artifact) **never appear in any commit**. It was replaced when the WP was merged on 2026-10-13 by "copy VS Code: return the empty default, no retry" (466acad). That predates the rollback, so it is **not a rollback loss** | None | Not needed (the replacement design is in HEAD) |
| WP-067 (WP-064/063: copy_path keeps symlinks, ISSUE-010) | done | Yes: 99a01f9 | No | `refuse_top_level_symlink_copy` / `refuses_to_copy_symlinks` are the old refusal behaviour, **removed by this WP** (last change 99a01f9) | None | Not needed |
| WP-060 (WP-058/057: Warming bucket, DirtyTooLarge abolished, searchable semantics) | done | Yes: a72ad6a | No | `sync_pct_from_view` was removed later in 7d737e0 (WP-134) | None | Not needed |
| WP-040 (WP-036: workspace concurrent overwrite + cross-device move) | done | Yes: in an early commit. Its tests `tests/wp028_probe_s1_xdev_move.rs` and `wp028_probe_s2_create_race.rs` are in HEAD; the artifact names no new src symbols | No | `refuse_top_level_symlink_copy` was removed by WP-067, as above | None | Not needed |
| WP-028 (WP-022: IDE base cleanup) | done | Yes: 2e75ac2 | No. Tests `workspace_file_crud`, `no_bare_canonicalize`, `death_list_src_pty_symbols_only_in_terminal_module` are in HEAD (tests/) | The other names were tests the WP proposed to cut or merge, already handled in 2e75ac2 | None | Not needed |
| WP-155 / WP-154 backend.md (research) | done / in progress | Not code | Not code | Never touched src | – | – |

## Every diff/patch found on the box
Search: `find /workspace -xdev -name '*.diff' -o -name '*.patch'`. Only these exist; none in `/workspace/wp155-recover` or anywhere else outside the repo.

| Path | What it touches | apply --check forward | apply --check reverse | Verdict |
|---|---|---|---|---|
| `.tmp/wp143/tools.diff` | src/tools/code_search.rs (WP-143 test assertions) | fail | fail | Superseded: HEAD already has the later form |
| `.tmp/wp143/tools_v2.diff` | src/tools/code_search.rs | fail | fail | Superseded: HEAD already has the later form |
| `.tmp/wp141/ide/router.diff` | src/serve/router.rs | fail | **pass** | Already applied |
| `.tmp/wp134/session_files.diff` | session_search (Session's) | fail | **pass** | Already applied |
| `.tmp/wp144r/wp144.diff` | session_search dense_parity / semantic_index (Session's WP-144) | fail | **pass** | Already applied |
| `.tmp/wp159/deps.diff` | web/package.json, package-lock.json (front end) | **pass** | fail | Not my domain: not applied, and it applies cleanly. Front end should check whether WP-159 needs it |
| `.tmp/wp159/web.patch` | web/ (front end) | not checked (not my domain) | – | Front end to check |

## Other observations
- **Untracked artifacts in `.litecode/ops/artifacts/`** (WP-154/, WP-155*, WP-156/, WP-157/, WP-151/158 files) are not in git. They are still present and are not code.
- **`target/` is entirely gone** (seen at 01:25 and still true). Running anything needs a full rebuild, which is heavy work and needs release.
