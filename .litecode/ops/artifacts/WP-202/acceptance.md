# WP-202 acceptance

**口径：** Andy + GM；LSP 活跃面拆七个独立 Code 工具（snake_case）；定位 `file_path` + 1-based `line` + 1-based UTF-16 `character`；砍 `text`；`workspace_symbol` 仅 `{query}`；失败 ToolError（无 ok:false）；空 resolve+[]+message。  
**分支：** `feat/tool-contract-ptc`（实验；**不合 main**）  
**方案：** `plan.md`（Q1 GM：**snake_case** 七名）  
**基线：** `d97561b`（WP-201 tip-backfill）  
**本 tip：** `TIP_PLACEHOLDER`

| # | done_when | 结果 |
|---|-----------|------|
| 1 | 上列多工具契约落地；旧 `text` / 单 `lsp` action 方言活跃面清零 | **通过**。七工具名 snake_case：`go_to_definition` / `find_references` / `hover` / `diagnostics` / `document_symbol` / `workspace_symbol` / `go_to_implementation`。活跃面无工具名 `"lsp"`+action；schema 无 `text`；`workspace_symbol` 仅 `{query}`。卡 id 仍 `lsp` → 注入七 Code；引擎 id `lsp` 保留。 |
| 2 | Code-only；教法对齐行列 | **通过**。`ToolSurface::Code`；live 七份 description + PRELUDE H9（禁单 lsp/action/text）；定位 1-based line + 1-based UTF-16 character。 |
| 3 | cargo 绿；artifacts/WP-202；push 实验分支；未合 main | **通过**。测数下（本收口**未** cargo 重跑）；artifacts 齐；push 实验分支；**未合 main**；**未改 `web/`**。 |

## Q1 定稿

工具名 **snake_case**（非 camelCase）。Settings 卡 stamp（卡 id=`lsp` vs 七工具名）产品侧另收口，非本包 blocker。

## 测数（artifacts 日志；本收口未整仓重跑）

| 日志 | 结果 |
|---|---|
| `cargo-lsp-tools.txt` | `tools::lsp::` **13 passed**（EXIT 0）+ registry/availability/presets **30 passed**（EXIT 0）+ `optional_lsp` **24 passed**（1 ignored RA smoke）+ `optional_p0` **8 passed**（EXIT 0） |
| `cargo-sdk-tools.txt` | `tool::sdk_snapshot` **2 passed**（EXIT 0）；fixture sha `60686e6ce9c9721480a6b6fb10502beaa89dd5b0934cd52a600c8b49da40e540` |

## 出参形状（钉死）

- locations：`{path,line,character,end_*?}` + message；上限 100  
- hover：`{markdown,message}`  
- diagnostics：`{items[{severity,path,line,character,message}],message}`；上限 100  
- symbols：`{symbols[{name,kind,line,character,end_*?,path?}],message}`；上限 200；workspace 每条必 path；document 可无  
- 空：resolve+[]+message；非法/未暖/越界 → ToolError（无 ok:false）

## Digs

- `dig-runtime-impl.md`（Runtime 终态）  
- `dig-tools-teaching.md`（Tools 教法 / SDK）  
- `dig-survey.md` / `brief.md` / `plan.md` / draft-* 七份

## 已知推迟（非本包 blocker）

- Settings 卡 stamp（id=`lsp` vs 七工具名）产品另收口。  
- 七工具仍走 lsp 卡注入，**未**并入 `builtin_code_surface_tools`（默认 SDK 工具表无七名 schema，只在 PRELUDE）。

## 非目标 / 未入库

- **不合 main**；**不改 `web/`**  
- callHierarchy / rename / Serena name_path  
- 未 stage ops `board.json` / `events/` / `log.jsonl` / `tool_traces` / `board.md`  
- 未入库 WP-193/195/196/203 无关 artifacts
