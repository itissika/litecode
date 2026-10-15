# WP-167 deliver 笔记（exact 验证通过）

日期：2026-10-15 13:24–13:25 HKT  
基线 tip：`0554d72`（`ops/wp-172-actions-ci`，**未 push `ops/team-framework`**）  
占槽口径：`CARGO_BUILD_JOBS=1`，起步 MemAvailable ≈6.3GiB（≥2GiB），结束 ≈6.3GiB。

## 可 deliver？

**可以。** 放行的 exact 测例 + 可选两项 **全部 exit=0**。接线已在 tip；F6（routes.rs）产物已确认干净；READY / health / session/new 本域未改。

合入策略按中层：以 tip `0554d72` 为工作基线，**不要 push `ops/team-framework`**；正式合入路径另议。

## 测例结果

| # | 命令 | 结果 | 耗时 |
|---|------|------|------|
| 1 | `cargo test --test startup_order_death_list` | **9 passed** | ~15s |
| 2 | `cargo test --lib -- sweep_gate spawn_ls_waits_for_sweep_gate` | **5 passed** | ~17s |
| 3 | `cargo test --lib optional::tests::reconcile_starts_warmup_for_network_core -- --exact` | **1 passed** | ~0s |
| 可选 | `cargo test --test optional_p0 network_core_engines_always_warmup -- --exact` | **1 passed** | ~2s |
| 可选 | `cargo test --test optional_p0 settings_reload_reconciles_engine_manager -- --exact` | **1 passed** | ~1s |

失败清单：**无**。

death_list 含：`serve_run_does_not_sweep_inline`、`serve_run_arms_sweep_before_ready_path`、`serve_run_does_not_warm_optional_engines`、`every_spawn_waits_for_the_sweep_gate`、`sweep_gate_has_no_timeout` 等。

## 产物路径

| 路径 | 用途 |
|------|------|
| `.litecode/ops/artifacts/WP-167/exact-verify.log` | 本回合完整日志 |
| `.litecode/ops/artifacts/WP-165/exact-verify.log` | 同内容副本 |
| `.litecode/ops/artifacts/WP-167/ide.F6-scrubbed.md` | F6/routes 剔除确认 |
| `.litecode/ops/artifacts/WP-167/cross-domain/session.diff` | 已刷新的 run.rs 跨域 diff |
| `.litecode/ops/artifacts/WP-167/ide.diff` | IDE 本域（无 routes） |
| `.litecode/ops/artifacts/WP-165/fix.diff` / `followup.diff` / `rebase-status.md` | WP-165 接线与状态 |
| `.litecode/ops/artifacts/WP-167/deliver-notes.md` | 本文件 |

## 范围回顾（deliver 声明）

- **在 tip**：`start_background`、`defer_reconcile`、sweep 耗时日志、sweep_gate 非 serve 注释、death_list、hub `wait()`、WP-167 IDE 门闩系。
- **未碰**：READY 打印时机、`/health`、`session/new`。
- **WP-171 测例大包**：另排；本 deliver 不依赖再改它们（tip 已 fold 的保持原样）。

## 建议 review / board

- WP-167：可标 deliver → review（owner ide → reviewer 按板面）。
- WP-165：接线已随 tip 验证；可与 167 一并收或等合入策略。
- **禁止**本 agent push `ops/team-framework`。
