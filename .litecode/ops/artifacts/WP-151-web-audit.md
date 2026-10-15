# WP-151 web 前端瘦身调研（WP-152，子包 A，只读）

- 范围：`/workspace/litecode/web`；基线 HEAD `3ddc643`；调研日期 2026-10-14。
- 硬规则执行情况：产品代码、package.json、package-lock.json、配置零改动。工具装在 `/tmp/wp152-tools`，脚本和中间产物放在 `/tmp/wp152`。后端透传的报错（变量形式的 `err.message`、`body.error`）不统计。没有 commit。
- 工具：knip（/tmp）、ts-prune（/tmp）、jscpd（/tmp）、rg。文案统计用 web/node_modules 里的 TypeScript 5.8.3 JS API，只读引用；/tmp 装到的 typescript 是 7.0.2，没有 JS API，不能用。Node 统一用 /exec-daemon/node v22.14.0。

## 开工前 git status（2026-10-14 22:46）
```
 M .litecode/ops/board.json
 M .litecode/ops/events/inbox.json
 M .litecode/ops/events/outbox.jsonl
 M .litecode/ops/log.jsonl
 M .litecode/ops/tool_traces.jsonl
 M .litecode/ops/tools/board.md
?? .litecode/ops/artifacts/WP-147-fill.txt
?? .litecode/ops/artifacts/WP-151-fill.txt
```
（web/ 下无改动。）

---
## 1 死代码

人工确认标准：
- **确认可删**：在 web 全目录用 rg 都找不到引用（包括字符串、动态 import、`import.meta.glob`、vite/vitest 配置、index.html、CSS `@import`、desktop 对 web 的引用），测试里也没有。
- **疑似**：只被测试或文档引用、属于协议镜像类型、桶文件（barrel）导出，或工具结论之间不一致。

证据：
- `/tmp/wp152/knip.json`：6 个未用文件、145 个未用导出、101 个未用类型、3 个未用 devDependency、2 个未声明依赖。
- `/tmp/wp152/tsprune.txt`：293 行，其中 209 行是「只在模块内使用」。
- `/tmp/wp152/tsprune-check.txt`：逐个符号的 rg 计数。prod 指非测试文件里的引用数，test 指测试文件里的引用数，self 指定义文件内出现的行数。

### 1.1 确认可删（24 条）
| # | 路径 / 符号 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| 1 | `src/api/json.ts`（`JsonValue` 类型，7 行） | knip 未用文件；rg `api/json` 无引用 | 删文件 | 低 |
| 2 | `src/theme/dockview.ts`（转发 `./dockview/adapter`，5 行） | knip 未用文件；唯一消费方 `AppShellDockview.tsx:11` 直接 import adapter | 删文件 | 低 |
| 3 | `src/theme/dockview.css`（133 行） | knip 未用文件；`theme/index.css:3` 引入的是 `./dockview/adapter.css`。diff 显示它是 adapter.css 的旧副本：少了 `--dv-tab-max-width`，色板是写死的十六进制色值，而不是 `var(--_dk-cat-*)` | 删文件 | 低；jscpd 统计的 132 行重复随之消失 |
| 4 | `src/utils/turnPhase.ts`（`formatTurnPhase` / `turnPhaseTone`，40 行） | knip 未用文件；rg 无 import | 删文件 | 低 |
| 5 | `src/components/knowledge/KnowledgeRelationStrip.tsx`（23 行） | knip 未用文件；rg 无引用 | 删文件 | 低；若是知识库规划中的组件，删前请中层确认 |
| 6 | `api/adapter.ts:274 isEmptyItemShell` | prod=0 test=0 self=1 | 删函数 | 低 |
| 7 | `api/adapter.ts:311 extractToolFilePath` | 同上 | 删函数 | 低 |
| 8 | `components/Logo.tsx:18 skipSplash` | 同上 | 删导出 | 低 |
| 9 | `lib/citationResolve.ts:28 peekCitation` | 同上 | 删函数 | 低 |
| 10 | `lib/litecodeLsp.ts:1641 refreshDiagnostics` | 同上 | 删函数 | 低到中；LSP 模块较大，删后要跑 tsc 和测试 |
| 11 | `lib/theme.ts:52 toggleTheme` | 同上 | 删函数 | 低 |
| 12 | `stores/settingsDocuments.ts:168 isRevisionedDocument` | 同上 | 删函数 | 低 |
| 13 | `stores/settingsStore.ts:698 mcpServersFromStore` | 同上 | 删函数 | 低 |
| 14 | `stores/turnStore.ts:105 headPermission` | 同上 | 删函数 | 低 |
| 15 | `components/knowledge/KnowledgeMarkdown.tsx:111 KnowledgeInlineBody` | 同上 | 删组件 | 低 |
| 16 | `components/knowledge/KnowledgeMarkdown.tsx:167 KnowledgeStatusBadge` | 同上 | 删组件 | 低 |
| 17 | `components/transcript/transcriptUserRailMarks.ts:134 newestUserSeq` | 同上 | 删函数 | 低 |
| 18 | `dockview/workbench/model.ts:54 groupHasComponent` | 同上 | 删函数 | 低 |
| 19 | `lib/knowledge/markers.ts:321 extractSymbolRefs` | 同上 | 删函数 | 低 |
| 20 | `components/PermissionModal.tsx:10` 中转导出 `PermissionGrantOpts` | 真正的定义在 `AskCard.tsx`，没有任何文件从 PermissionModal 引入它 | 删这一行中转导出 | 低 |
| 21 | `dockview/menu/menuItems.ts:27 menuItems` 常量（注释写着「for tests」） | 没有测试使用；`MenuBar.tsx` 用的是 `buildMenuItems` | 删常量 | 低 |
| 22 | devDependency `remotion` | knip 未用；整个仓库 rg 只在 package.json 和锁文件里出现，也没有对应的 npm script | 删依赖（会改 package.json 和锁文件，放到后续包） | 低；待 Andy 定（可能是做宣传视频预留的） |
| 23 | devDependency `@remotion/cli` | 同上 | 同上 | 同上 |
| 24 | devDependency `@types/opentype.js` | 整个仓库没有 opentype 的引用 | 删依赖 | 低 |

### 1.2 疑似，需复核（14 条）
| # | 路径 / 符号 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| 1–7 | `api/types.ts`：`SessionAttached`(446)、`CompactStarted`(661)、`JsonRpcRequest`(772)、`JsonRpcResponse`(779)、`JsonRpcNotification`(786)、`LspNotification`(803)、`TransportRequest`(814) | prod=0 test=0；它们是协议镜像类型 | 和后端协议对一下：已废弃的事件就删，仍在协议里的保留作文档 | 中；删了以后，类型和协议对不上的问题更难发现 |
| 8 | `dockview/workbench/queries.ts:205 LivePanel`（类型） | prod=0 | 并入 #9 一起处理 | 低 |
| 9 | `dockview/workbench/index.ts` 桶文件（约 50 个转发导出） | 只有 `shell/BrowserHeaderAction.tsx:4` 用到其中的 `groupRole`，其余导出只是公开接口 | 二选一：精简成只转发真正用到的，或者统一改为直接 import 子模块 | 中；工作台公共 API 是有意设计的，待 Andy 定 |
| 10 | `dockview/panels/settings/providerBilling.ts:22 API_PROVIDER_IDS` | prod=0；按设计，未知 provider 一律归 API，所以这张列表实际不参与判断 | 删除，或只留在注释里 | 低；它是 WP-114 口径的文档化清单 |
| 11 | `theme/tokens.ts:89 readDkTokens` | 只在 `theme/README.md:72` 文档里提到 | 删函数并同步改 README，或者留作公开 API | 低 |
| 12 | `src/lib/knowledge/fixture.ts` | 只有 3 个测试 import 它，却放在产品目录 lib 下 | 移到 `src/test/fixtures` | 低；knip 把测试也当入口，所以没报 |
| 13 | `scripts/ws-smoke.mjs`（140 行，手动跑的 WS 冒烟脚本） | knip 未用文件；没有 npm script 引用 | 保留并在 README 里写明，或者删掉 | 低；待 Andy 定 |
| 14 | ts-prune 报的 209 个「只在模块内使用」的导出 | 原文见 `/tmp/wp152/tsprune.txt` | 后续包批量去掉 `export`，不删代码 | 低；部分可能留作测试接口，要逐个看 |

附：knip 还报了 2 个未声明的依赖，它们不是死代码，是声明不完整：
- `theme/dockview/adapter.ts` 直接 `import type … from "dockview"`，但 package.json 只声明了 `dockview-react`；
- `prependAnchor.test.ts` 引用了 `@tanstack/virtual-core`，同样没有声明。

建议后续包把它们补进 package.json。目前靠间接依赖能跑，风险低到中，升级时可能断。

---
## 2 可合并重复组件（10 组）

方法：jscpd 的门槛是 50 个 token，不扫测试；整体重复率是 1.84% 行、2.01% token，共 72 处克隆（原文在 `/tmp/wp152/jscpd/jscpd-report.json`）。在这个基础上再按命名族人工比对。

| # | 路径 | 差异 | 合并目标 | 影响面 | 风险 |
|---|---|---|---|---|---|
| 1 | `settings/CustomToolsSection.tsx` 171-329 ↔ `settings/McpServersSection.tsx` 276-436 | 共 93 行重复：global/workspace 作用域切换、定义列表、JSON 编辑区、底部操作区；MCP 多了进程 Start/Stop/Restart | 抽出 `ScopedDefinitionEditor`，由两节分别传入各自的操作按钮 | 2 个文件和各自的测试 | 中；两节的保存逻辑和测试选择器都要对齐 |
| 2 | `panels/AgentPanel.tsx` 99-168 ↔ `panels/SubagentReadOnlyPanel.tsx` 20-73 | 70 行：会话加载、会话已不存在（「This session no longer exists」）、错误兜底几乎一样 | 抽成 hook `useSessionPanelBootstrap(sessionId)` | 2 个面板，以及 AgentPanel / SubagentReadOnlyPanel 的测试 | 中 |
| 3 | `components/SubagentReadOnlyContent.tsx` 48-113 ↔ `panels/AgentPanel.tsx` 225-261、379-399 | 63 行：渲染 transcript 列表和状态行，只读版去掉了输入框 | 抽出共享的 `TranscriptBody`，用 `readOnly` 参数区分 | 同上 | 中；涉及虚拟列表滚动，要回归测试 |
| 4 | `tabs/AgentTab.tsx` 40-62、`tabs/EditorTab.tsx` 28-51、`tabs/KnowledgeGraphTab.tsx` 16-38、`tabs/BrowserTab.tsx` | 每个约 23 行：标签外壳、关闭按钮、中键关闭；区别只是图标和标题来源 | 抽出 `BaseTab` 外壳，图标和标题由参数传入 | 4 个标签组件和 EdgeTab 的测试 | 低到中 |
| 5 | `ui/Dropdown.tsx` 237-354 ↔ `ui/Popover.tsx` 118-217 | 浮层定位、Esc 关闭、点外部关闭都一样；Dropdown 多了菜单项的键盘导航 | 抽出 `useFloatingLayer` | 两者的全部调用方 | 中；焦点管理和 a11y |
| 6 | `components/ModelSwitcher.tsx` 44-61 ↔ `ui/Select.tsx` 26-43 | 18 行：触发按钮的结构和样式相同 | ModelSwitcher 复用 Select 的触发按钮 | ModelSwitcher 测试 | 低 |
| 7 | `toolviews/FileParamView.tsx` 14-42 ↔ `WriteToolView.tsx` 17-44 ↔ `EditToolView.tsx` 56-63 | 文件路径头（图标、路径、打开按钮）重复 | 抽出 `FilePathHeader` | 3 个工具视图和 ToolCallCard 测试 | 低 |
| 8 | `components/AskUserCard.tsx` 62-97 ↔ `components/ToastMarkdown.tsx` 18-48 | 33 行：精简版 markdown 渲染的组件映射一样 | 抽出 `compactMarkdownComponents` | 2 个文件 | 低 |
| 9 | `components/GitPanel.tsx` 495-560 ↔ `panels/SearchPanel.tsx` 39-45、458-486 | 35 行：可折叠分组的标题行（箭头、计数、标题） | 抽出 `CollapsibleSectionHeader` | 2 个面板 | 低 |
| 10 | `fileview/ImagePreview.tsx` 25-36 ↔ `MediaPreview.tsx` 32-43；`panels/KnowledgeGraphPanel.tsx` 14-28 ↔ `KnowledgePanel.tsx` 10-23 | 两对各 12–15 行的预览外壳、面板 host 样板代码 | 各合成一个共享外壳 | 4 个文件 | 低；收益也小，可排在最后 |

不计入组数：
- `theme/dockview.css` ↔ `dockview/adapter.css`：前者是死代码，见 §1.1 #3。
- `lib/shader/hlslGlobals.ts` 内部有 254 行重复：这是悬浮文档的数据表，不是组件。建议用生成脚本或模板来减少，单独评估。
- `api/workspace.ts` 内部有 59 行重复：几个请求函数的样板代码相同，可以抽一个请求小工具，属于低风险。

疑似跨端（只标注，由中层合成）：
- **tokens.css**：web 的 `src/theme/tokens.css` 被 desktop 的 `desktop/scripts/copy-theme-tokens.mjs:6` 和 `desktop/src/hub.ts:10-12` 直接读取。这是共享源，不能删也不能挪。
- **滚动条样式**：`desktop/src/hub.ts:106` 注释写明它对齐 `web/src/theme/index.css` 的滚动条样式，两边各维护一份，属于疑似重复。

---
## 3 多余提示文案（按面板，18 条）

分类：可删 = 界面已经表达过的信息或开发期说明；可合并 = 同一屏里同义的多处；可缩短 = 超过一句话，或超过约 80 个字符。aria-label 默认保留。

| # | 面板 | 位置 | 原文 | 标注 | 建议文 | 风险 |
|---|---|---|---|---|---|---|
| 1 | 设置 › Files | `FilesSection.tsx:130-135` | These lists live in this workspace (.litecode/excludes.json). New workspaces start with the built-in defaults. Settings edits and direct changes to that file apply to the tree and to search immediately. Code semantic search uses the suffix list below, not Search exclude. | 可缩短 | Saved in .litecode/excludes.json; changes apply immediately. Semantic search uses the suffix list below. | 低 |
| 2 | 设置 › Files | `FilesSection.tsx:188-189` | Additional hides for search and glob (on top of files exclude). Does not change the code semantic index. | 可缩短；可与 #1 合并 | Extra hides for search and glob. | 低 |
| 3 | 设置 › Files | `FilesSection.tsx:210-211` | Suffixes the code semantic index includes. One per line. A missing suffix is left out; an empty list indexes no source files. | 可缩短 | File suffixes to index, one per line. | 低 |
| 4 | 设置 › MCP | `McpServersSection.tsx:327-328` | Register stdio MCP servers. Same id: workspace overrides global. Bind mcp_<id> on Agents. | 可缩短 | Workspace overrides global. Bind mcp_<id> on Agents. | 低 |
| 5 | 设置 › MCP | `McpServersSection.tsx:249` | Keeps a stdio process running. Save JSON, then Start. Config changes need Restart. | 可缩短 | Save, then Start. Restart after config changes. | 低 |
| 6 | 设置 › Custom Tools | `CustomToolsSection.tsx:222` | Definitions only. Bind on Agents. Same name: workspace overrides global. | 可合并（和 #4 是同一套口径） | 两节共用一个提示组件：Bind on Agents. Workspace overrides global. | 低；依赖 §2 #1 |
| 7 | 设置 › Agents | `AgentsSection.tsx:236-239` | Replaces identity, work, and voice. A builtin: value uses the built-in text. System, tool, and citation sections are always added. | 可缩短 | Replaces the built-in persona (builtin: keeps default). | 低 |
| 8 | 设置 › Agents | `AgentsSection.tsx:266` | Default 50 runs out on long tasks — 200 is a saner default. | 可缩短；语气待定 | Long tasks may need ~200. | 低；待 Andy 定 |
| 9 | 设置 › Agents | `AgentsSection.tsx:341` | subagent_launch is enabled but no subagents are allowed — launches will fail until you select at least one. | 可缩短 | Select at least one subagent, or launches will fail. | 低 |
| 10 | 设置 › Agents | `AgentsSection.tsx:999` | Compaction only assigns a model. Prompt, tools, and max steps are built in. | 可缩短 | Only the model is configurable. | 低 |
| 11 | 设置 › Agents | `AgentsSection.tsx:460`（aria-label） | {id} tool binding, {state}. Click the card to toggle. | 可缩短 | 去掉「Click the card to toggle.」，只留 `{id} tool binding, {state}`（`aria-pressed` 已经表达了能切换） | 中；测试用正则匹配了前半段，要核对 |
| 12 | 设置 › Advanced | `AdvancedSection.tsx:70-72` | Uses Exa hosted search. Paste an Exa API key for higher limits; leave empty for the anonymous free tier, or set EXA_API_KEY in the environment. | 可缩短 | Optional Exa API key (or EXA_API_KEY) for higher limits. | 低 |
| 13 | 设置弹框 | `SettingsDialog.tsx:67` | Agent is busy - settings saves are disabled until the chat view is stable. | 可缩短 | Agent is busy — saving is paused. | 低 |
| 14 | 设置 › Models | `ModelsSection.tsx:232` 与 `ModelSwitcher.tsx:364,594` | No provider holds a key yet. Add one on the Providers page — its models show up here. ／ Configure a provider API key in Settings → Providers ／ No active models — configure a provider API key in Settings. | 可合并 | 统一成一句：No models yet — add a provider key in Settings › Provider. | 低 |
| 15 | 标题栏 / 状态栏 | `TitleBar.tsx:68` 与 `StatusBar.tsx:105` | Connected to a remote litecode serve (no local sidecar)（两处一模一样） | 可合并 | 抽成常量，或只保留一处 | 低 |
| 16 | 会话列表 | `SessionItem.tsx:128,134` | 同一句「Cannot delete while this session chain is busy」同时写在 aria-label 和 title 里 | 可合并 | 共用一个变量 | 低 |
| 17 | 工具视图 | `ToolContentView.tsx:59` | Resource is held by session {id}. It will be retried automatically once released — switch sessions or wait. | 可缩短 | Held by session {id}; will retry when released. | 低 |
| 18 | 聊天区 / 子代理面板 | `AgentChatInput.tsx:624-625`、`SubagentRosterPanel.tsx:299-311`、`sessionStore.ts:648,654`、`transcriptMarks.tsx:126`、`SessionStatusLine.tsx:970`、`knowledge/KnowledgeFlowCard.tsx:338-433`、`KnowledgeAttentionIcon.tsx:22` | 15 条中文文案（停止、删除、执行计划、没有在运行的回合、停止失败，请稍后重试 等）夹在英文界面里 | 统一语言 | 先统一成英文，或者直接进入多语言包 | 中；有测试按中文断言，例如 WP-089 的 roster 测试；待 Andy 定 |

---
## 4 功能介绍文档大纲（问号弹框，和设置弹框同款）

弹框结构：左侧分节导航，右侧每节一句要点。入口按界面英文原文写，后面附中文要点。只收有界面、有交互的功能。

**Getting started（上手）**
1. **Sessions** — 入口：左侧栏 › Sessions。新建、切换、删除会话；忙碌的会话不能删除。
2. **Agent chat** — 入口：中央 Agent 面板 › Message the agent。给 agent 发消息，可以附图；agent 执行中发出的消息会排队到下一步；可以停止当前回合。
3. **Model switcher** — 入口：输入框旁边的模型下拉。切换当前会话的模型和思考档位。

**Working with the agent（与 agent 协作）**
4. **Permissions & questions** — 入口：聊天区里弹出的 Approve / Submit 卡片。批准工具调用和计划，回答 agent 的提问。
5. **Plan** — 入口：状态行 › Open plan。查看 agent 的执行计划和进度。
6. **Subagents** — 入口：Agent 面板的子代理名单。查看子代理，可以停止、删除，或打开只读视图。
7. **Context usage** — 入口：输入框旁边的上下文环 › Compact context。查看上下文占用，手动压缩。
8. **Tool results** — 入口：聊天区的工具卡片。展开工具调用详情（Bash、Edit、Write、搜索等），从卡片里打开文件。

**Workspace（工作区）**
9. **Explorer** — 入口：左侧栏 › Explorer。浏览、过滤（支持 glob）、拖动和复制文件，右键菜单可以进行文件操作和打开终端。
10. **Search** — 入口：左侧栏 › Search。全文搜索和会话搜索，可开区分大小写（Aa）。
11. **Source Control** — 入口：左侧栏 › Source Control。暂存、放弃、提交（Ctrl+Enter）、Pull，查看提交图。
12. **Editor & previews** — 入口：在 Explorer 里打开文件。代码编辑，Markdown、图片、媒体、SQLite 预览。
13. **Terminal** — 入口：底部 › Terminal。多个终端，可以关闭。
14. **Browser** — 入口：标签栏 › New Browser。内置浏览器，支持前进、后退和输入地址。
15. **Knowledge / Knowledge Graph** — 入口：左侧栏 › Knowledge、面板 › Knowledge Graph。浏览和编辑知识条目，查看关系图。

**Layout（布局）**
16. **Panels & tabs** — 入口：标签右键菜单 › Maximize / Rename / Popout Window / Return to Main Window。最大化、重命名、弹出到独立窗口，再收回主窗口。
17. **Options menu** — 入口：标题栏 › Options。回到 Home、打开 Settings...、切换 Theme: Dark / Light、打开 About。

**Settings（设置，各子节一句话）**
18. **Provider / Models** — 入口：Settings › LLM。填 API key，启用模型。
19. **Agents** — 入口：Settings › Agent › Agents。配置 agent 的模型、提示词、最大步数、工具绑定和子代理。
20. **Custom Tools / MCP** — 入口：Settings › Agent。定义自定义工具，注册 MCP 服务；工具要在 Agents 里绑定才会生效。
21. **Engines** — 入口：Settings › System › Engines。启停 LSP 和代码检索引擎，下载嵌入模型。
22. **Files** — 入口：Settings › System › Files。设置排除列表，以及参与语义检索的文件后缀。
23. **Advanced** — 入口：Settings › System › Advanced。网页搜索的 key、日志级别。

**Other（其他）**
24. **Status bar & logs** — 入口：底部状态栏 › Server logs。查看连接状态和服务日志。
25. **About** — 入口：Options › About。查看版本号和源码链接。

建议：问号按钮复用 `SettingsDialog` 的外壳，正文按上面分节写。风险：功能改了以后文档容易过期，建议每节都标一个对应的入口组件路径，便于以后核对。

---
## 5 硬编码文案统计

口径：
- **扫描范围**：`web/src/**/*.{ts,tsx}`，不含 `*.test.*`、`src/test/`、`*.d.ts` 和 `lib/knowledge/fixture.ts`（只给测试用）。
- **UI**：JSX 文本、JSX 表达式和三元表达式里的字符串、`label` / `heading` 等属性，以及对象里 `label`、`title`、`name` 键对应的带大写或中文的字符串。
- **提示与 tooltip**：`title`、`placeholder`、`tooltip`、`description`、`hint`、`alt` 等属性，以及对象里的 `description`、`hint`、`detail`、`summary` 键。**aria-label 计数单独列一行。**
- **前端自有错误文案**：`showToast`、`toast`、`setError`、`new Error`、`fail` 的第一个参数，以及对象里 `message` / `error` 键对应的、含空格的字符串字面量。
- **排除**：后端透传（变量）、className、URL 和路径、`console.*`、测试。模板字面量里的插值统一记为 `{}`，按静态部分计字符数。
- `lib/shader/*Globals.ts` 是编辑器里 HLSL / Unity / URP 的悬浮文档数据，单列，不进合计。

### 5.1 总表
| 分类 | 条数 | 字符 | 去重条数 | 去重字符 |
|---|---|---|---|---|
| UI | 313 | 5501 | 272 | 5088 |
| 提示与 tooltip（不含 aria-label） | 101 | 1931 | 91 | 1806 |
| 其中单列：aria-label | 74 | 1239 | 72 | 1218 |
| 前端自有错误文案 | 60 | 1622 | 51 | 1358 |
| **合计（不含 lib/shader）** | **548** | **10293** | **486** | **9470** |
| lib/shader 语言文档（单列，不进合计） | 1007 | 34320 | 815 | 30388 |

其中中文文案 15 条（见 §3 #18），其余都是英文。

### 5.2 按目录 / 面板
表中「提示与 tooltip」一列已包含 aria-label，aria-label 另列一列。

| 目录/面板 | UI 条/字符 | 提示与 tooltip 条/字符 | 其中 aria-label 条/字符 | 前端错误 条/字符 | 合计条 |
|---|---|---|---|---|---|
| lib/shader (编辑器语言悬浮文档, 单列) | 347/5524 | 660/28796 | 0/0 | 0/0 | 1007 |
| dockview/panels/settings/AgentsSection | 31/807 | 11/174 | 6/133 | 0/0 | 42 |
| dockview/panels/settings/engines/ | 17/219 | 10/134 | 5/95 | 3/111 | 30 |
| stores/ | 0/0 | 0/0 | 0/0 | 26/686 | 26 |
| components/toolviews/ | 21/276 | 3/43 | 1/33 | 0/0 | 24 |
| components/ContextUsageRing | 18/200 | 5/121 | 1/36 | 0/0 | 23 |
| dockview/panels/settings/McpServersSection | 15/304 | 5/49 | 2/23 | 1/17 | 21 |
| components/SessionStatusLine | 18/179 | 2/31 | 2/31 | 0/0 | 20 |
| dockview/panels/settings/FilesSection | 14/725 | 5/79 | 0/0 | 0/0 | 19 |
| components/knowledge/ | 9/88 | 9/207 | 5/66 | 0/0 | 18 |
| components/FileTree | 14/130 | 0/0 | 0/0 | 3/97 | 17 |
| dockview/panels/SearchPanel | 6/40 | 11/207 | 0/0 | 0/0 | 17 |
| components/AgentChatInput | 6/51 | 5/169 | 2/17 | 5/120 | 16 |
| components/GitPanel | 7/97 | 8/103 | 1/24 | 0/0 | 15 |
| components/ModelSwitcher | 4/108 | 10/277 | 3/52 | 0/0 | 14 |
| components/transcriptMarks | 10/228 | 4/77 | 3/57 | 0/0 | 14 |
| dockview/panels/BrowserPanel | 1/40 | 8/66 | 4/31 | 4/100 | 13 |
| dockview/panels/SettingsDialog | 12/143 | 1/17 | 0/0 | 0/0 | 13 |
| dockview/panels/settings/CustomToolsSection | 7/120 | 5/60 | 2/24 | 0/0 | 12 |
| components/EditorPane | 9/165 | 2/14 | 0/0 | 0/0 | 11 |
| lib/ | 2/22 | 0/0 | 0/0 | 9/283 | 11 |
| components/fileview/ | 10/163 | 0/0 | 0/0 | 0/0 | 10 |
| dockview/panels/settings/AdvancedSection | 5/157 | 4/35 | 0/0 | 0/0 | 9 |
| components/PermissionModal | 5/39 | 3/102 | 1/18 | 0/0 | 8 |
| components/SubagentRosterPanel | 3/58 | 5/25 | 3/21 | 0/0 | 8 |
| dockview/workbench | 8/73 | 0/0 | 0/0 | 0/0 | 8 |
| components/MermaidBlock | 7/81 | 0/0 | 0/0 | 0/0 | 7 |
| components/SessionList | 6/73 | 1/30 | 0/0 | 0/0 | 7 |
| components/FileTreeGlob | 3/58 | 3/85 | 0/0 | 0/0 | 6 |
| components/StatusBar | 3/33 | 3/107 | 2/52 | 0/0 | 6 |
| components/mention/ | 3/47 | 1/19 | 1/19 | 2/56 | 6 |
| dockview/panels/AgentPanel | 3/50 | 2/70 | 1/35 | 1/29 | 6 |
| dockview/panels/TerminalPanel | 1/12 | 5/66 | 3/40 | 0/0 | 6 |
| components/AskUserCard | 2/12 | 3/80 | 2/30 | 0/0 | 5 |
| dockview/config | 5/55 | 0/0 | 0/0 | 0/0 | 5 |
| dockview/panels/AboutPanel | 5/44 | 0/0 | 0/0 | 0/0 | 5 |
| dockview/panels/settings/ModelsSection | 1/85 | 4/72 | 2/22 | 0/0 | 5 |
| dockview/shell/TitleBar | 1/6 | 4/86 | 3/31 | 0/0 | 5 |
| dockview/tabs | 0/0 | 5/27 | 1/7 | 0/0 | 5 |
| components/ImageThumb | 1/11 | 2/45 | 2/45 | 1/17 | 4 |
| components/transcript/ | 2/51 | 2/34 | 2/34 | 0/0 | 4 |
| dockview/panels/SubagentReadOnlyPanel | 3/79 | 0/0 | 0/0 | 1/29 | 4 |
| dockview/panels/settings/ProvidersSection | 1/32 | 3/46 | 1/14 | 0/0 | 4 |
| api/ | 0/0 | 0/0 | 0/0 | 3/63 | 3 |
| components/ConflictCard | 2/38 | 1/16 | 1/16 | 0/0 | 3 |
| components/MiniChatInput | 1/15 | 2/35 | 0/0 | 0/0 | 3 |
| components/NotificationBell | 2/21 | 1/17 | 0/0 | 0/0 | 3 |
| components/SessionItem | 1/10 | 2/126 | 1/63 | 0/0 | 3 |
| components/ui/ | 0/0 | 3/44 | 3/44 | 0/0 | 3 |
| components/AgentMarkdown | 1/13 | 1/18 | 1/18 | 0/0 | 2 |
| components/ModalityIcons | 0/0 | 2/32 | 1/16 | 0/0 | 2 |
| components/ToolCallCard | 2/13 | 0/0 | 0/0 | 0/0 | 2 |
| components/VersionTags | 0/0 | 2/49 | 1/32 | 0/0 | 2 |
| dockview/shell/BrowserHeaderAction | 0/0 | 2/22 | 1/11 | 0/0 | 2 |
| components/CitationChip | 0/0 | 1/9 | 1/9 | 0/0 | 1 |
| components/InlineToolRow | 1/4 | 0/0 | 0/0 | 0/0 | 1 |
| components/LlmReconnectBubble | 1/5 | 0/0 | 0/0 | 0/0 | 1 |
| components/MessageList | 0/0 | 1/21 | 1/21 | 0/0 | 1 |
| components/ToastHost | 0/0 | 1/7 | 1/7 | 0/0 | 1 |
| components/ToolContentView | 1/192 | 0/0 | 0/0 | 0/0 | 1 |
| dockview/AppShellDockview.tsx | 0/0 | 1/5 | 0/0 | 0/0 | 1 |
| dockview/components | 0/0 | 1/12 | 1/12 | 0/0 | 1 |
| dockview/menu | 1/7 | 0/0 | 0/0 | 0/0 | 1 |
| dockview/panels/settings/shared | 0/0 | 0/0 | 0/0 | 1/14 | 1 |
| dockview/watermark | 1/22 | 0/0 | 0/0 | 0/0 | 1 |

### 5.3 误差
- **抽样核对**：随机抽 20 条人工核对，18 条分类正确，误差约 10%。错的 2 条：一条是 `lib/knowledge/fixture.ts` 的测试数据，已在脚本里排除；另一条是 `EmbedModelDownload.tsx:70` 的进度提示「Downloading embedding model…」，因为在 `message` 键下，被误归为错误文案。
- **漏计**：放在常量表里、靠变量间接传给 JSX 的文案，脚本只认 label/title/description 等几个键名，其余会漏掉；字符串拼接也会漏。
- **多计**：错误类里有少量开发者自用的不变量报错，例如「composer "{}" is not mounted」「terminal/create missing id」。多语言时可以不翻，约 5 条。
- **估量建议**：多语言工作量按「去重 486 条、约 9.5k 字符」估算，再加 10–15% 余量；lib/shader 是否翻译单独决定。

### 5.4 可复跑命令
```
# 准备工具（仓库外）
mkdir -p /tmp/wp152-tools && cd /tmp/wp152-tools && PATH=/exec-daemon:$PATH npm i --prefix /tmp/wp152-tools knip ts-prune jscpd typescript
# 死代码
cd /workspace/litecode/web && PATH=/exec-daemon:$PATH /tmp/wp152-tools/node_modules/.bin/knip --reporter json > /tmp/wp152/knip.json
cd /workspace/litecode/web && PATH=/exec-daemon:$PATH /tmp/wp152-tools/node_modules/.bin/ts-prune -p tsconfig.app.json > /tmp/wp152/tsprune.txt
cd /workspace/litecode/web && rg -v "used in module" /tmp/wp152/tsprune.txt | rg -v "workbench/index.ts" | while read l; do f=${l%%:*}; s=${l##* - }; prod=$(rg -lw "$s" src -g '!*.test.*' -g '!src/test/**' | rg -v "^$f$" | wc -l); tst=$(rg -lw "$s" src -g '*.test.*' | wc -l); self=$(rg -cw "$s" $f); echo "$f $s prod=$prod test=$tst self=$self"; done
# 重复
cd /workspace/litecode/web && PATH=/exec-daemon:$PATH /tmp/wp152-tools/node_modules/.bin/jscpd src --min-tokens 50 --ignore "**/*.test.*,**/test/**" --reporters json --output /tmp/wp152/jscpd --silent
# 文案统计
/exec-daemon/node /tmp/wp152/count-strings.mjs /workspace/litecode/web/src > /tmp/wp152/strings.json
python3 /tmp/wp152/aggregate.py /tmp/wp152/strings.json
```

### 5.5 脚本原文：`/tmp/wp152/count-strings.mjs`
```js
// WP-152 read-only hardcoded UI string counter for web/src.
// Usage (read-only; uses web/node_modules/typescript JS API, TS 7 in /tmp has no JS API): /exec-daemon/node /tmp/wp152/count-strings.mjs /workspace/litecode/web/src > /tmp/wp152/strings.json
import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";
const require = createRequire(import.meta.url);
const tsMod = require("/workspace/litecode/web/node_modules/typescript");
const ts = tsMod.default ?? tsMod;
const root = process.argv[2];
const HINT_ATTRS = new Set(["title", "aria-label", "placeholder", "tooltip", "description", "hint", "helpText", "aria-description", "alt"]);
const UI_ATTRS = new Set(["label", "heading", "emptyText", "confirmLabel", "cancelLabel", "buttonLabel"]);
const ERR_CALLS = new Set(["showToast", "toast", "setError", "Error", "setErrorMessage", "fail"]);
const out = [];
function files(d) {
  return fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(d, e.name);
    if (e.isDirectory()) return e.name === "test" ? [] : files(p);
    if (!/\.(ts|tsx)$/.test(e.name) || /\.test\.|\.d\.ts$/.test(e.name) || p.endsWith("lib/knowledge/fixture.ts")) return [];
    return [p];
  });
}
// Only JSX text / known text props / error calls are visited, so any string with
// 2+ letters that is not a URL or path counts as human text.
const isHuman = (s) => /[A-Za-z\u4e00-\u9fff]{2,}|[\u4e00-\u9fff]/.test(s) && !/^(https?:|\/|\.\/|#)/.test(s);
function text(n) {
  if (ts.isParenthesizedExpression(n)) return text(n.expression);
  if (ts.isConditionalExpression(n)) {
    const a = text(n.whenTrue), b = text(n.whenFalse);
    return a != null || b != null ? [a, b].filter((x) => x != null).join(" | ") : null;
  }
  if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return n.text;
  if (ts.isTemplateExpression(n)) return [n.head.text, ...n.templateSpans.map((s) => s.literal.text)].join("{}");
  return null;
}
function add(file, n, cat, s) {
  s = s.replace(/\s+/g, " ").trim();
  if (!s || !isHuman(s)) return;
  const sf = n.getSourceFile();
  out.push({ file: path.relative(root, file), line: sf.getLineAndCharacterOfPosition(n.getStart()).line + 1, cat, text: s });
}
for (const file of files(root)) {
  const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true, file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const visit = (n) => {
    if (ts.isJsxText(n)) add(file, n, "ui", n.text);
    else if (ts.isJsxAttribute(n) && n.initializer) {
      const name = n.name.getText();
      let v = n.initializer;
      if (ts.isJsxExpression(v) && v.expression) v = v.expression;
      const t = text(v);
      if (t != null) {
        if (name === "aria-label") add(file, n, "hint:aria", t);
        else if (HINT_ATTRS.has(name)) add(file, n, "hint", t);
        else if (UI_ATTRS.has(name)) add(file, n, "ui", t);
      }
    } else if (ts.isJsxExpression(n) && n.expression && ts.isJsxElement(n.parent)) {
      const t = text(n.expression);
      if (t != null) add(file, n, "ui", t);
    } else if (ts.isPropertyAssignment(n) && (ts.isIdentifier(n.name) || ts.isStringLiteral(n.name))) {
      // Object-literal label tables (section lists, menu items, status maps).
      const key = n.name.text;
      const t = text(n.initializer);
      if (t != null) {
        if (["label", "title", "heading", "name"].includes(key) && /[A-Z\u4e00-\u9fff]/.test(t)) add(file, n, "ui", t);
        else if (["description", "hint", "tooltip", "placeholder", "help", "detail", "summary"].includes(key)) add(file, n, "hint", t);
        else if (["ariaLabel"].includes(key)) add(file, n, "hint:aria", t);
        else if (["message", "error"].includes(key) && /\s/.test(t)) add(file, n, "error", t);
      }
    } else if ((ts.isCallExpression(n) || ts.isNewExpression(n)) && n.arguments?.length) {
      const callee = n.expression.getText().split(".").pop();
      if (ERR_CALLS.has(callee)) {
        const t = text(n.arguments[0]);
        if (t != null) add(file, n, "error", t);
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
console.log(JSON.stringify(out, null, 1));
```

### 5.6 脚本原文：`/tmp/wp152/aggregate.py`
```python
# Usage: python3 /tmp/wp152/aggregate.py /tmp/wp152/strings.json
import json,sys,collections
d=json.load(open(sys.argv[1]))
def group(f):
    if f.startswith('lib/shader/'): return 'lib/shader (编辑器语言悬浮文档, 单列)'
    p=f.split('/')
    if p[0]=='dockview' and len(p)>2 and p[1]=='panels' and p[2] in('settings',):
        return 'dockview/panels/settings/'+(p[3].replace('.tsx','') if len(p)==4 else p[3]+'/')
    if p[0]=='dockview': return '/'.join(p[:2])+('/'+p[2].replace('.tsx','') if p[1] in ('panels','shell') and len(p)>2 else '')
    if p[0]=='components' and len(p)>2: return 'components/'+p[1]+'/'
    if p[0]=='components': return 'components/'+p[1].replace('.tsx','')
    return p[0]+'/'
cats=['ui','hint','hint:aria','error']
rows=collections.defaultdict(lambda:{c:[0,0] for c in cats})
for x in d:
    g=group(x['file']); r=rows[g][x['cat']]; r[0]+=1; r[1]+=len(x['text'])
def tot(filter_):
    t={c:[0,0] for c in cats}; u={c:set() for c in cats}
    for x in d:
        if filter_(x): t[x['cat']][0]+=1; t[x['cat']][1]+=len(x['text']); u[x['cat']].add(x['text'])
    return t,u
main=lambda x: not x['file'].startswith('lib/shader/')
t,u=tot(main)
print('| 分类 | 条数 | 字符 | 去重条数 | 去重字符 |\n|---|---|---|---|---|')
for c in cats: print(f"| {c} | {t[c][0]} | {t[c][1]} | {len(u[c])} | {sum(map(len,u[c]))} |")
print(f"| 合计（不含 lib/shader） | {sum(v[0] for v in t.values())} | {sum(v[1] for v in t.values())} | {sum(len(v) for v in u.values())} | {sum(sum(map(len,v)) for v in u.values())} |")
t2,u2=tot(lambda x: not main(x))
print(f"| lib/shader 语言文档（单列） | {sum(v[0] for v in t2.values())} | {sum(v[1] for v in t2.values())} | {sum(len(v) for v in u2.values())} | {sum(sum(map(len,v)) for v in u2.values())} |")
print('\n| 目录/面板 | UI 条/字符 | 提示与 tooltip 条/字符 | 其中 aria-label 条/字符 | 前端错误 条/字符 | 合计条 |\n|---|---|---|---|---|---|')
for g in sorted(rows, key=lambda g:-sum(v[0] for v in rows[g].values())):
    r=rows[g]; h=[r['hint'][0]+r['hint:aria'][0], r['hint'][1]+r['hint:aria'][1]]
    print(f"| {g} | {r['ui'][0]}/{r['ui'][1]} | {h[0]}/{h[1]} | {r['hint:aria'][0]}/{r['hint:aria'][1]} | {r['error'][0]}/{r['error'][1]} | {sum(v[0] for v in r.values())} |")
```

---
## 待 Andy 定（汇总）
| # | 节 | 条目 | 选项 |
|---|---|---|---|
| 1 | §1.1 #22–23 | devDependency `remotion`、`@remotion/cli` 有没有用（宣传视频？） | 删除 / 保留并写明用途 |
| 2 | §1.2 #13 | `scripts/ws-smoke.mjs` 手动冒烟脚本 | 保留并写进 README / 删除 |
| 3 | §1.2 #9 | `dockview/workbench/index.ts` 桶文件的公共 API 是否保留 | 精简 / 保留 |
| 4 | §1.2 #1–7 | api/types 里 7 个没用到的协议类型 | 跟后端协议核对后再删 / 保留作文档 |
| 5 | §3 #8 | max steps 提示的语气（「a saner default」） | 用建议文 / 维持原文 |
| 6 | §3 #18 | 界面里夹着 15 条中文文案 | 统一成英文 / 直接进多语言包 |
| 7 | §5 | lib/shader 悬浮文档（1007 条、约 34k 字符）要不要进多语言范围 | 纳入 / 不纳入 |
| 8 | §4 | 问号弹框收哪些功能、分节是否照此 | 确认大纲 / 调整 |

## 结束时 git status（零改动证明）
```
 M .litecode/ops/board.json
 M .litecode/ops/events/inbox.json
 M .litecode/ops/events/outbox.jsonl
 M .litecode/ops/log.jsonl
 M .litecode/ops/tool_traces.jsonl
 M .litecode/ops/tools/board.md
?? .litecode/ops/artifacts/WP-147-fill.txt
?? .litecode/ops/artifacts/WP-151-desktop-audit.md
?? .litecode/ops/artifacts/WP-151-fill.txt
?? .litecode/ops/artifacts/WP-151-web-audit.md
```
（2026-10-14 结束时）web/ 下 `git status --short .` 和 `git diff --stat -- web` 都为空：产品代码、package.json、锁文件、配置零改动，web/ 下也没有残留的工具缓存。和开工前相比，只新增了本产物 `WP-151-web-audit.md`，另外有一个 desktop 子包自己的产物 `WP-151-desktop-audit.md`（不是本包写的）。
