# WP-150 方案（前端-web；父包 WP-147 / WP-145；只改 web/，不 commit，未批不 start）

口径（Andy 锁定）：bind 是 code_search 对 agent 可见的唯一开关。设置页开关只显示 `tools.code_search.enabled` 存值，不按引擎状态置灰、不改写存值。不新增字段/接口。`session_search` 不动。
依据：`WP-145-frontend.md` 全文；`WP-145-backend.md` D 节（第 26 行，与依据一致）、B 节（只有主 agent 默认绑定，子代理不默认绑定）；后端 `src/tool/availability.rs:86` 的 `available_tools` 无条件列出 code_search（kind=Engine, origin=Workspace）。

## 现状核实（只读，2026-10-14）
- web 产品代码无 `code_search` 字面引用；AgentsSection / settingsStore / api/settings 不引用 engineStore。开关 `disabled` 只跟 `saveBlocked`。
- 网格（:631-651）由目录驱动；后端目录总含 code_search → 主/子代理网格都会出卡。**前端不兜底、不加占位**，展示按后端卡片。目录未加载（null）不出卡（维持现状）。
- `bindingFor`（:59-64）缺行 → Off。
- PUT 不过滤：`agentPersistPayload` `tools: {...draft.tools}` 全量；`putAgent` / `saveAgent` 只套 `withSyncedToolSeries`；`syncToolEnableSeries` 不删键，code_search 不在 fallback series。
- 新建模板 `startCreate` `tools: {}`，不写 code_search。

## 结论：本包为纯测试锁定，产品代码不改

## 测试（AgentsSection.test.tsx + api/settings 测试；目录 fixture 的 code_search 用 `kind:"engine"`、`origin:"workspace"`）
1. 目录中的 code_search 卡在引擎 **stopped / warming**（engineStore 置态）时仍显示、未置灰；存值 On 显示 On，可点 Off。
2. 缺行 → Off（主 agent 网格 + 子代理网格各一条；子代理网格由目录驱动显示 code_search）。
3. PUT 全量：`putAgent` body 含完整 tools（code_search 原值 + 目录外 MCP 键）；autosave/saveAgent 在引擎 stopped/warming 下 payload 保留 `tools.code_search`；`syncToolEnableSeries` 不删 code_search。
4. 新建 agent：`createAgent` payload 的 tools 不含 `code_search` 键（不显式写 false）。
5. session_search：现有用例全过，不改断言。
6. web 全量 vitest（/exec-daemon/node v22）+ `tsc -b` 0；lint N/A。

## 不在范围
后端、Engines 页、session_search、desktop、commit。

## 批复记录（中层）
Q1 不存在（无占位，展示按后端卡片）；Q2 目录未加载不出卡；Q3 不额外防御；Q4 子代理网格同样显示 code_search，缺行 Off（已加测）；Q5 engineStore 设状态再断言算数。

## 交付结果（2026-10-14）
- 产品代码改动：无。只新增测试文件 `web/src/dockview/panels/settings/AgentsSection.codeSearch.test.tsx`（11 条），没有删测试。
- 用例：
  1–2. 引擎 stopped / warming：存值 On 显示 On、不置灰、可点 Off，autosave 写入 false。
  3–4. 引擎 stopped / warming：改别的工具后保存，`tools.code_search` 原值 On 和目录外的 `mcp_other_ws` 都保留。
  5. 主 agent 缺行 → Off。
  6. 子代理网格显示目录里的 code_search，缺行 → Off。
  7. 子代理存值 On → On。
  8. 目录未加载（null）→ 不出卡。
  9. 新建 agent 的 payload 不带 `code_search` 键。
  10. `putAgent` 的 PUT body 带完整 tools（code_search + 目录外键），可用目录只有 read 时也一样。
  11. `syncToolEnableSeries` 不删、不补 code_search。
- session_search：现有用例全过，没改断言。
- web 全量 vitest（/exec-daemon/node v22.14.0）：176 个文件 / 1359 条全部通过；`tsc -b` 退出 0；lint N/A。
- 未 commit。
- 交叉审补充（2026-10-14）：子代理缺行用例加了 aria-pressed="false" 断言；新增 2 条（引擎 stopped / warming 时 session_search 卡片显示、开关 On/Off 与存值一致、不置灰）。文件现为 13 条，单文件 vitest 全部通过，tsc -b 0；未重新 deliver，未 commit。
