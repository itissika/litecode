# WP-202 摸底 dig — LSP 拆多工具 + 行列对齐进 Code

> **作者：** backend_mid（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 02:37 Asia/Hong_Kong  
> **分支 / HEAD：** `feat/tool-contract-ptc` @ `d97561b`  
> **依据：** `brief.md`（Andy 定稿）  
> **硬约束：** **只读**；**未改代码**；**未 cargo**；**未 commit/push**；不合 main

---

## 0. Brief 目标（对照）

| # | 目标 | 现状摘要 |
|---|---|---|
| 1 | 操作集：goToDefinition / findReferences / hover / diagnostics / documentSymbol / workspaceSymbol(+query) / goToImplementation | hub/`format.rs` **已有**；agent 面 `LspTool` **只暴露前 4 个**（schema enum），且主动 Avoid `workspaceSymbol` |
| 2 | 多工具拆分；杀旧单 `lsp` + action bag | 现状唯一工具名 `"lsp"` + `action` 枚举 |
| 3 | `file_path` + 1-based `line` + 1-based `character`；砍 `text`；workspaceSymbol 用 `query` | 现状：`line`+`text` 行内子串定位 → UTF-16 column；**无 character 入参**；query 未接到 agent 面 |
| 4 | hub/format 挂到 agent 面；输出压平 | format 已压平 locations/symbols/hover/diagnostics |
| 5 | `ToolSurface::Code`；无 Direct | 现状 `LspTool` **默认 Direct**（未 override `surface()`） |

非目标（本包不做）：callHierarchy / rename / Serena name_path / 合 main / 前端。

---

## 1. 现 `tools.lsp`（Direct）入参 / 出参 / action / text 定位

### 1.1 身份与面

| 项 | 值 |
|---|---|
| `Tool::name` | `"lsp"` |
| `Tool::surface` | **默认 `Direct`**（`src/tools/lsp.rs` 未实现 `surface()`） |
| 注册 | `registry::instantiate_tool`：`tool_id == "lsp"` → `LspTool::new(engines, workspace_root)` |
| 目录卡 | `optional_builtin_ids()` 含 `"lsp"`；仅当 workspace engine readiness=`Ready` 才进 `available_tools`（与 `code_search` 的 bind-only 不同） |
| 默认绑定 | `default_bound_optional_builtins()` = `["code_search"]` — **lsp 不默认绑** |
| 教法 | `src/tools/descriptions/lsp.md`：四操作 +「diagnostics 只要 file_path；其余要 file_path+line+text」 |

### 1.2 Schema（模型可见）

```json
{
  "type": "object",
  "properties": {
    "action": {
      "type": "string",
      "enum": ["goToDefinition", "findReferences", "hover", "diagnostics"]
    },
    "file_path": { "type": "string" },
    "line": { "type": "integer" },
    "text": { "type": "string" }
  },
  "required": ["action", "file_path"]
}
```

- **无** `character`、**无** `query`。
- schema 文案明确 Avoid inventing `workspaceSymbol`。
- `OPERATIONS` 常量另含 legacy 别名 `"definition"` / `"references"`（validate 接受；schema enum 不列）。

### 1.3 校验 `needs_position`

对 `definition|goToDefinition|references|findReferences|hover`：

- 必填 `line`（integer，`>= 1`）
- 必填非空 `text`（trim 后）

`diagnostics`：只要 `action` + `file_path`。

单测 `validate_rejects_removed_actions_and_missing_text`：**拒绝** `workspaceSymbol`；缺 `text` 的 definition 失败。

### 1.4 `text` 定位怎么用

`resolve_text_on_line(file_path, line, text)`：

1. 读文件、按行取 `line`（1-based）。
2. 在该行找 **所有** `text` 子串命中。
3. 每个命中算 **1-based UTF-16 列**（`encode_utf16().count() + 1`）→ 交给 hub 的 `character`。
4. 0 命中 → 硬错（提示从 read 拷贝子串）。
5. ≥2 命中 → 对每个 column 各打一次 hub，拼 `format_multi_hit_view`（goToDefinition 会合并相同 landing）。

**没有**模型直接传 column/character 的路径；column 完全由 `text` 推导。

### 1.5 执行路径 → hub

```
validate → resolve_in_workspace(file_path)
        → per_server_failure / ensure_ready (Warm wait ≤30s)
        → [position actions] resolve_text_on_line
        → hub.tool_action_outcome(action, path, line?, column?, query=None)
        → format 已在 hub 内完成；nav 再 enrich_nav_result（贴 landing snippet）
```

出参：`ToolCallResult` **content 文本**（压平后的 locations / hover markdown / diagnostics 行）；无稳定 `data` schema（Direct 不要求）。ContentModified → warning，不重试。

### 1.6 路径参数名

LSP 工具用 **`file_path`**；Code 面主流（read/write/grep/glob/edit）用 **`path`**（apply_edits 项内亦接受 `path` 或 `file_path` 兼容）。

---

## 2. hub / format 已有能力（可挂）

### 2.1 `LspHub::tool_action_async`（`src/lsp/hub.rs`）

入参：`action, file_path, line: Option, character: Option, query: Option`  
内部：`line/character` **1-based → 0-based** 再进 LSP `Position`（UTF-16 语义与协议一致）。

| action（normalize 后） | LSP 方法 | 位置? | query? | format 分支 |
|---|---|---|---|---|
| `goToDefinition`（别名 definition） | `textDocument/definition`，空则试 `typeDefinition` | 要 | — | `format_locations` |
| `findReferences`（别名 references） | `textDocument/references` + includeDeclaration | 要 | — | `format_locations` |
| `hover` | `textDocument/hover` | 要 | — | `format_hover` |
| `goToImplementation`（别名 implementation） | `textDocument/implementation` | 要 | — | `format_locations` |
| `documentSymbol` | `textDocument/documentSymbol` | 否（uri） | — | `format_symbols` |
| `workspaceSymbol` | `workspace/symbol` | 否；`file_path` 只用来 **选/启 LS** | `query`（缺省 `""`） | `format_symbols` |
| `diagnostics` | wait publish | 否 | — | `format_diagnostics` |
| prepareCallHierarchy / incomingCalls / outgoingCalls | callHierarchy* | 要 | — | **本包不做**（format 有，agent 不挂） |

**路径范围：** hub 对 `workspaceSymbol` **没有**额外 path filter；标准 `workspace/symbol` 也只有 query。brief「可选 path 范围若底座已有」→ **底座没有**，本包勿假塞。

### 2.2 `src/lsp/format.rs` 压平形状（已可用）

- locations：`path:Lline` 行列表（空 → 固定「No locations found…」句）
- hover：markdown/plaintext contents
- diagnostics：最多 100 条 `Severity: message (Lline)`
- symbols：缩进 `Kind name (Lline)`（含 children）
- call hierarchy：有格式但 WP-202 不暴露

Agent 侧 `enrich_nav_result` 仅对 definition/references 再贴代码窗（implementation 目前不 enrich — 拆工具时可对齐或保持）。

### 2.3 旁路（勿与多工具混淆）

- `lsp_feedback.rs`：write/edit 成功后挂 Error diagnostics（`Diagnostic`），**不是** agent `lsp` 工具。
- PRELUDE 已写「LSP diagnostics 不在 tool data 里」——指 edit/write 旁路，与新 Code LSP 工具 data 设计要分清。

---

## 3. Direct vs Code 注册面（谁暴露单 `lsp`）

```
available_tools ──(optional builtin "lsp" if Ready)──► instantiate LspTool
                                                         surface=Direct (default)
project_tools ── surface==Code → code_tools → run_code 白名单
              └─ else → direct（模型 function-call）
```

| 面 | 现状 | WP-202 目标 |
|---|---|---|
| **Direct**（模型顶层 function call） | **唯一暴露点**：工具名 `lsp` + action bag | **清零**：无 Direct `lsp`，无 Direct 多工具替身 |
| **Code**（`tools.<name>` inside `run_code`） | **未进**：LspTool 非 Code，不在 whitelist；SDK fixture 无 lsp 绑定 | 七工具均 `ToolSurface::Code` + `data_schema`，进 bundle |
| Settings / bind 卡 | 一张 Engine 卡 `id=lsp`（workspace Ready 才列出） | 需拍板：卡与 7 工具如何映射（见 Q2） |
| Executor 门禁 | Direct 调 Code 名 / Code 调 Direct 名 → surface mismatch | 迁 Code 后 Direct 再叫 `lsp` 应 missing/mismatch |

**Companion 先例：** `wait`/`kill` 随 `run` 注入；`apply_edits` 随 `edit` 注入——**不是**独立 settings 卡，且均为 Code。

---

## 4. 建议工具名（Code camelCase vs snake）

### 4.1 现有 Code 命名对照

| 工具 | `name()` | JS 调用 |
|---|---|---|
| read / write / grep / glob / edit | 全小写单词 | `tools.read` |
| apply_edits / wait / kill | **snake_case** | `tools.apply_edits` |
| （无） | camelCase 先例 | — |

Brief 操作名是 **LSP/VS Code 方言 camelCase**：`goToDefinition` 等。

### 4.2 两套候选

| 方案 | 名示例 | 利 | 弊 |
|---|---|---|---|
| **A. camelCase（贴 brief / LSP）** | `goToDefinition`, `findReferences`, `documentSymbol`, `workspaceSymbol`, `goToImplementation`, `hover`, `diagnostics` | 与 brief/hub action 一字不差；`tools.goToDefinition` 是合法 JS 标识符；`pascal_case`→`GoToDefinition` 类型前缀干净 | 与现有 Code 工具 snake 风格不一致 |
| **B. snake_case（贴 Code 同伴）** | `go_to_definition`, `find_references`, `document_symbol`, … | 与 `apply_edits` 一族一致 | 与 brief/hub 字符串不一致，需 normalize 表；教法两套词 |

### 4.3 mid 建议（待 Q1 拍板）

**倾向 A（camelCase）**：本包 brief 已用该词；hub/`format`/`normalize_lsp_action` 已认；Code PRELUDE 允许 `tools.name` 任意标识符。snake 只为「全家统一」收益小、迁移噪音大。

路径字段：建议新工具入参统一 **`path`**（对齐 read/grep），内部再 resolve；`file_path` 作可选别名可减迁移痛（apply_edits 已有先例）——见 Q3。

---

## 5. 硬清清单（符号 / 教法 / SDK / 绑定）

### 5.1 代码 / 符号

| 项 | 动作 |
|---|---|
| `src/tools/lsp.rs` 单体 `LspTool` + `action`/`text`/`OPERATIONS`/multi-hit text 扇出 | **删除或拆空**：活跃面不得再暴露单 `lsp` |
| 新建 7× Code Tool（可同模块 factory） | 各 `name` + `surface=Code` + 完整 input/`data_schema` + description |
| `resolve_text_on_line` / multi-hit view / schema `text` | **砍**（定位改 `line`+`character`） |
| legacy 别名 `definition`/`references` 入参 | **随旧工具死**；hub 内 `normalize_lsp_action` 可留（编辑器/内部） |
| `registry::instantiate_tool("lsp")` | 改为注入 7 Code 工具，或废除 id、改 companion 注入 |
| `optional_builtin_ids` / readiness / bind | 按 Q2 改：要么一卡扩七，要么七 id（不推荐） |
| `data_schema` | Code 准入硬要求；须定义 locations/symbols/hover/diagnostics 的 **data** 形状（勿只扔生 LSP JSON；可与 content 压平并行或 data=结构化、content=短文） |
| 单测 | 删/改 `validate_rejects_removed_actions…`；新：character 定位、workspaceSymbol 必 query、Direct 无 `lsp`、Code whitelist 含七名 |
| `contract_matrix` / `phase*_gate` | 扫旧「lsp + text / 无 data」类断言（WP-201 已留「旧 data 断言另排」同类债） |
| `coords.rs` 注释「`lsp` `line`」 | 改称新工具名 / `line`+`character` |
| callHierarchy / rename | **不动** hub 实现即可，**不**加 Tool |

### 5.2 教法 / SDK

| 项 | 动作 |
|---|---|
| `descriptions/lsp.md` | 删；新增七份 description（或一模块多 `include_str`） |
| `code_manifest` PRELUDE | 视需要补一句「LSP 导航经 `tools.goToDefinition`…」；现有「LSP diagnostics 不在 data」指 write 旁路，勿误导 |
| `run_code_sdk.{txt,sha256}` | Tools 刷：whitelist 出现新名；无 `tools.lsp` |
| knowledge/contracts（若有 lsp 文） | 对齐或标废弃 |

### 5.3 绑定 / 产品面

| 项 | 动作 |
|---|---|
| Agent DB 已绑 `lsp: enabled` | 迁移策略：读旧绑 → 新七工具或 companion 开关（Q2） |
| Settings 卡 `CallSurface` | 迁 Code 后卡应显示 Code（`available_tools_payload` 已按 `Tool::surface` 映射） |
| 引擎 `lsp` desired/servers | **保留**（Warm/Failed/probe）；只砍 **工具方言**，不砍引擎 |

### 5.4 明确不碰

- `web/` 前端  
- hub 内 callHierarchy / rename  
- 合 main  

---

## 6. mid 拍板 Q（≤5）

**Q1 — 工具名风格**  
A) camelCase 贴 brief：`goToDefinition` / `findReferences` / …  
B) snake_case 贴 Code：`go_to_definition` / …  
**建议：A。**

**Q2 — 目录 / 绑定**  
A) 保留一张可选 builtin 卡 `lsp`（engine Ready）；bound 时 **注入七个 Code 工具**（类 edit→apply_edits）  
B) 七张独立 optional id（绑定爆炸，不推荐）  
C) 取消独立卡：引擎 Ready 且某核心条件即注入（无 bind）  
**建议：A**（最小产品面变化，硬清 Direct 名）。

**Q3 — 路径字段名**  
A) 新工具一律 `path`（Code 对齐）；可选接受 `file_path` 别名  
B) 继续只叫 `file_path`（贴旧 lsp / brief 字面）  
**建议：A。**

**Q4 — `character` 语义**  
确认：**1-based UTF-16 code units**（与现 text→column、与 LSP Position 一致），教法写明「与编辑器列一致，非 Unicode 标量」。是否要在非法/越界时硬错 vs clamp？  
**建议：硬错（与 line 越界一致），不 clamp。**

**Q5 — Code `data` vs content**  
A) `data` = 结构化（locations: `{path,line,character?}[]` / symbols / hover text / diagnostics）；content 短摘要或同构文本  
B) `data` 仅 `{ text: string }` 包压平串（准入最小，FE/程序弱）  
**建议：A**（进 Code 就是为了程序可读；压平规则继续走 format，避免生 LSP JSON）。

---

## 7. 风险 / 依赖（非扩 scope）

- 七工具均需 `data_schema` 才过 `validate_code_contract`；设计 data 形状是实现首步。  
- 旧 agent 绑 `lsp` 的迁移若漏，会出现「卡在、工具没了」。  
- `goToImplementation` / `documentSymbol` / `workspaceSymbol` 首次上 agent 面，需最小单测（可 mock hub）。  
- WP-203（webfetch/websearch 进 Code）串行，避免 cargo 互抢；本摸底不依赖 203。

---

## 8. 结论（给方案用）

底座 **已齐**：hub 七动作 + format 压平 + 1-based→0-based character。缺口全在 **agent 契约**：单体 Direct `lsp`+`text` 方言、缺 Code surface/data_schema、缺三操作暴露、缺 `query`/`character` 入参。WP-202 = **硬清旧方言 + 七 Code 工具挂现成 hub**，不是重做 LSP。
