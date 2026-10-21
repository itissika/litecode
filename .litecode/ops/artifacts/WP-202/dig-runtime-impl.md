# WP-202 Runtime 实现笔记 — LSP 拆七工具 + 行列对齐进 Code

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `d97561b`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`（Q1 GM snake_case）、`dig-survey.md`；mid 定稿（H5 `file_path`、H8 不暴露 workspace path、一卡注入七名）  
> **硬约束遵守：** cargo 见下；**未 commit/push**；**未合 main**；**未改** `web/`；Tools 教法/SDK fixture **未刷**（仅最小 description 可编译）

---

## 1. 关闭/替换层（一句话）

Settings 卡 id 仍为 `lsp`（engine Ready + bind）；**注入七个** `ToolSurface::Code` snake_case 工具。Direct 活跃面 **无** 工具名 `"lsp"`，无 `action`/`text` 方言。定位改为 `file_path` + 1-based `line` + 1-based UTF-16 `character`。

---

## 2. 七工具契约（钉死）

| 工具名 | 入参 | 出参 `data` |
|---|---|---|
| `go_to_definition` | `{ file_path, line, character }` | `{ locations: Location[], message }` |
| `find_references` | 同上 | 同上 |
| `go_to_implementation` | 同上 | 同上 |
| `hover` | 同上 | `{ markdown, message }` |
| `diagnostics` | `{ file_path }` | `{ items: DiagItem[], message }` |
| `document_symbol` | `{ file_path }` | `{ symbols: Symbol[], message }` |
| `workspace_symbol` | `{ query }` **仅此**（**不**暴露 `path`/`file_path`；hub 无范围过滤） | `{ symbols: Symbol[], message }`（每条**必** `path`） |

### Location

```json
{ "path": "…", "line": 1, "character": 1, "end_line?": 1, "end_character?": 1 }
```

- `line` / `character`：**1-based** UTF-16（与 LSP Position+1 一致）。
- `end_*`：有则带；无则省略。

### DiagItem

```json
{ "severity": "Error"|"Warning"|"Information"|"Hint"|"Unknown", "path": "…", "line": 1, "character": 1, "message": "…" }
```

- `severity` / symbol `kind`：**字符串名**（非数字枚举）。
- `path`：查询的 `file_path`（diagnostics 每条都带）。

### Symbol

```json
{ "name": "…", "kind": "Function"|…, "line": 1, "character": 1, "end_line?": …, "end_character?": …, "path?": "…" }
```

- `document_symbol`：`path` **可省略**。
- `workspace_symbol`：无 `path` 的行 **丢弃**（保证每条有 path）。

### 空结果 / 错误

- **空结果**：resolve 成功；对应数组 `[]` + `message`（如 `No locations found…` / `No diagnostics.` / `No symbols found.` / hover 空 `markdown` + message）。**无** `ok:false`。
- **非法入参 / character 越界 / 未暖 / 越界路径 / LS 失败**：`ToolError`（`coded_error`：`invalid_input` / `unavailable` / …）。**无** `ok:false` data。

### `character` 校验

- 1-based UTF-16；允许行尾 caret（`1..=utf16_len+1`）。
- 越界 / 行号越界 → `invalid_input` 硬错（不 clamp）。

### 条数上限（钉死）

| 集合 | 上限 | 触顶 message |
|---|---|---|
| `locations` | **100** | `Returning first 100 locations (more omitted).` |
| `items`（diagnostics） | **100** | 同上 diagnostics 文案 |
| `symbols` | **200** | 同上 symbols 文案 |

成功态 **content 空**（`ok_data`）；ContentModified → Warning 文案保留。

### 绑定

- `optional_builtin_ids` 仍含卡 id `"lsp"`；readiness 键仍 `"lsp"`。
- `instantiate_tool("lsp")` → `all_code_tools`（七 Arc）。
- 引擎 id `"lsp"` **保留**（Warm/probe）；只砍工具方言。

---

## 3. 删除清单（硬清）

| 砍 | 说明 |
|---|---|
| Direct 工具名 `"lsp"` | 七工具均为 Code；`name()` 永不为 `lsp` |
| `action` bag / OPERATIONS / legacy definition\|references 入参 | 随旧壳删；hub `normalize_lsp_action` **内部保留** |
| `text` 定位 / `resolve_text_on_line` / multi-hit 扇出 / ## 视图 | 全删 |
| schema/教法 text 路径 | 七份 description 无 text；schema `additionalProperties: false` |
| workspace_symbol 可选 `path` | **不暴露**（H8；hub 无 filter） |
| callHierarchy / rename | 不挂 agent |

---

## 4. 改动文件（Runtime）

**重写：** `src/tools/lsp.rs`  
**增强：** `src/lsp/format.rs`（LocationItem/SymbolItem/DiagnosticItem + extract_*）、`src/lsp/hub.rs`（`raw` + `workspace_symbol_outcome`）  
**注册：** `src/tool/registry.rs`（卡→七工具）  
**教法最小：** `src/tools/descriptions/{go_to_definition,find_references,hover,diagnostics,document_symbol,workspace_symbol,go_to_implementation}.md`；删 `lsp.md`  
**测：** `tests/optional_lsp.rs` agent UX；`src/permission/presets.rs` Fixed 列表；`src/tool/coords.rs` 注释  
**本文件 +** `cargo-lsp-tools.txt`

**未改：** `web/`；`run_code_sdk.*`（Tools）；engines 卡语义。

---

## 5. cargo（已跑；tee `cargo-lsp-tools.txt`）

```bash
LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2
cargo test --lib -- tools::lsp:: …          # EXIT_A:0 → 13 passed
cargo test --lib -- tool::registry:: tool::availability:: permission::presets::
                                              # EXIT_B:0 → 30 passed
cargo test --test optional_lsp --test optional_p0 -- --test-threads=2
                                              # EXIT_C:0 → 24+8 passed (1 ignored RA smoke)
```

顺带：edit 引擎把 `lsp_error` diagnostic 真正折进 `EditResult.message`（WP-198 意图；此前只读空 content）；optional_lsp edit 测改 flat `{path,old_string,new_string}`；registry fixture 补 `ApplyEditsTool`。

---

## 6. Tools follow-up

- 七份 description 可加深教法；SDK fixture / PRELUDE 绑定白名单需刷出七名、无 `tools.lsp`。
- Settings 卡 stamp：卡 id 仍 `lsp`，表面 stamp 可能仍 Direct（候选 `name()`≠`lsp`）——产品若要卡显示 Code，Tools/mid 另收口。

