# WP-201 acceptance

**口径：** Andy + GM；`run_code` 人看/FE 聚合走稳定 `ToolCallResult.data`（succeeded/failed/duration_ms/by_tool + 可选 ok/failure_kind/unsettled）；模型 `content` 仍 WP-200 三块（head + 自愿 logs + result），**不**塞 by_tool。  
**分支：** `feat/tool-contract-ptc`（实验；**不合 main**）  
**方案：** `plan.md`（Q1–Q5 全认）  
**基线：** `7dd9617`（WP-200 tip-backfill）  
**本 tip：** `TIP_SHA_PLACEHOLDER`

| # | done_when | 结果 |
|---|-----------|------|
| 1 | 成功跑完的 `run_code` 出站带稳定 `data` 聚合；测例可证 | **通过**。`render::human_data` → `finish_run_result` 写 `result.data`；单测 `render_run_output_attaches_by_tool_data_without_leaking_into_content` 等可证形状。 |
| 2 | `content` 不出现按工具名计数明细；与 WP-200 模型面不冲突 | **通过**。负向断言：content 无 `by_tool` / `calls (` / `@@run_code_data`；Head 仍只成败数+ms。 |
| 3 | 无前端代码改动；`artifacts/WP-201` 含缺口说明；cargo 绿；未合 main | **通过**。**未改 `web/`**；见 `frontend-gap.md`；测数下；push 实验分支；**未合 main**。 |

## 测数（artifacts 日志；本收口未整仓重跑）

| 日志 | 结果 |
|---|---|
| `cargo-run-code-data.txt` | `tools::code::` **23 passed**（EXIT 0）+ `run_code_model_view` **17 passed**（EXIT 0）= **40** |

## 负向（content 不泄 by_tool）

`rg 'by_tool|calls \(|@@run_code_data' src/tools/code/render.rs src/tools/code.rs tests/run_code_model_view.rs`：默认渲染路径**不**输出这些串；仅 `human_data` 组 JSON + 负向 `assert!(!text.contains…)` / `assert!(!result.content.contains…)`。

## 前端消费缺口

见独立 `frontend-gap.md`（字段路径、示例 JSON、如何从 `ToolCallResult.data` 读到；本包不做 `web/` ToolView）。

## Digs

- `dig-runtime-impl.md`（Runtime 终态；含 FE 缺口原文）  
- `brief.md` / `plan.md`

## 已知推迟（非本包 blocker）

- Tools 教法 / SDK fixture：**本包不动**（人看字段不进模型 PRELUDE）。  
- `contract_matrix` / `phase1_gate` 仍有「run_code 无 data」旧断言——属 Tools/mid 后续对齐，非本 Runtime 包范围。

## 非目标 / 未入库

- **不合 main**；**不改 `web/`**  
- 未恢复 ledger 进模型视图  
- 未 stage ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`  
- 未入库 WP-193/195/196 无关 artifacts
