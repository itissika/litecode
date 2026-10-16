# WP-167 ide.diff — F6 剔除确认（2026-10-15 13:22 HKT）

依据：`review_wp166.md` F6——旧 ide.diff 曾夹带 WP-166 的 `src/workspace/routes.rs` hunk，rebase 到 WP-166 之上会冲突/重复应用。

## 核查结果
对当前产物 `.litecode/ops/artifacts/WP-167/ide.diff`（1139 行）全文检索：
- **无** `diff --git a/src/workspace/routes.rs`
- **无** `routes.rs` / `mark_startup_settled` 字符串

文件列表仅：
- `src/engines/lsp_engine.rs`
- `src/engines/mod.rs`
- `src/engines/path_index/mod.rs`
- `src/engines/status_view.rs`
- `src/lsp/hub.rs`
- `src/lsp/mod.rs`
- `src/lsp/sweep_gate.rs`
- `tests/startup_order_death_list.rs`

**结论：当前 ide.diff 已不含 F6 夹带；无需再裁剪。** 本文件作为 rebase 卫生记录保留。`ide.diff` 本体未改字节（避免无意义 churn）。

合入基线：WP-166 已 done；上述 IDE 本域改动已在当前分支 tip `0554d72`（及更早的 05ab3c4 系）落地，见 `rebase-status.md`。
