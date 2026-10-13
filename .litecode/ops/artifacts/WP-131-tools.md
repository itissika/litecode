# WP-137 交付（tools）· 工具 sync% 改用新 total，首行契约 v2

父：WP-132（WP-131 引擎进度条）。未 commit；未碰 src/engines/**。

## 改动（src/tools：mod.rs、code_search.rs、session_search.rs）
- `sync_pct_for_workspace` 读 `crate::engines::retrieval_progress_for_workspace(engines, &root).total`（`CorpusProgress`），无临时适配。
- 首行只由 `format_sync_pct_line` 生成，契约 v2（WP-057-sync-pct-contract.md，GM 拍板 B）：
  `sync_pct: <value>[ (<label>)][ chunks <done>/<total>]`
  - value ∈ `<n>%` | `scanning` | `failed` | `unknown`，全小写；n 取 `CorpusProgress::percent()`。
  - 只有 n% 才带 chunks 段；needs_rebuild（工具可见时）→ `sync_pct: unknown (needs_rebuild)`；failed 只写状态和原因，不给数字。
  - 防御：百分比钳到 ≤100，显示的 done ≤ total。
- 删除：`sync_pct: 0` 写死兜底、`format_index_progress`（files 比 / N files pending 估算）、`read_pending_hint`；tools 内对 `SyncPct`/`sync_label`/`sync_pct_from_view`/`read_pending_hint` 引用为 0（rg 验证）。SyncPct 类型本身由 IDE 后续删除。
- code_search 测试中的 `write_pending_hint` 改为 `write_code_queue`（WP-134 的 queue.json），测试意图不变。
- 测试 helper `raw()` 补 `files_pending: 0`（WP-134 新字段）。

## 测试
- 新增：`sync_line_percent_from_chunks`、`sync_line_scanning_has_no_fake_percent`、`sync_line_failed_has_no_number`、`sync_line_unknown_for_missing_progress_and_needs_rebuild`、`result_leads_with_sync_line`、`malformed_done_over_total_never_exceeds_100_or_total`、`malformed_scanning_or_odd_state_never_carries_chunks`。
- 语法正则与前端 WP-138 一致：`^sync_pct: (?:(\d{1,3})%|(scanning|failed|unknown))(?: \(([^()\n]+)\))?(?: chunks (\d+)\/(\d+))?$`；每次校验另断言 百分比 ≤100、非百分比行不带 chunks、done ≤ total。

## 验证（2026-10-14，WP-134/135 关闭后的当前树，默认 cargo 1.99.0）
- `cargo test --lib tools:: </dev/null`：436 passed，0 failed。
- `cargo fmt --check`：通过（全仓库与三文件均 rc=0）。
- 已知：`tool::registry::core_bindings_produce_full_builtin_list` 为既有 ISSUE-055，与本包无关。
