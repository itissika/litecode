# WP-158 web B3 交付物（WP-163）

## 批复记录（前端中层，2026-10-14 23:28）
- WP-163 计划已批；等 B2 done 后开工。
- Q1：A2 的视觉方案用现有 token 自定（状态栏用 danger 或 muted 的色值），不加文字。交审时附截图或样式 diff。
- Q2：断开态两个来源都接：
  - WS 连接状态：断开或重连中时显示，重连成功后恢复；
  - desktop 的远程断开事件：按 WP-161 契约订阅，不自造事件。
- Q3：W-R10 可以跳过，但要在交付物里写明理由。
- 护栏同 B1：重型任务前后都要报中层，由后端中层放行；重型任务不并行。

## 裁定（来自 WP-162 验证批复）
- 断连态：收到 remote-disconnected 时置位，重连后整窗 reload 时清除。不自制重连检测，不加恢复事件。

## 实施记录（WP-163，2026-10-15）
已 `wp.mjs start`。在主树里改，保留了 WP-159/162 的已有改动。没跑 tsc、vitest、knip，也没 commit。desktop 没碰（包括 hub.ts）；原计划里本来就没有 hub.ts 的改动，所以没有要删的。
只做过一次轻量检查：用 TypeScript 的 transpileModule 逐文件解析 31 个改动或新增的文件，语法诊断为 0。这不是类型检查，类型问题要等 tsc。

### 各组合并结果（按风险从低到高）
| 组 | 结果 | 落点 | 说明 |
|---|---|---|---|
| W-R7 | 做了 | `components/toolviews/FilePathHeader.tsx` | 三个视图共用文件路径头（图标、路径、占位文字、颜色、infoicon）；Edit 的“N edits”通过 children 传入。DOM 和 class 不变。 |
| W-R8 | 做了 | `components/compactMarkdown.tsx`（`compactMarkdownComponents`） | 共用 strong、em、code、a 四个映射；p、ul、ol、li、pre 仍由各处自己定义（两处的 p 间距本来不同：mt-1 和 mt-1.5）。 |
| W-R9 | 做了，**换了落点** | `lib/useVerticalSplit.ts` | 实际核对代码后，两个面板的折叠标题并不重复（Git 用 FoldCard，Search 用 SearchSection）。审计所指行段（GitPanel 495-560、SearchPanel 39-45 和 458-486）真正重复的是上下分栏：readSplit、拖动分隔条、存 localStorage。因此抽成了 hook，两边各自的 key 和默认值不变。见疑问 Q1。 |
| W-R6 | 做了 | `ui/Select.tsx` 导出 `SELECT_TRIGGER_CLASS` 和 `SelectChevron` | ModelSwitcher 删掉了自己的 Chevron 副本；它的 FIELD_TRIGGER 改为 `SELECT_TRIGGER_CLASS` 加原有的 `min-w-0 disabled:cursor-not-allowed`，最终 class 集合不变。 |
| W-R10 | **跳过** | — | 理由：(1) 审计已标“收益小”；(2) 两对组件（Image/MediaPreview、KnowledgeGraphPanel/KnowledgePanel）重复的只是外壳几行，各自的加载、错误和交互逻辑不同，抽共享外壳得加不少参数，代码不会明显变少；(3) 按批复 Q3 允许跳过，把风险预算留给 W-R5、W-R2、W-R3。 |
| W-R4 | 做了 | `dockview/tabs/BaseTab.tsx`（BaseTab、TabIcon、TabTitle） | 四个标签共用外壳和关闭按钮（pointerdown 阻止激活、click 关闭）。唯一差异：KnowledgeGraphTab 原来根节点没有 `group` 类，现在有了；它的子元素没有用 group-hover，所以没有可见变化。 |
| W-R1 + W-C6 | 做了 | `settings/ScopedDefinitionEditor.tsx`（含 `ScopeHint`） | 两节共用页面外壳、Create 按钮、+ 菜单、Global/Workspace 分组、行（名称、摘要、删除）和 (new) 卡；各节只传自己的 editorForm、摘要和打开回调（MCP 额外清 probe）。**W-C6**：两节共用 ScopeHint，Custom Tools 文案从“Bind on Agents. Workspace overrides global.”改为“Workspace overrides global. Bind on Agents.”，与 MCP 同口径；MCP 文案不变。这是计划里唯一的文案变化。 |
| W-R5 | 做了 | `ui/useFloatingLayer.ts` | 共用的是：打开时监听 scroll（capture）和 resize 来重新定位，按 Esc 或在外部 mousedown 时关闭。Popover 在 hover 模式下滚动即关闭的逻辑通过 onScroll 传入；Dropdown 的 dock pane ResizeObserver 留在 Dropdown 自己的 effect 里。处理函数改为通过 ref 读取最新版本，监听只在 open 变化时重挂（原来 Dropdown 会随 direction 等 props 重挂），行为等价。window 统一用 `viewOf`；它和 Dropdown 原来写法的唯一区别是窗口已关闭时回退到主 window。 |
| W-R2 | 做了 | `dockview/panels/useSessionPanelBootstrap.ts` | 抽出的公共逻辑：active 跟踪、每次 connected 时重新订阅、“session not found”时弹 toast 并关闭、真正卸载时 release，以及 session 和 preview 的选择器。两个面板各自保留设标题的逻辑（截短 id 的规则不同）。 |
| W-R3 | 做了，**部分合并** | `components/transcript/TranscriptBody.tsx` | 两边的 DOM 布局差异很大：可写版有 user rail、mini-chat 编辑锚点、外层 frame 不同；只读版有 px-4 和 prose 宽度。若硬合成一个带 `readOnly` 的组件，DOM 要分叉，界面不变就很难保证。所以只合并了确实相同的部分：`useTranscriptFeed`（消息、分页、运行态）、`usePendingRevealScroll`（搜索跳转）、`useTopBlur`，以及 `TranscriptVeil`、`TranscriptTopBlur`。各自的 DOM 结构原样保留，虚拟列表的 scrollRef 不变。见疑问 Q2。 |

### A2：远程断开态
- 实现：StatusBar 挂载时调用 `window.litecode?.onRemoteDisconnected(...)`。window 的类型来自 auth.ts 的 `import type`，也就是 desktop 的 bridge-contract，没有引通道常量。收到事件后在 `.status-bar-root` 上加 `data-disconnected` 属性。组件里没有任何清除逻辑，只靠整窗 reload 重新挂载来恢复。没做重连检测，没加恢复事件，没加文字。
  - 与早先批复 Q2 不同的地方：Q2 说“WS 断开或重连中也显示”，现在按 WP-162 的裁定和本次指示，只接 desktop 事件。
- 样式（只用现有 token）：
```diff
+.status-bar-root[data-disconnected] {
+  border-top-color: var(--_dk-red-500);
+}
+
+.status-bar-root[data-disconnected] .status-bar-strip {
+  color: var(--_dk-text-muted);
+}
```
  - 效果：状态栏顶边线变成 danger 红，状态条文字转为 muted 灰。完整 diff 在 `.tmp/wp163-a2-style.diff`，含 StatusBar.tsx 和 status-bar.css。
- 测试：新增 `components/StatusBar.test.tsx`，共 2 条：
  - 用 stubDesktopBridge 触发事件后，根节点带上 `data-disconnected`，textContent 不变；卸载时会取消订阅；
  - 没有 bridge 时正常渲染、没有该属性。

### 改动文件统计
- 新增 9 个文件，共 710 行：FilePathHeader、compactMarkdown、useVerticalSplit、BaseTab、ScopedDefinitionEditor、useFloatingLayer、useSessionPanelBootstrap、TranscriptBody、StatusBar.test。
- 修改 22 个文件（21 个 ts/tsx 加 status-bar.css）。相对 HEAD 是 +239 / −960，但这个数字也包含这些文件里 WP-159/162 的已有改动。
- 没删测试，也没改已有测试的断言。整个 web/ 的 git status：12 个 ??、6 个 D、155 个 M。
- 本轮没改 desktop。

### 疑问（交中层）
- Q1：W-R9 的实际重复是分栏逻辑，不是“折叠分组标题”，所以落点从 `CollapsibleSectionHeader` 改成了 `lib/useVerticalSplit.ts`。这样可以吗？
- Q2：W-R3 只合并了 hook 和外层装饰（遮罩、顶部模糊），没有做单一的 `TranscriptBody readOnly` 组件，因为两边 DOM 差异大，合成一个就保证不了界面不变。这样可以吗？
- Q3：W-C6 让 Custom Tools 的提示语序和 MCP 一致（文字有变化）。如果要求一字不改，可以给 ScopeHint 加参数还原原句。
- Q4：等重负载放行后，按计划串行跑 tsc -b、vitest 全量（--maxWorkers=1）、knip 和 jscpd（前后对比）。建议先单独跑以下相关测试：ToolCallCard、EditToolView、AskUserCard、ToastHost、ModelSwitcher、AgentTab、EdgeTab、CustomTools、McpServers、Dropdown、Popover、AgentPanel、SubagentReadOnlyPanel、MessageList、StatusBar。

## 中层裁定（2026-10-15 02:16）及落实
- Q1：W-R9 的落点定为 `lib/useVerticalSplit.ts`，已认可。
- Q2：W-R3 只做部分合并，已认可。**不做单一 `readOnly` 组件的理由**：
  1. 可写版的 transcript 有 user rail 一列（UserMessageRail，带 layoutRef、notifyRef 和跳转），外层 frame 只留顶部 16px、不留左右边距，内层宽度按 prose 宽度加 rail 宽度计算，遮罩放在 frame 里面。只读版的外层 frame 是 px-4，内层就是 prose 宽度，没有 rail，遮罩和 frame 是平级的兄弟节点。
  2. 可写版的 MessageList 多接 9 个参数：编辑锚点、mini-chat 动画、贴底、jumpToEnd、revealBash、maxFileRevertSeq、composerCollapsed、rail 的两个 ref。只读版只传 `readOnly`。
  3. 硬合成一个组件，内部至少要有 3 处 DOM 分叉加一组可选参数，结果就是把两份布局塞进同一个文件，重复行并不会减少，反而增加了虚拟列表 scrollRef 和层叠顺序回归的风险。所以只合并真正相同的部分：`useTranscriptFeed`、`usePendingRevealScroll`、`useTopBlur`、`TranscriptVeil`、`TranscriptTopBlur`。
- Q3：提示文字一字不改。已给 `ScopeHint` 加 `bindFirst` 参数，Custom Tools 改用 `<ScopeHint bindFirst />`，渲染为“Bind on Agents. Workspace overrides global.”。MCP 仍渲染为“Workspace overrides global. Bind mcp_<id> on Agents.”。
  - 核对结果：两处渲染文本和 **WP-163 开工前的主树**（也就是 WP-159 交付的版本，见 `.tmp/wp159/web.patch` 第 1576 行和第 1637 行）逐字一致。
  - **与 HEAD 不一致**：HEAD 里的原文是“Definitions only. Bind on Agents. Same name: workspace overrides global.”和“Register stdio MCP servers. Same id: workspace overrides global. Bind mcp_<id> on Agents.”。这两句是 WP-159 按 W-C4、W-C6 批准缩短的，属于 B1 的已交付改动，不是 B3 引入的。本次没有回退。见疑问 Q5。
- W-R10 跳过已认可。

### 疑问
- Q5：验收要求“渲染文本与 HEAD 一致”，但 HEAD 是 WP-159 文案缩短之前的版本。现在的 Custom Tools 和 MCP 文案与 B3 开工前（WP-159 后）完全一致，与 HEAD 不一致。请确认口径是“与 WP-159 交付后一致”；如果确实要回到 HEAD 原文，那就是回退 B1 的 W-C4、W-C6，需要另行授权。

## Q5 裁定（2026-10-15 02:18）
- 验收口径为“与 WP-159 交付后一致”；B1 的 W-C4、W-C6 文案不回退。

## WP-163 验证结果（2026-10-15，guard.sh 守护，阈值 4.5GiB，严格串行，全程未触发 kill）
| 步骤 | 命令 | 结果 | 最低 MemAvailable |
|---|---|---|---|
| 1 web tsc -b | `tsc -b` | rc=0，一次通过，没有需要修的 | 4.91GiB |
| 2 web vitest 全量 | `vitest run --maxWorkers=1` | **177 个文件 / 1365 条全部通过**（比 B2 多 1 个文件、2 条，都是新增的 StatusBar.test） | 4.83GiB |
| 3 web knip | `.tmp/wp159-tools/.../knip --reporter json` | 导出 23 / 类型 21 / 文件 1 / devDep 0 / 未声明 0，与 23/21/1/0/0 持平；本包新增的文件和导出一条都没报 | 5.53GiB |
| 4 jscpd 前后对比 | — | **没跑，按规则停下**：本机找不到 jscpd。WP-151 用的 /tmp/wp152-tools 已不存在，.tmp/wp159-tools 里也没有，全盘也搜不到。要装就得 npm install（属于重负载，这次没有放行）。 | — |

- 前值可以用 WP-151 审计记录的数字：jscpd 门槛 50 个 token，不扫测试，重复率 1.84% 行、2.01% token，共 72 处克隆。但那是 WP-159 之前的 HEAD，不是 B3 开工前的状态。
- jscpd 后值还没有，所以**还没有 deliver**。
- 为做“前”快照，曾在 .tmp/wp163-before 导出 HEAD 并试打 WP-159 补丁，没成功，已删掉。主树工作区没有动过。

### git status
- web/：     12 ??       6 D     155 M 
- desktop/：17 项，都是 WP-160/161/164 的改动，本包没碰。
- 没有 commit。

### A2 样式 diff
完整 diff 在 `.tmp/wp163-a2-style.diff`；关键部分如下：
```diff
+.status-bar-root[data-disconnected] {
+  border-top-color: var(--_dk-red-500);
+}
+.status-bar-root[data-disconnected] .status-bar-strip {
+  color: var(--_dk-text-muted);
+}
```
StatusBar.tsx 的改动：订阅 `window.litecode?.onRemoteDisconnected`，把它放进 `remoteDisconnected` 状态，再给根节点加 `data-disconnected`。

### 复跑命令（都在 web/ 下执行）
```
G=/workspace/litecode/.tmp/wp159/guard.sh
$G b3-tsc /exec-daemon/node node_modules/typescript/bin/tsc -b
$G b3-vitest /exec-daemon/node node_modules/vitest/vitest.mjs run --maxWorkers=1
PATH=/exec-daemon:$PATH $G b3-knip /workspace/litecode/.tmp/wp159-tools/node_modules/.bin/knip --reporter json
# jscpd 要等放行安装后再跑（下面是建议做法）：
# PATH=/exec-daemon:$PATH npm i --prefix /workspace/litecode/.tmp/wp163-tools jscpd
# $G b3-jscpd .tmp/wp163-tools/node_modules/.bin/jscpd src --min-tokens 50 --ignore "**/*.test.*,**/test/**" --reporters json --output /workspace/litecode/.tmp/wp163/jscpd-after --silent
```

### 待中层决定
- Q6：能否放行在 .tmp/wp163-tools 装 jscpd（npm install，不改 package.json）？前值打算这样取：用 git archive HEAD 导出代码，打上 WP-159 补丁和 WP-162 的 web diff（WP-162 的 web 改动从现有工作区 diff 中拆不出来，需要按文件挑出来），导出到 .tmp 下单独扫。或者直接用 WP-151 的 1.84%/72 处作前值并注明。请定。

## Q6 裁定与重复核对（2026-10-15 02:24）
**jscpd 没跑，重复改为按 9 组合并清单逐项核对。** jscpd 不在验收范围内，不跑也不装。核对只用了 rg，范围是 web/src，不含测试文件。W-R10 已跳过，不在 9 组之内。

| 组 | 合并到 | 调用处是否都已切过去 | 旧副本是否已删 |
|---|---|---|---|
| W-R7 | `components/toolviews/FilePathHeader.tsx` | 是。FileParamView、WriteToolView、EditToolView 都 import 并使用了它。 | 是。在 toolviews/ 下搜 `truncate font-mono` 路径行，只剩 FilePathHeader 里那一处（SessionStatusLine:898 是另一个组件的命令行，不属于本组）。 |
| W-R8 | `components/compactMarkdown.tsx` | 是。ToastMarkdown、AskUserCard 都展开了 `compactMarkdownComponents`。 | 是。搜 code 样式 `rounded-sm bg-(--_dk-side) px-1 py-px`，只在 compactMarkdown 里出现。 |
| W-R9 | `lib/useVerticalSplit.ts` | 是。GitPanel、SearchPanel 都用了。 | 是。`function readSplit` 和 `onSplitterDown = (` 都只在这个 hook 里出现。 |
| W-R6 | `components/ui/Select.tsx`（`SELECT_TRIGGER_CLASS`、`SelectChevron`） | 是。ModelSwitcher 的 FIELD_TRIGGER 和它的 2 处 Chevron 都已切过去。 | 是。`function Chevron(` 已没有；触发按钮的 class 字符串只在 Select 里出现。 |
| W-R4 | `dockview/tabs/BaseTab.tsx` | 是。AgentTab、EditorTab、KnowledgeGraphTab、BrowserTab 都用了。 | 是。`title="Close"` 只在 BaseTab 里出现。 |
| W-R1 + W-C6 | `dockview/panels/settings/ScopedDefinitionEditor.tsx`（含 `ScopeHint`） | 是。CustomToolsSection、McpServersSection 都用了。 | 是。`Delete ${…}`、`key="__new"` 只在 ScopedDefinitionEditor 里出现；两个 Section 里只剩各自的 newLabel 参数。 |
| W-R5 | `components/ui/useFloatingLayer.ts` | 是。Dropdown、Popover 都用了。 | 是。在 components/ui 下，外部 mousedown 关闭的监听只在这个 hook 里。 |
| W-R2 | `dockview/panels/useSessionPanelBootstrap.ts` | 是。AgentPanel、SubagentReadOnlyPanel 都用了。 | 是。在面板里，`ensureSubscribe(` 和 `releaseSessionTab(` 只在这个 hook 里出现（另外 sessionPanelNav:87 是导航入口，sessionTeardown 是被调用的函数本身，都不属于本组）。 |
| W-R3（部分合并） | `components/transcript/TranscriptBody.tsx` | 是。AgentPanel、SubagentReadOnlyContent 都用了这 5 个导出。 | 是。`scrollTop / 72`、`opacity-[0.33]` 遮罩、顶部 blur 参数、reveal 用的 ensureSeqLoaded 都只在 TranscriptBody 里出现。AgentPanel:352 的 ensureSeqLoaded 是 rail 跳转（onJump），属于可写版独有的逻辑，不是副本。 |

**异常：没有发现本组遗漏。** 只记一条范围外的观察：FileTreeContextMenu:46 和 SessionStatusLine:282 也各自写了外部 mousedown 关闭的逻辑，但不在 W-R5 的范围内（审计只列了 Dropdown 和 Popover），这次没改，也没扩大范围。
