# WP-204 review — agent 可见提示词文本面

**分支：** `feat/tool-contract-ptc` @ `62ccead`  
**依据：** `brief.md` / `plan.md`；dig：`dig-survey.md`；对照 WP-196…203 acceptance  
**性质：** 只读分析；**本包不改码**

---

## 一句话总评

WP-196…203 硬清后的**契约正向面**（snake LSP、`run`/`wait`/`kill`、edit/web 软失败、198.18、run_code 三块视图）在 live description + PRELUDE/SDK 上**基本对齐、无活跃旧方言可调入口**；主要缺口是 **PRELUDE 对可拔插/可解绑工具的无条件点名**（域隔离）、以及 **`session_search` 仍写 `bash`** 等少数过时措辞——建议另开实现包收口，本包无需动码。

---

## 五轴（严重度大 → 小）

### 1) 域隔离 / 可拔插 — **P1（建议实现包）**

**结论：** Settings 卡本身不向模型注入卡文；绑定投影正确（未绑则 declare 表无该工具；`lsp` 卡→七 Code）。但 **`run_code` 的固定 PRELUDE 无条件点名** `tools.edit` / `tools.webfetch` / `tools.websearch`，且 **无**「when these bindings appear below」闸（LSP 七名有闸）。用户拔掉 network/edit 绑定时，模型仍被教调用可能不存在的 `tools.*`。系统固定 harness **未**渗单工具用法（仅运输层 `run_code`），固定层本身稳；泄漏点在 PRELUDE。

**证据：**

- `src/tool/code_manifest.rs` PRELUDE L20–21：无条件 edit + webfetch/websearch 软失败教法  
- 同文件 L24：LSP 七名带 *「when … appear below」*  
- `render_code_tool_description` L216：`out.push_str(PRELUDE)` 不依赖当前 whitelist  
- `src/config/global_db/tools.rs`：`network_core_tools` / `core_configurable` 可解绑；`lsp` 为 optional，默认不绑  

**建议后续包：** PRELUDE 按当次 bundle **条件拼接**可选段（或一律加「仅当下方 declare 出现」闸）；LSP 卡 stamp（id=`lsp` vs 七工具名）仍按 WP-202 产品另收口，非本提示词 P0。

---

### 2) 自相矛盾 / 过时方言 — **P1（少量；建议实现包）**

**结论：** 系统 vs 工具、PRELUDE vs description **无实质性契约冲突**（软 `{ok:false}` vs 硬 `ToolError` 已分层写清；LSP 全硬错与 web 软失败并存是定稿差异）。过时方言几乎清零，**残留：**

| 项 | 严重度 | 证据 |
|---|---|---|
| `session_search.md`「bash cannot reach it」 | P1 | WP-196/197 后默认无 Direct bash；应改「shell / `tools.run` 不可达」等中性说法 |
| PRELUDE ToolError 枚举仍列 `stale_cursor` | P2 | WP-199 教法已禁 cursor；`file_identity` 仍可产该 code — 可保留实现 code，或教法改为不点名旧分页方言 |
| `litecode_workspace` 的 `action` 字段名 | P3 | 与已禁的 LSP `action` bag 同词不同义；低风险 |

**未发现：** camelCase LSP 工具名、可调单 `lsp`+action、`text` 定位教法、`run_command`、硬崩 must-try/catch webfetch、拒 198.18 的旧说法。

---

### 3) 精准 — **基本通过（P2 微调）**

**结论：** live 七 LSP / webfetch / websearch / run·wait·kill / edit·apply_edits / grep·glob·read 与 WP-197…203 acceptance **一致**（坐标 1-based + UTF-16 character；workspace_symbol 仅 `{query}`；body 50000；成功 message `Fetched.` / `Search completed.`；无 `results[]`）。PRELUDE 头与 `run_code_sdk.txt` 头匹配；fixture sha `b89efd…1e3c33`。

**轻微不准：** PRELUDE 在「未出现在 declare」时仍描述 webfetch/edit 行为（并入轴 1）。负向句密度高但不与现行 schema 打架。

---

### 4) 系统固定稳 — **通过**

**结论：** `harness/system.md` + agent identity/work/voice + citations **只谈跨工具不变量与工作流**；`harness/tools.md` 仅在运输层点名 `run_code`（带 in-list 条件）。未渗 LSP 用法、web SSRF、edit 匹配规则等。Compaction / Orchestrator / Explore 包干净。

**证据：** `prompts/harness/tools.md` 全文 4 条；rg 具名工具仅 `run_code`。

---

### 5) 冗余 — **P2（可接受 / 可瘦）**

**结论：** 大量 *「There is no X」* 负向教法在 description 与 PRELUDE 重复（防 WP-196…203 退化）——**有意冗余**，非实现双轨。部分 description 偏长（`read`/`apply_edits`/`webfetch` 300+ words）。不阻塞；若开实现包可顺手：PRELUDE 保留总禁令、单工具 description 删重复负向句。

---

## 无需动 / 建议开实现包

### 无需动（本轮提示词面）

- 系统固定 harness / citations / 各 agent editable pack  
- live 七 LSP、webfetch/websearch、run/wait/kill、edit/apply_edits、grep/glob/read 的**正向契约**教法  
- 删除 `lsp.md`、无 Direct bash description、SDK fixture 与 PRELUDE 头同步  
- Settings 卡不注入散文；LSP 卡→七工具投影机制  
- WP-200/201：模型面不教 `by_tool`（正确）

### 建议开实现包（不在 WP-204 改）

| 草案 | 内容 | 优先级 |
|---|---|---|
| **WP-205? PRELUDE 可拔插闸** | 按 bundle 条件拼接 edit/network/LSP 段，或统一「仅当下方 declare 出现」；测：拔绑后 SDK 文案无强推 | P1 |
| **WP-205? session_search 去 bash** | 改中性「进程/`tools.run` 不可达」；可选扫其他 Direct 文案 | P1 |
| **可选瘦身** | description 与 PRELUDE 负向句去重；评估 `stale_cursor` 是否仍应出现在 PRELUDE 枚举列表 | P2 |
| **产品另排** | Settings 卡 stamp（`lsp` vs 七名）；`builtin_code_surface_tools` 是否纳入 LSP 七名（WP-202 已知推迟） | 非提示词 blocker |
| **测对齐（Q4）** | contract_matrix 旧断言若仅测侧 — 另排；提示词面无则本包不扩 | 另排 |

---

## 附录 A — 路径清单

| 层 | 路径 |
|---|---|
| 拼装 | `src/context_pipeline/system.rs` |
| 固定/可编辑 | `src/config/global_db/builtin_prompts.rs`；`prompts/harness/{system,tools}.md`；`prompts/citations.md`；`prompts/{default,orchestrator,general,explore}/*`；`prompts/compaction.md` |
| PRELUDE/SDK | `src/tool/code_manifest.rs`；`src/tool/fixtures/run_code_sdk.{txt,sha256}` |
| descriptions | `src/tools/descriptions/*.md`（31；无 `lsp.md`） |
| 卡/投影 | `src/tool/availability.rs`；`src/tool/registry.rs`（`LSP_CARD_ID`→七工具；`builtin_code_surface_tools`）；`src/tools/lsp.rs` |
| 对照 | `.litecode/ops/artifacts/WP-196…203/acceptance.md` |

## 附录 B — 关键摘录

**Harness（稳）：**

```text
# Using your tools
- When this request's tool list already has a dedicated tool for the step, use it.
- …
- When `run_code` is in your tool list, the tools it binds are available only inside
  that program as `tools.<name>(...)`. … do not call them directly.
```

**PRELUDE 域隔离差（无闸）vs LSP（有闸）：**

```text
`tools.edit` resolves EditResult{ok,path,message}. …
`tools.webfetch` and `tools.websearch` likewise resolve `{ok,…,message}` …
For code navigation (when the LSP engine is ready and these bindings appear below)
use go_to_definition, … — never a single `lsp` tool, `action` bag, or row-inline `text` locator.
```

**过时方言：**

```text
# session_search.md
… join that directory, the id shown in the result, and `.md`. bash cannot reach it. …
```

**正向对齐例（webfetch）：**

```text
Soft failure … no try/catch required …
Fake-IP / TUN ranges such as `198.18.0.0/15` are **allowed** …
```

## 附录 C — 建议后续包草案（给 GM / Andy）

1. **提示词可拔插一致性（P1）** — PRELUDE 条件化；修 `session_search` bash；可选负向句瘦身。不合 main 约束随实验分支。  
2. **Settings stamp / catalog（产品）** — 卡 id `lsp` 与七工具名展示；是否 always-on schema。  
3. **不重做** WP-193 架构总评；本包已覆盖 agent 可见文本面查漏。

---

**验收自检：** 五轴有结论+证据；「无需动 / 建议实现包」明确；未改产品码；未合 main。
