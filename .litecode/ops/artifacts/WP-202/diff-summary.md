# WP-202 diff-summary

**分支：** `feat/tool-contract-ptc`  
**相对起点：** `d97561b`（WP-201 tip）  
**本包：** LSP 拆七 Code 工具 + 1-based 行列；砍 text / 单 lsp action；教法与 SDK fixture 对齐。

## 产品改动

| 路径 | 变更 |
|---|---|
| `src/tools/lsp.rs` | 单 `lsp`+action 拆为七 snake_case Code 工具；入参 `file_path`+1-based line/character；`workspace_symbol` 仅 `{query}`；出参压平 locations/hover/diagnostics/symbols；失败 ToolError |
| `src/lsp/hub.rs` | 挂接七工具所需 hub API / 行列约定 |
| `src/lsp/format.rs` | 出参压平与上限（locations/items 100、symbols 200） |
| `src/tools/edit/engine.rs` | 引擎 id `lsp` 保留；与拆分后工具面协调 |
| `src/tool/registry.rs` | 七工具注册 / 活跃面无单名 `"lsp"` |
| `src/tool/coords.rs` | 1-based line + 1-based UTF-16 character |
| `src/tool/code_manifest.rs` | PRELUDE H9：导航用七名，禁单 lsp/action/text |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | SDK fixture 刷七名教法；sha `60686e6…` |
| `src/permission/presets.rs` | 七工具权限预设 |
| `src/tools/descriptions/{七名}.md` | 新增七份 live description |
| `src/tools/descriptions/lsp.md` | **删除** |
| `tests/optional_lsp.rs` | 对齐七工具契约与 1-based 坐标 |

## 未改

- **`web/`**（本包禁止）  
- Settings 卡 stamp UI（id=`lsp` vs 工具名）— 产品另收口  
- callHierarchy / rename  
- ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`

## 测数（不重跑；以 artifacts 为准）

- `cargo-lsp-tools.txt`：`tools::lsp::` **13** + registry/availability/presets **30** + `optional_lsp` **24**（1 ignored）+ `optional_p0` **8**  
- `cargo-sdk-tools.txt`：`tool::sdk_snapshot` **2 passed**  
- dig：`dig-runtime-impl.md` / `dig-tools-teaching.md`

## Artifacts

- `acceptance.md` / `diff-summary.md`  
- `brief.md` / `plan.md` / `dig-survey.md` / `dig-runtime-impl.md` / `dig-tools-teaching.md`  
- `draft-*.md`（七份） / `cargo-lsp-tools.txt` / `cargo-sdk-tools.txt`

## 未做 / 不入库

- 不合 main；不改 `web/`  
- 未入库脏 ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`  
- 未入库 WP-193/195/196/203 无关 artifacts
