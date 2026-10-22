# WP-206 acceptance

## Commits（feat/tool-contract-ptc，不合 main）
- C IDE #3#8：d36d11a
- A Runtime #1#2（同一 commit）：fb5e986
- D PTC #6#7：61cfd38
- B Tools #4#5：1350f56
- SDK fixture 刷新：385eb9e
- baseline read__empty fixture 对齐 Q6：3a35ec4

## 测数
全量 `cargo test -j 2 --lib`（tip 385eb9e，日志 cargo-full.txt）：2213 passed / 10 failed / 3 ignored。
3a35ec4 刷新 read__empty.json 后 `tools::baseline` 2/2 绿 → 剩 9 条失败，均为基线。

## 基线红（9 条）
- 已知 6：ptc::tests::stack_limit、ptc::tests::stack_overflow_stays_sealed_when_the_program_catches_it、tool::contract_matrix::phase5_contract_matrix_asserts_live_output、tool::phase1_gate 3 条（edit_lsp_error_reaches_outcome_and_run_code_data、run_code_calls_match_the_text_ledger_and_logs_keep_level、uncaught_tool_error_keeps_the_outer_code）。
- 新确认 3（worktree @236edfe 即 WP-206 改动前，独立 target，-j 2 对照，同样红 → 基线，非本包引入）：
  - tool::deletion_gate::phase0_direct_legacy_deletion_gate（缺 src/tools/edit/feedback.rs）
  - tools::phase3_gate::phase3_apply_edits_partial_reaches_run_code_ledger
  - tools::litecode_workspace::tests::refresh_reports_invalid_mcp_definitions_outside_the_accepted_diff

## 逐条对 brief 验收
1. run ~2s 命令直接返回 exit_code；>30s → null 可 wait/kill：A 组测试（cargo-A-jobs/runlock.txt），全量绿。✅
2. 4 并行非只读 run ≈1s：`run_lock_gate.rs` 测试，全量绿。✅
3. 索引未就绪三工具 reject unavailable：C 组测试绿（⚠ 仅纯函数测试，见限制）。✅
4. 越界 read resolve 空 lines + N 行 message；原测试翻转：B 组 + baseline fixture 3a35ec4。✅
5. glob.md basename 任意深度说明：已入 fixture。✅
6. bigint/symbol/function 带路径：D 组测试绿。✅
7. undefined 宽容 + head dropped 封顶；charter/PRELUDE/schema 一致：D 组测试绿；PRELUDE 文案已被 run_code_sdk.txt:13 覆盖，无漂移。✅
8. LSP 工作区内相对/外绝对：C 组测试绿。✅
9. SDK fixture+sha 刷新，sdk_snapshot 2/2 绿；cargo 日志入 artifacts。✅

## 限制
- LSP 冷启动只有纯函数测试（无真实语言服务器冷启动集成测试）。
- Windows symlink 路径（#8 相对化）待在 Windows 上复核。
- #2 实现偏离 plan：用 task_local `RunScope/SpawnLocks` 传递锁（非 ToolExecutionContext 字段），语义一致：锁只持到 spawn。
