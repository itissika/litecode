# WP-202 Tools 教法 — dig（收口完成）

> **作者：** Tools  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支：** `feat/tool-contract-ptc`（不合 main；**未 commit**）  
> **阶段：** **收口完成** — draft 叠 live 七份；`lsp.md` 已删（Runtime）；PRELUDE H9；刷 `run_code_sdk`；sdk_snapshot 绿。  
> **依据：** `brief.md`、`plan.md`、`dig-runtime-impl.md`；mid H1–H11（H8：workspace_symbol **仅** `{query}`）。

## 0. 总评

| 项 | 状态 |
|---|---|
| 单轨 | **只教** 七个独立 Code snake_case；禁 `tools.lsp` / `action` / 行内 `text` |
| 工具名 | `go_to_definition` / `find_references` / `hover` / `diagnostics` / `document_symbol` / `workspace_symbol` / `go_to_implementation` |
| 定位 | `file_path` + **1-based** `line` + **1-based UTF-16** `character`；`workspace_symbol` 仅 `{ query }`（H8 无 path） |
| 上限（写死） | locations/items **100**；symbols **200**；触顶进 `message` |
| 空/错 | 空 → resolve+[]/`""`+message；非法/未暖/越界 → **ToolError**；无 `ok:false` |
| live / SDK | **已叠**；sdk_snapshot **2 passed**；sha 见下 |
| web/ | **未改**（卡 stamp 产品另收口） |

## 1. 交付路径

| 路径 | 说明 |
|---|---|
| `src/tools/descriptions/{go_to_definition,find_references,hover,diagnostics,document_symbol,workspace_symbol,go_to_implementation}.md` | live 七份（含上限数字） |
| `src/tools/descriptions/lsp.md` | **已删**（Runtime 硬清；本包确认缺席） |
| `src/tools/lsp.rs` | include_str 七份（Runtime；本包未改实现） |
| `src/tool/code_manifest.rs` PRELUDE | **H9** 导航短句（七名；勿 lsp/action/text） |
| `src/tool/fixtures/run_code_sdk.txt` | 渲染自当前 HEAD（PRELUDE 含七名） |
| `src/tool/fixtures/run_code_sdk.sha256` | `60686e6ce9c9721480a6b6fb10502beaa89dd5b0934cd52a600c8b49da40e540` |
| `.litecode/ops/artifacts/WP-202/dig-tools-teaching.md` | 本笔记（终态） |
| `.litecode/ops/artifacts/WP-202/cargo-sdk-tools.txt` | UPDATE + VERIFY tee |
| `draft-*.md` | 草稿保留对照；live 已覆盖 |

## 2. 契约摘要（与 Runtime dig / H1–H11 对齐）

```ts
tools.go_to_definition({ file_path, line, character }) -> { locations, message }  // ≤100
tools.find_references({ file_path, line, character }) -> { locations, message }    // ≤100
tools.go_to_implementation({ file_path, line, character }) -> { locations, message } // ≤100
tools.hover({ file_path, line, character }) -> { markdown, message }               // H1
tools.diagnostics({ file_path }) -> { items, message }                             // ≤100；severity 字符串
tools.document_symbol({ file_path }) -> { symbols, message }                       // ≤200；可无 path
tools.workspace_symbol({ query }) -> { symbols, message }                          // ≤200；每条必 path；无范围 path
```

- **H8：** `workspace_symbol` 教法**不**暴露可选 `path`/`file_path`。  
- **H9：** PRELUDE 已加导航短句。  
- **builtin_code_surface：** 七工具仍为 **lsp 卡注入**（optional），**未**并入 `builtin_code_surface_tools()`——默认 SDK 工具表仍为 read/glob/grep/edit/apply_edits/write/run/wait/kill；七名出现在 PRELUDE 文案。若强行入 always-on SDK，会撞 `sdk_snapshot` 的 `!file_path` 断言（H5 定位字段名）。

## 3. cargo

```bash
MemAvailable ≥2 GiB；CARGO_BUILD_JOBS=2

UPDATE_SDK_FIXTURE=1 cargo test --lib tool::sdk_snapshot::phase0_run_code_sdk_snapshot -- --exact
→ UPDATED sha=60686e6c…；EXIT 0

cargo test --lib tool::sdk_snapshot -- --test-threads=2
→ 2 passed（phase0 + sdk_hash_is_stable…）；EXIT 0
```

tee → `artifacts/WP-202/cargo-sdk-tools.txt`。UPDATE hook **已从** `sdk_snapshot.rs` **撤回**。

## 4. 残留扫描

- `descriptions/lsp.md`：不存在  
- agent 教法面无 `tools.lsp` 调用教法（仅 PRELUDE「never a single `lsp` tool」禁令）  
- **未改** `web/`；卡 id 仍 `lsp` 注入七 Code（Runtime）

## 5. 约束 / 未做

- **未** commit / push / 合 main  
- **未**改 Runtime 执行 / hub / schema（仅加深 description + PRELUDE）  
- **未**把七工具并入 `builtin_code_surface_tools`（见 §2）  
- Settings 卡 stamp 表面仍可能显示 Direct 语义——产品另收口（Runtime dig §6）

## 6. Blockers

无。可交 mid 收口 commit/deliver。
