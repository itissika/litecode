# WP-200 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `c688b0a`（WP-199 tip）  
**本包：** run_code 默认模型面硬清为 head +（自愿）logs + result。

## 产品改动（Runtime）

| 路径 | 变更 |
|---|---|
| `src/tools/code/render.rs` | 硬清 failure/diagnostics/ledger/error_details 默认面；Head/Logs/Result；负向单测 |
| `src/tools/code.rs` | `keep_value`、组装 `duration_ms`、outcome 对齐已结算计数 |
| `src/ptc/mod.rs` | `Outcome.duration_ms` |
| `src/ptc/engine.rs` | `wall_elapsed` 写入 Outcome |
| `src/ptc/budget.rs` | 墙钟耗时支持（微触） |

## 产品改动（Tools / 教法）

| 路径 | 变更 |
|---|---|
| `src/tool/code_manifest.rs` | PRELUDE：去 outer text；自然语言三块；LSP 自 log/return |
| `src/tools/descriptions/write.md` | 去 outer text；LSP 不在 resolve |
| `src/tool/coords.rs` | `FILE_LINE_PREFIX_HINT`：outer ledger → transcript or read output |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | 已刷；sha `58cd2f33a5111ed0bcf6b6ed662f4338f8582fbd11314e091ce18a3650a49a6e` |

## 测试改写

| 路径 | 变更 |
|---|---|
| `tests/run_code_model_view.rs` | Head/Logs/Result + 负向无 `calls (` / `diagnostics:` |
| `tests/code_surface_phase{2b,3,4}.rs` | 去 ledger 行断言；对齐 Head 计数 / 旁注 |

## 测数（不重跑；以 artifacts 为准）

- `cargo-run-code-view.txt`：lib `tools::code::` **18** + `run_code_model_view` **17** = **35 passed**
- `cargo-sdk-tools.txt`：`tool::sdk_snapshot` **2 passed**
- digs：`dig-runtime-impl.md`、`dig-tools-teaching.md`、`dig-survey.md`

## 未做 / 不入库

- 不合 main；不改 `web/`
- 未入库脏 ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`
- 未入库 WP-193/195/196/201 artifacts；本 tip 仅 WP-200
- WP-201（人看 data）另包，待本 tip 后再 start
