# WP-151 前端瘦身调研 · 总产物（frontend_mid 合成，只读）

- 输入：`WP-151-fill.txt`（计划）、`WP-151-web-audit.md`（WP-152，前端-web）、`WP-151-desktop-audit.md`（WP-153，前端-desktop）。
- 本文只做合成、跨端裁定和抽查；逐条明细、工具原始输出、脚本全文以两份子产物为准，本文按编号引用（web 用 W，desktop 用 D/S/R/C/A）。
- 基线：分支 `ops/team-framework`，web 基线 HEAD `3ddc643`，日期 2026-10-14。
- 规则：产品代码、package.json、锁文件零改动，未 commit。后端原样透传的报错不在统计和删改范围内。

---
## 0 计数总览

| 节 | web | desktop + scripts | 合计 |
|---|---|---|---|
| 1 死代码 · 确认可删 | 24 | 7 组（D1–D7，展开为 2 处删除 + 2 处未用参数 + 23 个去 export + 1 个再导出） | 31 组 |
| 1 死代码 · 疑似 | 14 组（#1–7 为 7 个协议类型） | 7（S1–S7，其中 S4、S7 结论为保留） | 21 |
| 1 附 · 未声明依赖 | 2（dockview、@tanstack/virtual-core） | 0 | 2 |
| 2 重复组件 | 10 组 | 8 组（R1–R8，其中 R1–R3 跨端） | 18 组，其中跨端 5 项（见 §2.3） |
| 3 多余提示文案 | 18 | 11 | 29 |
| 4 功能大纲 | 25 节 | 8 节 | 合并后 30 节（见 §4） |
| 5 硬编码文案（计入合计） | 548 条 / 10293 字符（去重 486 / 9470） | 89 条 / 1897 字符（不去重） | 637 条 / 12190 字符 |
| 5 单列，不进合计 | aria-label 74 / 1239（已含在 548 内，单列一行）；lib/shader 1007 / 34320 | new Error 142 / 5368 | — |
| 6 待 Andy 定 | 8 | 5（A1–A5） | 13 |

---
## 1 死代码

### 1.1 确认可删

**web（24 条，原文见 web 产物 §1.1）**

| # | 路径 / 符号 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| W1 | `src/api/json.ts`（`JsonValue`） | knip 未用文件；rg 无引用 | 删文件 | 低 |
| W2 | `src/theme/dockview.ts` | knip 未用文件；`AppShellDockview.tsx:11` 直接 import adapter | 删文件 | 低 |
| W3 | `src/theme/dockview.css`（133 行） | knip 未用文件；`theme/index.css:3` 引的是 `./dockview/adapter.css`；它是旧副本 | 删文件 | 低；jscpd 132 行重复随之消失 |
| W4 | `src/utils/turnPhase.ts` | knip 未用文件；rg 无 import | 删文件 | 低 |
| W5 | `components/knowledge/KnowledgeRelationStrip.tsx` | knip 未用文件；rg 无引用 | 删文件 | 低；若是知识库规划中组件，删前确认 |
| W6 | `api/adapter.ts:274 isEmptyItemShell` | prod=0 test=0 self=1 | 删函数 | 低 |
| W7 | `api/adapter.ts:311 extractToolFilePath` | 同上 | 删函数 | 低 |
| W8 | `components/Logo.tsx:18 skipSplash` | 同上 | 删导出 | 低 |
| W9 | `lib/citationResolve.ts:28 peekCitation` | 同上 | 删函数 | 低 |
| W10 | `lib/litecodeLsp.ts:1641 refreshDiagnostics` | 同上 | 删函数 | 低到中；删后跑 tsc 和测试 |
| W11 | `lib/theme.ts:52 toggleTheme` | 同上 | 删函数 | 低 |
| W12 | `stores/settingsDocuments.ts:168 isRevisionedDocument` | 同上 | 删函数 | 低 |
| W13 | `stores/settingsStore.ts:698 mcpServersFromStore` | 同上 | 删函数 | 低 |
| W14 | `stores/turnStore.ts:105 headPermission` | 同上 | 删函数 | 低 |
| W15 | `knowledge/KnowledgeMarkdown.tsx:111 KnowledgeInlineBody` | 同上 | 删组件 | 低 |
| W16 | `knowledge/KnowledgeMarkdown.tsx:167 KnowledgeStatusBadge` | 同上 | 删组件 | 低 |
| W17 | `transcript/transcriptUserRailMarks.ts:134 newestUserSeq` | 同上 | 删函数 | 低 |
| W18 | `dockview/workbench/model.ts:54 groupHasComponent` | 同上 | 删函数 | 低 |
| W19 | `lib/knowledge/markers.ts:321 extractSymbolRefs` | 同上 | 删函数 | 低 |
| W20 | `PermissionModal.tsx:10` 中转导出 `PermissionGrantOpts` | 定义在 `AskCard.tsx`，无人从 PermissionModal 引入 | 删这一行 | 低 |
| W21 | `dockview/menu/menuItems.ts:27 menuItems` | 注释写「for tests」但没有测试用；MenuBar 用 `buildMenuItems` | 删常量 | 低 |
| W22 | devDependency `remotion` | knip 未用；全仓只在 package.json / 锁文件出现 | 删依赖（放后续包，会改 package.json 与锁文件） | 低；待 Andy 定（U1） |
| W23 | devDependency `@remotion/cli` | 同上 | 同上 | 同上 |
| W24 | devDependency `@types/opentype.js` | 全仓无 opentype 引用 | 删依赖（后续包） | 低 |

**desktop + scripts（7 组，原文见 desktop 产物 §1.1）**

| # | 路径 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| D1 | `desktop/src/remote-state.ts:14 managedRemoteAfterSuccessfulConnect` | knip；rg 只在定义处 1 次；函数体只是 `return next` | 删函数 | 低 |
| D2 | `main.ts:703 scheduleTunnelReconnect` 的 `workspace` 参数 | tsc TS6133；唯一调用点 :741 | 删参数并改调用点 | 低 |
| D3 | `main.ts:398` `find((a, i, arr) => …)` | tsc TS6133 | 改 `_a` 或 `indexOf` | 无 |
| D4 | `components.test.ts:375` `init` 参数 | tsc TS6133 | 改 `_init` | 无 |
| D5 | 16 个只在本文件用的 `export`（browser-host、bundle-paths、components、dev-ui、hub、ipc-trust、ssh-session） | knip；`rg -lw` 只命中定义文件且文件内 ≥2 次 | 只去掉 `export`，不删实现 | 低 |
| D6 | 7 个只在本文件用的导出类型（components、instance-registry、remote-progress、ssh-session） | 同 D5 | 同 D5 | 低 |
| D7 | `instance-registry.ts:9` 再导出 `stripVerbatimLap` | 调用方都直接从 `./lap-path` 导入 | 去掉再导出 | 低 |

scripts：没有可直接删的文件（调用方证据见 desktop 产物 §1 末尾）。

### 1.2 疑似，需复核

**web（14 条）**

| # | 路径 / 符号 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| W-S1–7 | `api/types.ts`：SessionAttached(446)、CompactStarted(661)、JsonRpcRequest(772)、JsonRpcResponse(779)、JsonRpcNotification(786)、LspNotification(803)、TransportRequest(814) | prod=0 test=0；协议镜像类型 | 与后端协议核对，废弃的删 | 中；待 Andy 定（U4） |
| W-S8 | `workbench/queries.ts:205 LivePanel` | prod=0 | 随 W-S9 处理 | 低 |
| W-S9 | `dockview/workbench/index.ts` 桶文件（约 50 个转发） | 只有 `BrowserHeaderAction.tsx:4` 用 `groupRole` | 精简，或改直接 import 子模块 | 中；待 Andy 定（U3） |
| W-S10 | `settings/providerBilling.ts:22 API_PROVIDER_IDS` | prod=0；未知 provider 一律归 API | 删或改为注释 | 低 |
| W-S11 | `theme/tokens.ts:89 readDkTokens` | 只在 `theme/README.md:72` 提到 | 删并改 README，或留作公开 API | 低 |
| W-S12 | `src/lib/knowledge/fixture.ts` | 只被 3 个测试 import | 移到 `src/test/fixtures` | 低 |
| W-S13 | `scripts/ws-smoke.mjs` | 无 npm script 引用 | 写进 README 或删 | 低；待 Andy 定（U2） |
| W-S14 | ts-prune 209 个「只在模块内使用」的导出 | `/tmp/wp152/tsprune.txt` | 后续包批量去 `export` | 低 |

**desktop + scripts（7 条）**

| # | 路径 | 证据 | 建议 | 风险 |
|---|---|---|---|---|
| S1 | `preload.ts:197-211` pause/stop/retryComponent 及 `main.ts:1538` handler | web 与 hub 无调用方 | 待 Andy 定（A1） | 删后重做需加回 |
| S2 | `preload.ts:178 onRemoteDisconnected` 及 `main.ts:125/680 emitRemoteDisconnected` | 发了事件无人订阅，断开时用户无提示 | 待 Andy 定（A2） | 静默 UX 缺口 |
| S3 | `preload.ts:219 focusWorkspace` 及 `main.ts:1706` | web 无调用方，只在 auth.ts 类型里 | 确认 openWorkspace 已覆盖后删 | 中：多实例行为 |
| S4 | knip 报 `electron-builder` 未用 | 误报：`dist:win`/`pack:dir`、`package_win.ps1:40` 在用 | 保留 | — |
| S5 | `scripts/package_local_cuda.ps1` | 无引用，文件头「Personal CUDA nightly」 | 待 Andy 定（A3） | 可能有人手动在用 |
| S6 | ps1 的 Deprecated no-op 参数（package_win.ps1:18 等） | 注释写明废弃 | workflows rg 不到后删 | 外部旧调用会报错 |
| S7 | `scripts/litecode.service.example` | 被 package_linux.sh 打进包 | 保留 | — |

### 1.3 web 直接 import dockview，但只声明了 dockview-react

- 事实：`web/src/theme/dockview/adapter.ts:1` 写 `import type { DockviewTheme } from "dockview"`；`web/package.json:34` 只声明 `"dockview-react": "^8.4.1"`。`dockview` 是 `dockview-react` 的依赖（`node_modules/dockview-react/package.json:88` `"dockview": "^8.4.1"`），web 只是靠传递依赖拿到它。中层复核属实。
- 同类：`prependAnchor.test.ts` 引用 `@tanstack/virtual-core`，也未声明。
- 建议：后续包把 `dockview`（与 dockview-react 同版本）和 `@tanstack/virtual-core` 显式加进 web 的 package.json。本包不改。
- 风险：低到中。现在能跑；dockview-react 升级或包管理器提升策略变化时，可能装不到或版本不一致导致类型断裂。目前仅 `import type`，运行时不受影响。

### 1.4 中层抽查（rg 复核 web 确认可删项）

在 `web/src` 和 `desktop/src` 下复跑：
```
cd /workspace/litecode/web
for s in isEmptyItemShell extractToolFilePath peekCitation toggleTheme headPermission groupHasComponent extractSymbolRefs newestUserSeq; do rg -cw $s src ../desktop/src; done
rg -l KnowledgeRelationStrip src
rg -n "from ['\"](\./|\.\./)+(api/)?json['\"]" src
rg -l 'utils/turnPhase' src
```
结果：
- W6、W7、W9、W11、W14、W17、W18、W19 共 8 个符号，各自只在定义文件出现 1 次，desktop 无引用。与子产物一致。
- W5 `KnowledgeRelationStrip` 只命中自身文件。一致。
- W1 `api/json`：没有任何 import；`rg 'api/json|/json"'` 命中的只是 `"application/json"` 请求头，不是引用。一致。
- W4 `utils/turnPhase`：无 import；`turnStore.ts` 里的 `turnPhase` 是同名字段，不是对该模块的引用。一致。
- desktop D1 在 WP-153 交叉抽查时已复跑，一致。
- 结论：抽查 11 项，全部与子产物一致，未发现误判。

---
## 2 可合并重复组件

### 2.1 web 内部（10 组，详见 web 产物 §2）

| # | 路径 | 合并目标 | 风险 |
|---|---|---|---|
| W-R1 | `CustomToolsSection.tsx` ↔ `McpServersSection.tsx`（93 行） | `ScopedDefinitionEditor` | 中 |
| W-R2 | `AgentPanel.tsx` ↔ `SubagentReadOnlyPanel.tsx`（70 行） | hook `useSessionPanelBootstrap` | 中 |
| W-R3 | `SubagentReadOnlyContent.tsx` ↔ `AgentPanel.tsx`（63 行） | `TranscriptBody`（`readOnly`） | 中；虚拟列表滚动要回归 |
| W-R4 | AgentTab / EditorTab / KnowledgeGraphTab / BrowserTab | `BaseTab` | 低到中 |
| W-R5 | `ui/Dropdown.tsx` ↔ `ui/Popover.tsx` | `useFloatingLayer` | 中；焦点与 a11y |
| W-R6 | `ModelSwitcher.tsx` ↔ `ui/Select.tsx` | 复用 Select 触发按钮 | 低 |
| W-R7 | FileParamView / WriteToolView / EditToolView | `FilePathHeader` | 低 |
| W-R8 | `AskUserCard.tsx` ↔ `ToastMarkdown.tsx` | `compactMarkdownComponents` | 低 |
| W-R9 | `GitPanel.tsx` ↔ `SearchPanel.tsx` | `CollapsibleSectionHeader` | 低 |
| W-R10 | ImagePreview↔MediaPreview；KnowledgeGraphPanel↔KnowledgePanel | 各合一个外壳 | 低；收益小 |

另：`lib/shader/hlslGlobals.ts` 254 行数据重复、`api/workspace.ts` 59 行请求样板，不计组数，单独评估。

### 2.2 desktop + scripts 内部（R4–R8，详见 desktop 产物 §2）

| # | 分组 | 结论 | 风险 |
|---|---|---|---|
| R4 | 三套进度阶段类型 | 不合并，只去 export（D6） | — |
| R5 | 版本号读取三处（ps1 / sh / mjs） | 统一一个入口或共用正则 | 低到中：发布版本号 |
| R6 | sh/ps1 成对脚本 | 不合并；文件头互相标注，加一致性断言 | 低 |
| R7 | ps1 小工具函数重复 | 抽 `scripts/lib/common.ps1` | 低；CI 要回归 |
| R8 | 「Opening workspace…」两层各发一次 | 见 §3 C4 | 低 |

### 2.3 跨端裁定（每项一个方向）

| # | 项 | 现状 | 唯一源（合并方向） | 范围 | 风险 |
|---|---|---|---|---|---|
| X1 | `web/src/theme/tokens.css` | desktop `scripts/copy-theme-tokens.mjs:6` 与 `src/hub.ts:10-12` 直接读 web 这份 | **web 为唯一源**，维持现状；它不是重复，是共享源 | 不动。web 改 token 时要知道 desktop 也在读，在文件头加一行说明即可 | 低；不能删、不能挪 |
| X2 | 滚动条样式 | `desktop/src/hub.ts:106-110` 注释写明对齐 `web/src/theme/index.css`，两边手抄 | **web 为唯一源**：把滚动条规则移进 tokens.css（或一个同样由 copy-theme-tokens 复制的共享片段），hub 走 X1 同一通道读取，删掉 hub 内联副本 | web `theme/index.css`、`tokens.css`；desktop `hub.ts` | 低；hub 首页列表滚动条要目测一次 |
| X3 | 桥接接口类型（R1） | 实现在 `desktop/src/preload.ts`（71 键）；web `api/auth.ts LitecodeDesktopBridge` 手抄，全可选，已漂移 | **desktop 为唯一源**：preload 实现即契约，抽出 `bridge-contract.d.ts`（类型-only，由 desktop 持有），web 只 import 类型，删除 auth.ts 手写镜像 | `preload.ts`、`auth.ts`、两侧 tsconfig 路径或 include；同时处理 S1–S3 的键（按 A1/A2 裁定保留或删） | 中；涉及构建边界。只有一份类型，不保留旧镜像，不做兼容层 |
| X4 | 组件进度类型（R2） | desktop `ComponentProgressEvent`/`ComponentPhase` ↔ web `DesktopComponentProgress`（缺 status、percent、bytesPerSecond） | **desktop 为唯一源**，并入 X3 契约文件 | `components.ts`、`auth.ts`、`EmbedModelDownload.tsx` 及其 2 个测试 | 低到中 |
| X5 | 下载百分比（R3） | desktop `components.ts` 已发 `percent`；web `EmbedModelDownload.tsx:95 percent()` 又算一遍 | **desktop 为唯一源**：契约里 `percent` 定为必填，web 只读 `event.percent`，删掉 web 的 `percent()`。不保留「没有时再回退」（与单一格式规则一致，修正 desktop 产物 R3 的回退建议） | `EmbedModelDownload.tsx`、`embedInstall.test.ts`、`EmbedModelDownload.test.tsx` | 低；X5 依赖 X4 先落 |

建议顺序：X1（加说明）→ X3+X4 同一包 → X5 → X2。

---
## 3 多余提示文案（按面板）

标注：可删 / 可合并 / 可缩短。原文与建议文全文见子产物（web §3，desktop §3）。

**web · 设置**

| # | 面板 | 位置 | 标注 | 建议要点 | 风险 |
|---|---|---|---|---|---|
| W-C1 | Files | `FilesSection.tsx:130-135` | 可缩短 | 存储位置 + 立即生效 + 后缀表一句话 | 低 |
| W-C2 | Files | `FilesSection.tsx:188-189` | 可缩短，可与 W-C1 合并 | Extra hides for search and glob. | 低 |
| W-C3 | Files | `FilesSection.tsx:210-211` | 可缩短 | File suffixes to index, one per line. | 低 |
| W-C4 | MCP | `McpServersSection.tsx:327-328` | 可缩短 | Workspace overrides global. Bind mcp_<id> on Agents. | 低 |
| W-C5 | MCP | `McpServersSection.tsx:249` | 可缩短 | Save, then Start. Restart after config changes. | 低 |
| W-C6 | Custom Tools | `CustomToolsSection.tsx:222` | 可合并（与 W-C4 同口径） | 共用提示组件 | 低；依赖 W-R1 |
| W-C7 | Agents | `AgentsSection.tsx:236-239` | 可缩短 | Replaces the built-in persona (builtin: keeps default). | 低 |
| W-C8 | Agents | `AgentsSection.tsx:266` | 可缩短 | Long tasks may need ~200. | 低；待 Andy 定（U5） |
| W-C9 | Agents | `AgentsSection.tsx:341` | 可缩短 | Select at least one subagent, or launches will fail. | 低 |
| W-C10 | Agents | `AgentsSection.tsx:999` | 可缩短 | Only the model is configurable. | 低 |
| W-C11 | Agents（aria-label） | `AgentsSection.tsx:460` | 可缩短 | 去掉「Click the card to toggle.」 | 中；测试正则要核对 |
| W-C12 | Advanced | `AdvancedSection.tsx:70-72` | 可缩短 | Optional Exa API key (or EXA_API_KEY) for higher limits. | 低 |
| W-C13 | 设置弹框 | `SettingsDialog.tsx:67` | 可缩短 | Agent is busy — saving is paused. | 低 |
| W-C14 | Models / 模型切换 | `ModelsSection.tsx:232`、`ModelSwitcher.tsx:364,594` | 可合并（3→1） | No models yet — add a provider key in Settings › Provider. | 低 |

**web · 工作区与聊天**

| # | 面板 | 位置 | 标注 | 建议要点 | 风险 |
|---|---|---|---|---|---|
| W-C15 | 标题栏 / 状态栏 | `TitleBar.tsx:68`、`StatusBar.tsx:105` | 可合并 | 同一句抽常量或只留一处 | 低 |
| W-C16 | 会话列表 | `SessionItem.tsx:128,134` | 可合并 | aria-label 与 title 共用变量 | 低 |
| W-C17 | 工具视图 | `ToolContentView.tsx:59` | 可缩短 | Held by session {id}; will retry when released. | 低 |
| W-C18 | 聊天区 / 子代理面板 等 7 处 | 15 条中文夹在英文界面 | 统一语言 | 统一英文或进多语言包 | 中；有中文断言测试；待 Andy 定（U6） |

**desktop 壳（Hub、远程向导、系统对话框）**

| # | 位置 | 标注 | 建议要点 | 风险 |
|---|---|---|---|---|
| C1 | hub 首页 lead + 空列表 :568 | 可合并 | 空列表只写 "No workspaces yet." | 低 |
| C2 | hub 远程页 lead | 可删 | 1–4 步已说明流程 | 低 |
| C3 | Token 步骤 3 处 | 可合并（3→1） | Session ready. Token is filled in automatically. | 低；待 Andy 定（A4） |
| C4 | "Opening workspace…" ×3 | 可合并 | 只由主进程发一次 | 低 |
| C5 | 选择工作区目录 3 句 | 可合并/可缩短 | "Connected." + "Pick a folder inside your home directory." | 低 |
| C6 | hub 选择目录提示 | 可缩短 | Select a folder first. | 低 |
| C7 | 第 2 步 "Preparing…"/"Starting…" | 可合并 | 只留一个 | 无 |
| C8 | `main.ts:556-558` 指纹对话框 | 可缩短 | detail 改 "Fingerprint:\n…" | 低；安全措辞，待 Andy 定（A5） |
| C9 | `main.ts:775-777` sidecar 崩溃 | 可缩短 | message 缩短，detail 保留 | 低 |
| C10 | `main.ts:827` | 可缩短 | Code search unavailable on remote (no embed model). | 低 |
| C11 | ssh-session 部署 6 句 | 可缩短（仅 Extracting） | "Extracting…" | 无 |

NSIS 用 electron-builder 默认文案，无自定义；托盘和菜单不存在，均不涉及。

---
## 4 功能介绍大纲（左侧栏底部问号按钮弹框）

形式：复用 `SettingsDialog` 外壳，左侧分节导航，右侧每节一句要点 + 入口。入口写英文界面原文，后附中文要点。只收有界面的功能。【web】【desktop】标明来源；【desktop】节只在桌面版显示。

**Getting started（上手）**
1. 【desktop】**Workspace Hub** — 入口：启动窗口 Local / Remote 与最近列表。打开本地文件夹或远程机器；最近列表可置顶、移除、一键重连。
2. 【desktop】**Open remote (SSH)** — 入口：Hub › Remote（4 步向导）。输入 user@host，选密码、私钥或 SSH agent，自动部署后选择远程目录；首次连接会弹主机指纹确认；最后一步 Copy token / Enter workspace（该步是否保留待 A4）。
3. 【web】**Sessions** — 左侧栏 › Sessions。新建、切换、删除会话；忙碌的会话不能删。
4. 【web】**Agent chat** — Agent 面板 › Message the agent。发消息、附图；执行中发的消息排到下一步；可停止当前回合。
5. 【web】**Model switcher** — 输入框旁模型下拉。切换模型与思考档位。

**Working with the agent（与 agent 协作）**
6. 【web】**Permissions & questions** — 聊天区 Approve / Submit 卡片。
7. 【web】**Plan** — 状态行 › Open plan。
8. 【web】**Subagents** — Agent 面板子代理名单。查看、停止、删除、打开只读视图。
9. 【web】**Context usage** — 上下文环 › Compact context。
10. 【web】**Tool results** — 聊天区工具卡片。展开详情，从卡片打开文件。

**Workspace（工作区）**
11. 【web】**Explorer** — 左侧栏 › Explorer。浏览、glob 过滤、拖动复制、右键菜单与打开终端。
12. 【web】**Search** — 左侧栏 › Search。全文与会话搜索，Aa 区分大小写。
13. 【web】**Source Control** — 左侧栏 › Source Control。暂存、放弃、提交（Ctrl+Enter）、Pull、提交图。
14. 【web】**Editor & previews** — 在 Explorer 打开文件。代码编辑；Markdown、图片、媒体、SQLite 预览。
15. 【web】**Terminal** — 底部 › Terminal。
16. 【web】**Browser** — 标签栏 › New Browser。前进、后退、输入地址。
17. 【desktop】**Browser pop-out pin** — 浏览器弹出窗口 chrome 的 pin。弹出窗口置顶。
18. 【web】**Knowledge / Knowledge Graph** — 左侧栏 › Knowledge；面板 › Knowledge Graph。

**Layout & window（布局与窗口）**
19. 【web】**Panels & tabs** — 标签右键 › Maximize / Rename / Popout Window / Return to Main Window。
20. 【web】**Options menu** — 标题栏 › Options。Home、Settings...、Theme: Dark / Light、About。
21. 【desktop】**Window controls** — Hub 与工作台标题栏。最小化、最大化/还原、关闭（无边框窗口）。
22. 【desktop】**Drag & drop files / theme sync** — 把文件拖进窗口得到真实路径；主题与宿主同步。

**Settings（设置）**
23. 【web】**Provider / Models** — Settings › LLM。
24. 【web】**Agents** — Settings › Agent › Agents。
25. 【web】**Custom Tools / MCP** — Settings › Agent。需在 Agents 里绑定才生效。
26. 【web】**Engines** — Settings › System › Engines。启停 LSP 与检索引擎。
27. 【web+desktop】**Embedding model download** — Settings › System › Engines 下载卡片。桌面版首次使用时下载并显示进度（暂停/停止/重试目前无界面，见 A1，不写入）。
28. 【web】**Files** — Settings › System › Files。
29. 【web】**Advanced** — Settings › System › Advanced。

**Other（其他）**
30. 【web】**Status bar & logs / About** — 状态栏 › Server logs；Options › About。

不收：远程断开提示（S2，当前无界面）、组件暂停/重试（S1）。风险：功能变动后文档会过期，每节在源文件里标对应入口组件路径，便于核对。整体确认待 Andy 定（U8）。

---
## 5 硬编码文案统计

### 5.1 合计

| 端 | 分类 | 条数 | 字符 |
|---|---|---|---|
| web | UI | 313 | 5501 |
| web | 提示与 tooltip（不含 aria-label） | 101 | 1931 |
| web | aria-label（单列） | 74 | 1239 |
| web | 前端自有错误文案 | 60 | 1622 |
| web | **小计** | **548** | **10293**（去重 486 / 9470） |
| desktop | UI | 63 | 1355 |
| desktop | hint + tooltip（含 hub 的 aria-label，未单拆） | 16 | 171 |
| desktop | error | 10 | 371 |
| desktop | **小计** | **89** | **1897**（不去重） |
| **合计** | | **637** | **12190** |

单列，不进合计：
- web `lib/shader/*Globals.ts` 悬浮文档：1007 条 / 34320 字符（去重 815 / 30388）。是否纳入多语言待 Andy 定（U7）。
- desktop `new Error(...)`：142 条 / 5368 字符。大多是内部错误，少数透传到 hub 状态栏，做多语言时逐条看。
- web 的 aria-label 74 条已含在 548 内，单列一行便于另行决定。

口径差异（合并时注意）：web 给了去重数，desktop 按出现次数计不去重；web 错误类按调用点识别（含 `new Error`），desktop 把 `new Error` 剔出单列。多语言估量建议：web 去重 486 条约 9.5k 字符 + desktop 89 条约 1.9k 字符，再加 10–15% 余量。中文夹杂 15 条（web §3 #18）。误差：web 抽样 20 条错 2 条（约 10%），常量表间接文案与字符串拼接会漏计。desktop 约 85% 集中在 hub.ts 与 main.ts，hub 需先把 HTML 模板改成键值表。

### 5.2 复跑命令

web（工具在 `/tmp/wp152-tools`，脚本全文见 web 产物 §5.5–5.6）：
```
mkdir -p /tmp/wp152-tools && cd /tmp/wp152-tools && PATH=/exec-daemon:$PATH npm i --prefix /tmp/wp152-tools knip ts-prune jscpd typescript
cd /workspace/litecode/web && PATH=/exec-daemon:$PATH /tmp/wp152-tools/node_modules/.bin/knip --reporter json > /tmp/wp152/knip.json
cd /workspace/litecode/web && PATH=/exec-daemon:$PATH /tmp/wp152-tools/node_modules/.bin/ts-prune -p tsconfig.app.json > /tmp/wp152/tsprune.txt
cd /workspace/litecode/web && PATH=/exec-daemon:$PATH /tmp/wp152-tools/node_modules/.bin/jscpd src --min-tokens 50 --ignore "**/*.test.*,**/test/**" --reporters json --output /tmp/wp152/jscpd --silent
/exec-daemon/node /tmp/wp152/count-strings.mjs /workspace/litecode/web/src > /tmp/wp152/strings.json
python3 /tmp/wp152/aggregate.py /tmp/wp152/strings.json
```

desktop（工具在 `/tmp/audit`，脚本全文见 desktop 产物 §5）：
```
ELECTRON_SKIP_BINARY_DOWNLOAD=1 npm i --prefix /tmp/audit knip typescript@5.8 @types/node@24 electron@44 --ignore-scripts --no-audit --no-fund
cd /workspace/litecode/desktop && node /tmp/audit/node_modules/knip/bin/knip.js --config /tmp/audit/knip.wp153.json --no-progress --reporter compact > /tmp/wp153-knip.txt
cd /tmp/audit && node node_modules/typescript/bin/tsc -p tsconfig.wp153.json > /tmp/wp153-tsc.txt
node /tmp/wp153-strings.mjs /workspace/litecode          # 汇总表
node /tmp/wp153-strings.mjs /workspace/litecode --list   # 逐条明细
```
注意：脚本都在 /tmp，机器清理后需按子产物里的脚本全文重建。

---
## 6 待 Andy 定（合并表）

| # | 来源 | 问题 | 选项 | 我们的建议 | 风险 |
|---|---|---|---|---|---|
| U1 | web W22–23 | devDependency `remotion`、`@remotion/cli` 全仓无用，是否为宣传视频预留 | 删除 / 保留并写明用途 | 删除（在后续依赖包里做） | 低；若确有手动用途，删后需重装 |
| U2 | web W-S13 | `scripts/ws-smoke.mjs` 手动 WS 冒烟脚本 | 保留并写进 README / 删除 | 保留并写进 README（WP-154 延迟调研正需要 WS 脚本） | 低 |
| U3 | web W-S9 | `dockview/workbench/index.ts` 桶文件公共 API | 精简为实际用到的 / 保留 / 改直接 import 子模块 | 精简为实际用到的 | 中；公共 API 是有意设计 |
| U4 | web W-S1–7 | `api/types.ts` 7 个未用协议类型 | 与后端协议核对后删废弃的 / 全保留作文档 | 核对后删废弃的，其余保留 | 中；删多了类型漂移难发现 |
| U5 | web W-C8 | max steps 提示语气「a saner default」 | 用建议文 / 维持原文 | 用建议文 "Long tasks may need ~200." | 低 |
| U6 | web W-C18 | 英文界面里夹 15 条中文文案 | 先统一成英文 / 直接进多语言包 | 先统一英文，多语言包另起 | 中；有中文断言测试要改 |
| U7 | web §5 | lib/shader 悬浮文档（1007 条 / 约 34k 字符）是否进多语言 | 纳入 / 不纳入 | 不纳入（是 HLSL/Unity 参考资料，非界面文案） | 低 |
| U8 | web+desktop §4 | 问号弹框收录范围和分节 | 按 §4 合并大纲确认 / 调整 | 按 §4 确认 | 低；需维护防过期 |
| A1 | desktop S1 | 组件暂停/停止/重试 IPC 有实现、无界面 | 做 UI 接上 / 删 preload 与 handler | 删除（一种格式，无人用的通道不留），需要时再加 | 以后要重新加回 |
| A2 | desktop S2 | 远程断开事件无人订阅，用户看不到断开 | web 状态栏加断开提示 / 删除 | web 加提示（这是 UX 缺口） | 不处理则持续静默；做则需小 UI 包 |
| A3 | desktop S5 | `scripts/package_local_cuda.ps1` 个人脚本 | 保留 / 移出 scripts（如 tools/personal）/ 删 | 移出 scripts | 低；有人手动用时路径变化 |
| A4 | desktop C3 | 远程向导第 4 步 Session token | 保留并把 3 句缩成 1 句 / 自动进入跳过此步 | 自动进入，跳过（令牌已自动生成，无需用户操作） | 中；需要令牌的高级用户少一个复制入口 |
| A5 | desktop C8 | SSH 指纹对话框文案 | 保留原文 / 缩短 detail | 保留原文（安全提示宁长勿短） | 缩短可能让用户跳过核验 |

---
## 7 零改动证明

中层合成时复核（2026-10-14 22:5x HKT）：
```
$ git status --short
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
（以及本文件 WP-151-frontend-audit.md）

$ git diff --stat -- web desktop scripts
（空）

$ sha256sum
ce6e7d8595a1dfc4efc492268b2deeecb042a855f97bb39cfb1abafe10e4e115  desktop/package.json
1c551b1dc125921c7b9aa6b83665a888741b248e8eff8167579c5854591c8ae7  desktop/package-lock.json
9ff2a4c3a03c66cfe9ae6bcfc408e8583a8546b48f5605a9e2e23ffc5f232e2f  web/package-lock.json
31121350dd7d9f4d7bf0747dac6fb4baee723caa53a4660f39664fdb52256bcf  web/package.json
```
- desktop 三个哈希与 WP-153 开工前（`/tmp/wp153-before.txt`）一致。
- 与开工前相比，只新增了三份 WP-151 产物；`.litecode/ops` 下的 M 项是看板与事件日志，开工前就存在，不是本包写的。
- 产品代码、package.json、锁文件零改动；未 commit。

---
## 附：后续瘦身包建议顺序（按风险从低到高）

1. 低风险删改：W1–W21、D1–D7、W-S10–12，以及 §3 中标「低」的文案。
2. 依赖声明：§1.3 补 dockview / @tanstack/virtual-core；W24 删 `@types/opentype.js`；U1 定后处理 remotion。
3. 跨端契约：X3 + X4 → X5 → X2。
4. 中风险重构：W-R1–W-R5、R5、R7。
5. 待 Andy 定的各项，按裁定结果分包。
