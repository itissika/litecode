# WP-204 dig-survey — agent 可见提示词四层盘点

**基线：** `feat/tool-contract-ptc` @ `62ccead`（WP-203 tip）  
**性质：** 只读；未改产品码 / 未刷 fixture / 未 cargo / 未 commit  
**对照：** WP-196…203 acceptance（同 artifacts 目录）

---

## 0. 拼装调用链（系统面如何进模型）

```
build_system_prompt(agent_id, agent_config, ctx?)   # src/context_pipeline/system.rs
  ├─ resolve_body → builtin_prompt_for / 用户覆盖
  ├─ (role≠hidden) splice_harness:
  │     HARNESS_SYSTEM  + editable + HARNESS_TOOLS + CITATION_PROMPT
  └─ splice AGENTS.md then CLAUDE.md（有则）
```

`run_code` 工具的 `description` = 当次投影的完整 SDK 文本  
（`CodeTool::description` → `render_code_tool_description`：固定 `PRELUDE` + `LIBS_SDK` + 按名排序的绑定工具 declare）。

Settings 卡（`AvailableTool`）**不**把卡 title/description 注入模型回合；卡只决定是否 instantiate。  
`lsp` 卡 id → `lsp::all_code_tools` 注入七个 Code 工具进投影，再进 `run_code` SDK。

---

## 1. 系统固定

| 路径 | 角色 | 要点 |
|---|---|---|
| `src/context_pipeline/system.rs` | 拼装入口 | harness 包 editable；hidden 仅 editable |
| `src/config/global_db/builtin_prompts.rs` | `include_str!` 入口 | HARNESS_* / CITATION / 各 agent pack |
| `src/config/global_db/prompts/harness/system.md` | 跨工具不变量 | 权限否认不重试、system-reminder、注入警惕、压缩 |
| `src/config/global_db/prompts/harness/tools.md` | 跨工具工作流 | 有专用工具就用；可并行；**仅点名** `run_code` 绑定语义 |
| `src/config/global_db/prompts/citations.md` | 引用格式 | `[@ …]`；与工具参数无关 |
| `prompts/{default,orchestrator,general,explore}/{identity,work,voice}.md` | 可编辑身份/工作/语气 | **无**单工具用法配方 |
| `prompts/compaction.md` | hidden 压缩 | 固定结构；禁工具 |

**关键发现：** 固定层基本干净。唯一具名工具是 `run_code`（运输层，且带「when in tool list」）。未渗 LSP / webfetch / edit 等单工具契约。

---

## 2. Code / PRELUDE / SDK

| 路径 | 角色 |
|---|---|
| `src/tool/code_manifest.rs` `PRELUDE`（约 L16–31） | 固定教法头，**每次** `render_code_tool_description` 无条件 `push_str` |
| 同文件 `LIBS_SDK` | 纯 JS libs；无 IO |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | SDK 快照；sha=`b89efddee23d3d398aee0607ccefe5c98ca2a50b10229e074319ca8f571e3c33`（与 fixture 字节一致；PRELUDE 头 15 行与 fixture 头匹配） |

**PRELUDE 已对齐 WP-197…203 的正向契约：**

- edit / apply_edits → `{ok,…}` 软失败  
- webfetch / websearch → `{ok,…}` 软 vs ToolError 硬  
- LSP 七 snake 名；禁单 `lsp` / `action` / row-inline `text`  
- read/grep/glob：无 cursor / `page.next` / `reason`；续读 `start_line`  
- run_code 模型面：head + 自愿 logs + result；无 ledger / 默认 diagnostics 列表  

**关键发现（域隔离）：**

1. LSP 句带闸：*「when the LSP engine is ready and these bindings appear below」* — 未绑时仍点名七工具，但有条件。  
2. **webfetch / websearch / edit 句无「when appear below」闸** — 即使用户在 Settings 拔掉绑定，PRELUDE 仍教 `tools.webfetch` / `tools.edit`；声明表可能无该名。  
3. ToolError `code` 枚举仍列 `stale_cursor`（WP-199 已从 grep/glob/read 教法清 cursor；实现 `file_identity` 仍可产该 code）— 残留方言名，非教分页。

---

## 3. live descriptions（`src/tools/descriptions/*.md`）

**31 份；无 `lsp.md`（WP-202 已删）。**

| 组 | 文件 |
|---|---|
| Code shell | `run.md` `wait.md` `kill.md` |
| Code edit/IO | `edit.md` `apply_edits.md` `write.md` `read.md` `grep.md` `glob.md` |
| Code network | `webfetch.md` `websearch.md` |
| Code LSP×7 | `go_to_definition` `find_references` `hover` `diagnostics` `document_symbol` `workspace_symbol` `go_to_implementation` |
| Direct / 引擎 | `ask_user` `plan` `todo` `knowledge` `code_search` `session_search` `litecode_workspace` + subagent×6 |

**与 WP-196…203 活跃面对照（rg）：**

| 已硬清项 | descriptions / PRELUDE / fixture |
|---|---|
| Direct bash / wait_shell / kill_shell | **无**独立 description；无推荐调用 |
| `run_command` / `command_*` | **无** |
| camelCase LSP 工具名 / 单 `lsp`+action 可调 | **无**（负向禁令有） |
| LSP `text` 定位 | 负向禁；出参 hit.`text`（行内容）保留 — 不同义 |
| webfetch 必须 try/catch | **无**；教 branch on `ok` |
| 拒 198.18 | **无**；教放行 `198.18/15` |

**过时方言命中：**

- `session_search.md`：*「bash cannot reach it」* — WP-196/197 后默认面无 Direct bash；应改中性措辞（如 shell / `tools.run` 不可达）。

**刻意负向教法（反退化，非实现残留）：**  
多数 Code description 含 *「There is no X」*（replace_all、cursor、action、Direct twin…）。与 PRELUDE 负向句叠加 → **冗余**，服务防退化。

**跨工具互提（同域工作流，可接受）：**  
`webfetch`↔`websearch`；`run`/`wait`/`kill`；`edit` 例调 `run`；LSP 例调 `grep`。  
`write.md` 一句「LSP errors … not in resolved data」— 轻量跨域负向，非强推 LSP。

---

## 4. 卡注入 / 绑定文案

| 触点 | 进模型？ | 说明 |
|---|---|---|
| `availability::tool_card` / Settings catalog | **否**（无 prose 注入回合） | 只列 id/kind/surface |
| `registry`：`tool_id == "lsp"` → 七 Code | **间接** | 绑定且 Ready 时七 description 进 SDK declare |
| `wait`/`kill` companion of `run`；`apply_edits` of `edit` | **间接** | 无独立 Settings 卡；随主工具出现 |
| `builtin_code_surface_tools` | 准入/Settings | 含 webfetch/websearch；**不含** LSP 七名（仍靠卡注入；WP-202 已知推迟） |
| `code_search` / `session_search` description | 绑定时 | 自述 Warm/隐藏 — 与可拔插一致 |
| 引擎失败串（`LspShared::availability_failure`） | 进 **ToolError message**（调用时） | Settings 指向；非系统固定层 |

**关键发现：** 卡→工具投影机制健康；问题在 **PRELUDE 固定头对可拔插工具的无条件点名**（见 §2），而非卡文泄漏。

---

## 5. 对照基线摘要（WP-196…203）

| WP | 硬清 | 提示词面现状 |
|---|---|---|
| 196 | Direct bash 默认关 | description 无 bash 工具；**session_search 仍写 bash** |
| 197 | `run`/`wait`/`kill` | live 对齐 |
| 198 | edit/apply_edits `{ok}` | live + PRELUDE 对齐 |
| 199 | grep/glob/read；禁 cursor | live 对齐；PRELUDE 仍列 `stale_cursor` code |
| 200/201 | run_code 模型面三块；data 不进 content | PRELUDE 教 head/logs/result；未教 by_tool（正确） |
| 202 | 七 snake LSP | live + PRELUDE H9；无 lsp.md |
| 203 | Code webfetch/websearch；软硬；198.18 | live + PRELUDE 对齐 |

---

## 6. 本 dig 未改

未改 `src/`、`web/`、fixture、未 cargo、未 commit、未合 main。产物仅本文件与 `review.md`。
