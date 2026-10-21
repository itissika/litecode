# WP-200 Runtime 实现笔记 — run_code 默认模型面硬清

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `c688b0a`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`、`dig-survey.md`；mid 拍板（对齐 dig-survey Q1–Q5 实现口径）  
> **硬约束遵守：** **200 cargo 绿**（见下）；**未 commit/push**；**未合 main**；**未改** Tools SDK fixture（`src/tool/fixtures/run_code_sdk.*`）

---

## 1. 关闭/替换层（一句话）

默认 `render_model_view` **只**三块：Head →（自愿）Logs →（自愿）Result。独立 failure / diagnostics / ledger / error_details 段及常量、渲染路径全部删除（不双轨开关）。内部 `CallLedger` / `diagnostics` 结构可留测与人看通道，**不进**默认 agent 面。

---

## 2. 行为（对照 mid 拍板）

| 项 | 口径 |
|---|---|
| **Head · 跑不起来** | Syntax / 建 runtime 等：`run_code failed: {label} · line N, column C · {snippet}: {cause}`；**不**堆 stack |
| **Head · 跑起来** | `run_code ok|failed: {tag} · {ok} succeeded, {fail} failed · {ms}ms`；过程失败标签（timeout / uncaught ToolError / exception…） |
| **成功/失败数** | **已结算**工具调用；`ok`/`ok(a/b)`→成功；`error:*`（caught reject）→失败；**unsettled 不计入**，Head 另起一句旁注 |
| **耗时** | `BudgetClock::wall_elapsed` → `Outcome.duration_ms`（墙钟 ms） |
| **Logs** | 仅脚本主动写出；空则整段省略；成功工具调用不自动占位 |
| **Result** | 按 `RUN_CODE_VIEW_MAX_BYTES` 窗；**失败不空抹**（`render_run_output` 有 value 即 present，含非 Cancelled 失败） |
| **session-read** | Head 追加 `· reads session store`（产品信号）；**废除**默认面对 ledger 行文本的依赖 |
| **lsp_error / edit_block_failed** | 默认面**完全不进**；需可见则脚本自 log/return |

Wire 层失败结果仍可能带信号前缀 `Error: `（既有 `finalize_signals`）；Head 正文本身以 `run_code ok|failed:` 起。

---

## 3. 删除清单（硬清）

| 类 | 已删/已停用（默认面） |
|---|---|
| failure 大段 | `render_failure`、`FAILURE_CAP`、正文/stack/caret 独立段 |
| diagnostics | `render_diagnostics`、`diagnostics:` 段、`DIAGNOSTICS_CAP` / `DIAGNOSTIC_RENDER_CAP` |
| ledger | `render_ledger`、`calls (N of M shown)`、`LEDGER_CAP`、逐 call 行 / omission |
| error_details | `render_error_details`、`ERRORS_CAP`、`#seq … code: message` 复述 |
| Header 旧句式 | `N calls (tool×k)`、`N rejected (…), handled by the program`、finished/unsettled 混进成功失败数 |
| keep_value 空抹 | 失败除有 value 外不再抹；Cancelled 与其它失败一视同仁保 partial |

**故意保留（内部/测）：** `LedgerSnap` / `CallLedger` / `aggregate_diagnostics` / `RunReport.diagnostics` 字段（不渲染）；`ledger_reads_sessions` 仍识别 Head 上的 `RUN_CODE_SESSION_NOTE`。

---

## 4. 改动文件列表（Runtime）

**主战场：**  
`src/tools/code/render.rs`（硬清 + Head/Logs/Result）  
`src/tools/code.rs`（`keep_value`、组装 `duration_ms`、outcome 测改写）  

**耗时管道：**  
`src/ptc/mod.rs`（`Outcome.duration_ms`）  
`src/ptc/engine.rs`（`wall_elapsed` 入 Outcome）  
`src/ptc/budget.rs`（无产品行为变更；曾试 active 后改回墙钟）  

**测改写：**  
`tests/run_code_model_view.rs`  
`tests/code_surface_phase2b.rs` / `phase3.rs` / `phase4.rs`（去掉 ledger 行断言，改 Head 计数 / 旁注 / 负向无 `calls (`）  

**产物：**  
`.litecode/ops/artifacts/WP-200/dig-runtime-impl.md`（本文件）  
`.litecode/ops/artifacts/WP-200/cargo-run-code-view.txt`

---

## 5. cargo（已跑）

```bash
LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2
# available ≈2.8Gi ≥2Gi；独占槽

cargo test --lib -- tools::code:: --test-threads=2
# EXIT_lib:0 → 18 passed

cargo test --test run_code_model_view -- --test-threads=2
# EXIT_mv:0 → 17 passed
```

合计 **35 passed**（lib 18 + integration 17）。tee → `artifacts/WP-200/cargo-run-code-view.txt`。

---

## 6. 残留 rg（默认面）

在 `src/tools/code/render.rs` 渲染路径中：

- **无** `diagnostics:` 段输出  
- **无** `calls (` ledger 段输出  
- **无** `render_failure` / `render_ledger` / `render_diagnostics` / `render_error_details`

`diagnostics:` / `calls (` 仅出现在**负向断言**（测例证明默认面不含）。

---

## 7. Tools SDK？

**需要 Tools 刷 SDK / 教法（是）。**  
Runtime **未**改 `src/tool/fixtures/run_code_sdk.*` / `code_manifest` PRELUDE。plan 顺序：Runtime 硬清 → Tools 对齐三块视图文案与 fixture（扫残留 ledger/diagnostics/failure 段教法）→ mid 收口。

---

## 8. 非目标 / 未做

- **未 commit / 未 push / 未合 main**  
- 未改 shell/edit/grep/glob/read 已定契约（phase 测仅改断言；partial_edit 测改为 `apply_edits` 以跟 WP-198）  
- 未改 live description / SDK fixture（归 Tools）
