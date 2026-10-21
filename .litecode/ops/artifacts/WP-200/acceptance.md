# WP-200 acceptance

**口径：** Andy + GM 硬清；`run_code` 默认模型面只留 **head +（自愿）logs + result**；砍独立 failure / diagnostics / ledger / error_details 观测层；学 pi 宽容（失败保 partial）。  
**分支：** `feat/tool-contract-ptc`（实验；**不合 main**）  
**方案：** `plan.md`（Q1–Q5 全认）  
**基线：** `c688b0a`（WP-199 tip）  
**本 tip：** `8905deff3c840544ecaab9bfd52773512b4f2ba7`

| # | done_when | 结果 |
|---|-----------|------|
| 1 | 默认模型视图仅 head +（自愿）logs + result；冗余分层清零（代码与测可证） | **通过**。`render_model_view` 硬清；无默认面 `diagnostics:` / `calls (` ledger 段（仅负向断言 + 内部 `diagnostics` 字段）。session-read → Head `· reads session store`。 |
| 2 | 跑不起来：行(+列)+错因；跑起来：成功数/失败数/耗时；成功静默、失败可见 | **通过**。Head 两形态；耗时 `BudgetClock::wall_elapsed` → `duration_ms`；已结算计数（caught `error:*` 计失败；unsettled 不计 + 旁注）。 |
| 3 | 结果按窗尽量保；失败不空抹 | **通过**。`keep_value`：失败除 Cancelled 外保 value/partial；窗 `RUN_CODE_VIEW_MAX_BYTES`。 |
| 4 | cargo 绿 + artifacts；push 实验分支；不合 main | **通过**。见下测数；artifacts 齐；push `feat/tool-contract-ptc`；**未合 main**。 |

## 测数（artifacts 日志；本收口未整仓重跑）

| 日志 | 结果 |
|---|---|
| `cargo-run-code-view.txt` | `tools::code::` **18 passed**（EXIT_lib:0）+ `run_code_model_view` **17 passed**（EXIT_mv:0）= **35** |
| `cargo-sdk-tools.txt` | `tool::sdk_snapshot` **2 passed**；fixture sha `58cd2f33a5111ed0bcf6b6ed662f4338f8582fbd11314e091ce18a3650a49a6e` |

## 负向残留（收口复核）

`rg 'diagnostics:|calls \(' src/tools/code/render.rs tests/run_code_model_view.rs`：仅负向 `assert!(!…contains…)` 与内部结构字段 `diagnostics: Vec`；**无**默认渲染路径输出。

## Agent 视角（教法）

1. 短头：成败数 + ms；未启动则行/列+错因；unsettled / session-store 旁注同头。  
2. 过程日志：仅脚本自写；无则整段省略。  
3. 返回值：按窗；失败不空抹。LSP / 工具 diagnostics 不进默认面——需可见自行 log/return。

## Digs

- `dig-survey.md` / `dig-runtime-impl.md` / `dig-tools-teaching.md`（终态）  
- `draft-run_code-view.md`（已折进 live）

## 非目标 / 未入库

- **不合 main**；不改 `web/`  
- 未动 WP-201 实现；未 stage ops board / events / log / 其它 WP artifacts
