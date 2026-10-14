# WP-147（WP-145 前端）：设置页 code_search bind 只显示库里的期望

依据：`.litecode/ops/artifacts/WP-145-backend.md` D 节（契约 v1，等 GM 批）+ 后端中层口述补充（PUT 整份覆盖、缺行兜底 Off）。Andy 已批 WP-145；本包 approver=gm。不 commit；未批不 start。

## Goal（只做一项）
Settings → Agents：`code_search` bind 开关只反映 `GET/PUT /api/settings/agents/{id}` 里存的 `tools.code_search.enabled`，不按引擎 Warming / stopped / usable 置灰、改写或隐藏。`session_search` 不动。

## 契约引用（D + 补充）
- 不新增字段/接口。开关读写现有 agents GET/PUT 的 `tools.code_search.enabled`。
- 只显示库里存的值；不按引擎状态置灰/改写。
- 迁移后主 agent 有行且多为 true；缺行仍按 Off 显示（兜底）。
- PUT 是整份覆盖：提交时把 `tools` 完整带上，不要按可用性过滤，否则会被持久化成解绑。
- 引擎 off / Warming 只出现在调用结果首行（WP-146 C），不进设置页开关。

## 现状核实（web/src，只读）

### 无直接引擎耦合（保持）
- `AgentsSection.tsx` / `settingsStore.ts` / `api/settings.ts` **不**引用 `engineStore`、`usable`、`warming`。
- 开关 `disabled` 只跟 `saveBlocked`（约 :451），不跟引擎态。
- Engines 页 play/stop 只写 `engines.json` desired（`EngineView.tsx`），不写 agent bind。

### 间接耦合：目录决定是否出卡
| 位置 | 现状 | 要改什么 |
|------|------|----------|
| `AgentsSection.tsx:631-644` `bindableToolsPrimary` | 网格只渲染 `availableTools` 里的项；`kind==="engine"` 仅排序 | 依赖 WP-146 后 `GET /available-tools` **始终**含 `code_search`（去掉 readiness/desired 门禁）。前端不得再按引擎态过滤。若落地后仍缺项，需在本包补「profile 已有 `tools.code_search` 时也出卡」——优先以后端目录为准，改前先跟后端中层对齐。 |
| `AgentsSection.tsx:646-651` `bindableToolsSubagent` | `filter(isSubagentBindableTool)`；`code_search` 本身可绑子代理 | **不改** session_search / 子代理规则；确认不因本包误伤。 |
| `AgentsSection.tsx:59-64` `bindingFor` | 缺行 → `{ enabled: false, … }`（Off 兜底） | **保持**；测缺行 Off。 |
| `AgentsSection.tsx:424-456` 卡片 On/Off | `enabled = binding.enabled`；点击只改 draft | **保持**只读库值；禁止接入 `engineStatuses` / usable。 |
| `AgentsSection.tsx:547-585` `agentPersistPayload` | 注释：目录外绑定保留；`tools: { ...draft.tools }` | **保持**全量 tools；加测：引擎 stopped/warming 时 PUT 仍带 `code_search` 原值。 |
| `AgentsSection.tsx:672-678` persist serialize | 走 `agentPersistPayload` → `saveAgent` | 同上，禁止按 available 删键。 |
| `api/settings.ts:397-410` `putAgent` | `JSON.stringify(withSyncedToolSeries(...))` 整份 PUT | **保持**；series 同步不得删掉 `code_search`。 |
| `api/settings.ts:736-753` `syncToolEnableSeries` | 只同步 series 成员开关，不删无关键 | **保持**；确认 `code_search` 无 series 误伤。 |
| `stores/settingsStore.ts:623+` `saveAgent` | 再跑一遍 `withSyncedToolSeries` 后 `putAgent` | **保持**全量；禁止按 `availableTools` 过滤 payload。 |
| `AgentsSection.test.tsx` | 有 lsp 目录闪烁 / 跨工作区保留用例；**无** code_search×引擎态用例 | **新增**见下。 |
| 结果视图 `CodeSearchToolView*` / `SyncPctHeader` | 只解析调用结果首行 | **本包不改**（WP-138 已接 v2）。 |
| `session_search` | core，始终在 available-tools；Agents 绑定逻辑与引擎无关 | **明确不动**（契约 A/D）。 |

### session_search 耦合检查
- Agents 页：无单独门禁；随 core 目录展示；`bindingFor` 同缺行 Off。
- 工具结果：`SessionSearchToolView` 独立，本包零改动。
- 后端 WP-146：session_search LLM 门禁规则「不改」——前端不跟。

## 改动清单（预计很小）
1. **主路径**：WP-146 Tools 让 `available-tools` 常驻 `code_search` 后，设置页已「只显示库值」——本包以 **补测试 + 防回归** 为主；若仍有按引擎隐藏/置灰代码则删掉（当前 rg 未发现）。
2. **护栏**：PUT / auto-save 路径断言 `tools.code_search` 不因 available 闪烁或引擎态丢失（比照现有 lsp 闪烁测试）。
3. **产物**：本文件；子包产物可另写 `WP-145-web-bind.md`（实现证据）。
4. **不改**：Engines 页 desired；`session_search`；LLM 工具表/调用结果（后端）；不 commit。

## 测试（web vitest + tsc -b）
1. 库里 `code_search.enabled=true`，模拟引擎 stopped / warming（只动 engineStore 或根本不挂引擎数据）→ 卡片仍 On，可点 Off；不出现置灰/unavailable。
2. profile 无 `code_search` 行 → 卡片 Off（兜底）。
3. 保存/PUT：`saveAgent`/`putAgent` payload 含完整 `tools`，且 `code_search` 与 draft 一致，不因 availableTools 省略或引擎态而消失。
4. `session_search`：既有绑定行为用例仍过；本包不改其断言语义。
5. 全量相关 AgentsSection / settings API 测试绿；`tsc -b` 0。

## 计划
- **一个** web 子包挂在 WP-147 下（前端-web），批后、且 WP-146 契约/目录行为定稿后再 start。
- frontend_mid：填 plan → submit → gm 批 → 派子包 → 验收 → deliver WP-147。
- 与 WP-146（Tools/Runtime）同批落地意识：前端测可 mock available-tools 已含 code_search；真机联调等后端迁移后主 agent 有行。

## 不在范围
- 后端 availability / seed 迁移（WP-146/148/149）
- Engines 页 usable 标签与 progress 条
- sync_pct 调用结果文案（C 表）
- 新建 agent：前端模板不得显式写 code_search=false（缺键时后端对 primary agent 自动补 enabled=true）；测试覆盖新建 payload 不含 code_search=false。（后端中层 D 节补充，2026-10-14）
