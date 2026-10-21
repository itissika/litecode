# WP-201 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `7dd9617`（WP-200 tip）  
**本包：** run_code 人看/FE 聚合 `ToolCallResult.data`；模型 content 不变。

## 产品改动（Runtime）

| 路径 | 变更 |
|---|---|
| `src/tools/code.rs` | `CallLedger.by_tool` + `note_settled`；`LedgerSnapshot`/`RunReport` 透传；`finish_run_result` 写 `result.data`；出口单测 |
| `src/tools/code/render.rs` | `human_data` / `by_tool_json` / `failure_kind_slug`；负向「content 无 by_tool」单测 |

## 未改

- **`web/`**（本包禁止）  
- Tools 教法 / `code_manifest` PRELUDE / `run_code_sdk` fixture  
- PTC / WP-200 模型面三块结构  

## 测数（不重跑；以 artifacts 为准）

- `cargo-run-code-data.txt`：lib `tools::code::` **23** + `run_code_model_view` **17** = **40 passed**  
- dig：`dig-runtime-impl.md`

## Artifacts

- `acceptance.md` / `frontend-gap.md` / `diff-summary.md`  
- `brief.md` / `plan.md` / `dig-runtime-impl.md` / `cargo-run-code-data.txt`

## 未做 / 不入库

- 不合 main；不改 `web/`  
- 未入库脏 ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`  
- 未入库 WP-193/195/196 无关 artifacts  
- Tools `contract_matrix`「run_code 无 data」旧断言推迟对齐
