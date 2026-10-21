# WP-202 方案 — LSP 拆多工具 + 行列对齐并进 Code

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** HEAD `d97561b`  
**依据：** brief；Andy 2026-10-22 定稿硬清；Code-only  
**内存：** available ≥ 2GiB；中层串行放行 cargo（优先本包，WP-203 串后）

---

## 1. 目标契约（唯一活跃面）

七个**独立** Code 工具（object bag；无 Direct；无万能 `lsp`+action）：

```ts
// 定位类（需位置）：file_path + 1-based line + 1-based character；禁止 text
tools.goToDefinition({ file_path, line, character })
  -> { locations: Array<{ path, line, character, end_line?, end_character? }>; message: string }
  // 或压平字符串 locations 列表 + message——实现选压平优先（跟现 format.rs），形状在 dig 钉死

tools.findReferences({ file_path, line, character })
  -> 同上 locations + message

tools.hover({ file_path, line, character })
  -> { markdown: string; message: string }   // 或 content 文本；禁生 LSP JSON

tools.goToImplementation({ file_path, line, character })
  -> locations + message

// 文件级
tools.diagnostics({ file_path })
  -> { items: Array<{ severity, path, line, character, message }>; message: string }
  // 压平优先

tools.documentSymbol({ file_path })
  -> { symbols: Array<{ name, kind, line, character, end_line?, end_character? }>; message: string }

// 工作区检索：必带 query；可选 path 范围（底座已有则暴露，无则不造方言）
tools.workspaceSymbol({ query, path? })
  -> symbols + message
```

- **行/列：** 入参出参一律 **1-based** `line` + `character`（业界对齐；**砍**行内 `text` 定位）。
- 输出继续走 hub/`format.rs` 压平（locations / symbols / hover markdown）；**禁止**把生 LSP JSON 丢给模型。
- 入参 object bag；失败：可 catch 的 `ToolError` 或 `{…, message}`——跟现 Code 工具习惯，Runtime dig 钉死一种，不双轨。

**工具名默认：** 采用 brief 所列 camelCase（`goToDefinition` 等），与 hub `normalize_lsp_action` 对齐；若与 Code 面 `read`/`grep` 命名风格冲突，见 Q1（可改为 snake，语义不变）。

---

## 2. 硬清（活跃面一行不留）

| 砍 | 含 |
|---|---|
| 单工具 `lsp` + `action` bag | 注册、schema、教法、SDK、agent 绑定；优先**直接删除**活跃面（不留默认暴露的转发壳） |
| `text` 定位 | 入参、教法、「从行内子串解析列」路径 |
| 未列入操作 | callHierarchy / rename / Serena name_path；schema 枚举与教法禁止暗示 |
| Direct 轨 | 无 Direct 版；`ToolSurface::Code` only |
| 方言 | 旧 action 别名若仅服务单 `lsp` 则随壳删；hub 内 normalize 可留内部 |

---

## 3. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | 拆七工具实现；挂 hub（含已有 documentSymbol / workspaceSymbol / goToImplementation）；砍 text→character；删/退单 `lsp`；Code-only 注册；测改写；dig-runtime-impl |
| **Tools** | 七份 description + SDK fixture/PRELUDE 绑定；扫残留 `lsp`/`text`/action 方言；dig-tools-teaching |
| **mid** | 串槽（本包优先于 WP-203）；交叉审；acceptance/diff-summary；commit/push；deliver |

顺序：Runtime 硬清 → Tools 对齐 → mid 收口。

**非目标：** callHierarchy、rename、Serena、合 main、前端、libs。

---

## 4. 验收

1. 七工具契约落地；旧 `text` / 单 `lsp` action 方言活跃面清零（`rg` 可证）。  
2. Code-only；教法对齐 1-based line+character。  
3. 相关 cargo 绿；`artifacts/WP-202`；push 实验分支；未合 main。

---

## 5. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 工具名 snake_case（`go_to_definition` 等） | **GM 定稿采用** |
| Q2 | 定位只用 `file_path`+1-based `line`+`character`；砍 `text` | 是 |
| Q3 | 杀活跃面单 `lsp`（优先删除，不默认暴露转发壳） | 是 |
| Q4 | Code-only；无 Direct | 是 |
| Q5 | callHierarchy/rename 本包不做；不合 main | 是 |

