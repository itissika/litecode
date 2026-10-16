# WP-154/157 启动工作清单：desktop 壳 + web 前端（frontend_mid）
这份清单只靠静态阅读（rg/Read）整理，没有构建、没有运行。行号基于 HEAD 3ddc643。按 GM 新方向取消计时，不补测。desktop 的耗时一律写「待真机测」。
范围：从打开工作区开始，到窗口可见、输入框可打字、各面板就绪为止。
阶段口径：P0 = 打开前必须做；P1 = 打开后立即做；P2 = 可推迟或按需做。

## A. 建议阶段与现状不同的项
- D5 createWindow / show：现在排在 READY 和 /health 之后（打开前阻塞），建议提到 P0 起点，先显示骨架或加载页，与 sidecar 启动并行。
- D3 等 LITECODE_READY 和 D4 /health 轮询：现在阻塞窗口创建，建议改为只阻塞 loadURL（窗口先出来），P1。
- W1 同步 import `./lib/monaco`：现在在首屏 bundle（P0），建议改为 P2，打开编辑器时再 lazy import。
- W1b 7 份字体 CSS：现在全部同步引入，建议只保留 Inter 400 和 JetBrains Mono 400 两份作为 P0，其余字重 P2。
- W9 auth 帧：现在 onopen 后立即发出（P1），而 serve 回 -32601，建议删除（WP-159 已在删）。
- W14 GET /api/workspace/engines：现在 hello 后立即发（P1），建议 P2，打开 Engines 面板或状态栏需要时再取。
- W15 GET /api/settings summary：现在 hello 后立即发（P1），建议 P2，设置面板或依赖 summary 的组件挂载时再取。
- W19 文件树 loadRoot：现在随 FileTree 挂载立即 HTTP 取根（P1），保持 P1，但应排在 session/new 之后，避免在大仓上抢 serve。属于顺序调整，阶段不变。
- W17（恢复布局路径）面板 writable 依赖 session/list：现在要等 list 回复，而 list 被 replay 拖到约 240–280ms，建议让布局里的 root 面板按持久化的 sessionKind 先可写（P1 立即），list 回来后再校正。

## B. 必须留在 P0、且代码显示偏慢的项
- D3+D4：壳先等 sidecar 的 stdout 打出 `LITECODE_READY`（超时 90s，sidecar.ts:115），再每 250ms 轮询 /health，最长 15s（main.ts:619-650）。两者全部完成后才 createWindow（main.ts:1369-1384）。serve 启动本身必须在 P0，但「窗口等它」不是必须的，见 A。
- W12→W13（全新 profile 路径）：输入框要等 `server/hello`（agentWs.ts:435）之后，再等 `session/new` 回复（centreChat.ts:23-28 → sessionStore.ts:591-597）。两段串行 RTT，实测 session 约 60ms 解锁。可以考虑 hello 里预分配 root session，把两段并成一段。
- W1 首屏 bundle：monaco 加 7 份字体同步加载，会推迟 FCP 和 T0（main.tsx:1-16）。其中 bundle 解析属于 P0，但 monaco 本身可以移出（见 A）。

## C. 清单
| # | 工作项 | 代码位置 | 它依赖什么 | 谁依赖它 | 现在所在阶段 | 建议阶段（P0/P1/P2）| 一句话理由 | 已测耗时 |
|---|---|---|---|---|---|---|---|---|
| D1 | app.whenReady → boot() | desktop/src/main.ts:1825 | Electron 启动 | D2–D7 | 打开前阻塞 | P0 | 一切的起点。 | 待真机测 |
| D2 | createToken + resolveBootWorkspace；无工作区时走 hub | main.ts:1365-1367,1379-1380 | D1 | D3 | 打开前阻塞 | P0 | 计算量很小，sidecar 需要 token 和 cwd。 | 待真机测 |
| D3 | startSidecar：spawn serve（cwd=工作区，LITECODE_TOKEN），扫 stdout 等 `LITECODE_READY`，超时 90s，不轮询 | main.ts:1369-1373; sidecar.ts:36,107-116,166-190 | D2；serve 冷启动（大仓有索引） | D4 | 打开前阻塞（阻塞窗口） | P0 启动 / 只阻塞 D6 | serve 必须先起来，但窗口不必等它。 | 待真机测 |
| D4 | verifySidecarAttached：每 250ms GET /health，截止 15s，校验 ok、workspace_id、root | main.ts:616-660,1374 | D3 | D5 | 打开前阻塞（阻塞窗口） | P1（与窗口并行） | 只是确认 attach，可以与骨架窗口并行。 | 待真机测 |
| D5 | attachSidecarWatch、recordRecent、localWorkbenchUrl | main.ts:1375-1378 | D4 | D6 | 打开前阻塞 | P1（recents 为 P2） | recents 写盘与首屏无关。 | 待真机测 |
| D6 | createWindow → loadURL(handshake ?token=) → ready-to-show 时 show | main.ts:1384,310-335,292-307 | D3–D5（现状串行） | W1 起所有 web 项 | 打开前阻塞（最后一步） | P0（提前到 D2 之后） | 窗口早出现可显示骨架，体感最大的收益。 | 待真机测 |
| D7 | InstanceRegistry.start、setWorkspace | main.ts:1385-1389 | D6 | 多实例 | 窗口后 | P1 | 不影响首屏。 | 待真机测 |
| D8 | remote 模式：waitForWorkspaceHealth(remote) 后 loadURL | main.ts:1355-1360 | LITECODE_TOKEN，远端 | D6 | 打开前阻塞 | P1（窗口先建） | 远端 RTT 不该让窗口白等。 | 待真机测 |
| D9 | preload：sendSync get-auth-token、get-session-mode | desktop/src/preload.ts:75-90 | D6 | W8（token） | web 脚本执行时同步 | P0 | WS 需要 token；同步 IPC 很小。 | 待真机测 |
| D10 | preload：sendSync load-layout | preload.ts:92；web lifecycle.ts:66-71 | D6 | W5 | before input（布局恢复） | P0 | 恢复布局必须读到它。 | 待真机测 |
| D11 | preload：sendSync get-ui-theme（桌面主题优先于 localStorage） | preload.ts:238; web lib/theme.ts:29-30 | D6 | W3 | 首次渲染前 | P0 | 防止闪色。 | 待真机测 |
| D12 | sidecar 意外退出 → 重启 → loadLocalWorkbench / reload | main.ts:786-802,1215-1229 | 运行期 | — | 异常路径 | 不在启动路径 | 只列出备查。 | — |
| W1 | 首屏 bundle 同步 import：dockview.css、theme/index.css、`./lib/monaco` | web/src/main.tsx:8-9,15 | D6 加载 index.html | W2 | 首次渲染前（P0） | dockview/theme P0；monaco P2 | 首屏没有编辑器时 monaco 是纯负担。 | 未测 |
| W1b | 字体 CSS ×7（Inter 200/300/400/500，JetBrains Mono 300/400/500） | main.tsx:1-7 | 同上 | 渲染 | 首次渲染前 | 400 两份 P0，其余 P2 | 其余字重首屏用不到。 | 未测 |
| W2 | applyTheme(getTheme()) | main.tsx:19; lib/theme.ts:29 | D11 / localStorage | W3 | 首次渲染前 | P0 | 防止闪色。 | 未测 |
| W3 | createRoot.render(<App/>)（StrictMode） | main.tsx:21-25 | W1, W2 | W4–W6 | 首次渲染 | P0 | — | 未测 |
| W4 | installDebugConsole | App.tsx:15 (useEffect) | W3 | 调试 | 渲染后 effect | P2 | 与首屏无关。 | 未测 |
| W5 | AppShellDockview onReady → startWorkbench：读布局，schemaVersion 匹配才 fromJSON（含 popout 等待）→ finishChromeAndSettle → noteLayoutSettled | dockview/hooks/useDockviewConfig.ts:19-21; dockview/workbench/lifecycle.ts:36-49,66-71,128-175; lib/centreChat.ts:31 | W3, D10 | W12（ready 标志）、各面板挂载 | before input | P0（popout 重开 P1） | 中心区面板必须先定，popout 窗口可以稍后再开。 | 未测 |
| W6 | installEmbedInstallWatcher：只挂宿主事件监听，事件到达后才轮询 | App.tsx:17; dockview/panels/settings/engines/embedInstall.ts:25-36,52 | W3 | Engines 刷新 | 渲染后 effect（启动时无 HTTP，静态判断） | P2 | 只在装 embed 模型后才有用。 | 未测（原计划实测确认无 HTTP） |
| W7 | connectionStore.init：getAuthToken（preload / ?token= / env） | App.tsx:16; stores/connectionStore.ts:150-153; api/auth.ts:213-225 | W3, D9 | W8 | 渲染后 effect | P0 | WS 的前置。 | 未测 |
| W8 | new WebSocket(/ws)，同时启动 2000ms 握手超时（500→8000ms 退避重连） | api/agentWs.ts:26,114-131,327 | W7 | W9–W12 | before input | P0 | 首个对 serve 的请求（T0）。 | 未测 |
| W9 | onopen 时发 `auth {token}`，fire-and-forget；serve 回 -32601 | agentWs.ts:135-141 | W8 | 无 | after open | 删除（WP-159） | 服务端不认这个帧，不在关键路径上。 | 未测 |
| W10 | onopen → state="connected"（早于 hello，D1 禁用条件在此解除） | agentWs.ts:139; connectionStore.ts:157-160 | W8 | 发送按钮、W11 | before input | P0 | — | 未测 |
| W10a | knowledge onReconnect（未 hydrated 时跳过） | connectionStore.ts:161; stores/knowledgeStore.ts:642-645 | W10 | 知识面板 | after open | P2 | 首开时本来就跳过。 | 未测 |
| W11 | `session/list` RPC（单飞合并） | connectionStore.ts:164 → stores/sessionStore.ts:658-680 | W10 | W17（恢复路径的 writable）、会话列表 | after open（恢复路径下 before input） | P1 | 恢复路径的关键依赖，回复被 replay 拖慢。 | 约 240–280ms（被 replay 拖慢） |
| W11a | telemetry onConnectionChange | connectionStore.ts:172 | W10 | 遥测 | after open | P2 | — | 未测 |
| W12 | ← `server/hello` → handshakeComplete；setRevision；写入 project/version/workspaceId/hostOs | agentWs.ts:435-438; connectionStore.ts:255-279 | W8 | W13–W16 | before input | P0 | 全新 profile 的解锁前提。 | 未测 |
| W12a | 模型列表：onHello → availableModels = hello.models；后续 models/changed 推送 | sessionStore.ts:360-368; connectionStore.ts:281-285 | W12 | ModelSwitcher、hasModel（可发送） | before send | P1 | 只挡发送，不挡打字。 | 未测 |
| W13 | noteTransportReady → 中心区为空则 `session/new` → openSessionPanel(root) | stores/settingsStore.ts:420-423; lib/centreChat.ts:23-28,41-44; sessionStore.ts:585-597 | W12 + W5 | W17（全新 profile 的输入框） | before input | P0 | 全新 profile 下输入框要等它。 | 约 60ms 解锁 |
| W14 | GET /api/workspace/engines（engineStore.ensureLoaded） | connectionStore.ts:264; stores/engineStore.ts:108-110; api/workspace.ts:581-582 | W12 | 引擎/索引状态 UI | after open（void 并发） | P2 | 只在 Engines 视图或状态处使用。 | 未测 |
| W15 | GET /api/settings summary | settingsStore.ts:424-430; api/settings.ts:327 | W12 | 设置面板 | after open（void 并发） | P2 | 失败也不挡中心区（代码注释已经这样写）。 | 未测 |
| W16 | desktop notifyWorkspace IPC | connectionStore.ts:277 | W12 | 壳标题/recents | after open | P2 | 不影响输入。 | 待真机测 |
| W17 | AgentPanel 挂载：writable = !knownChild && (explicitRootIntent \|\| session 在 list 里)；否则只读分支 | dockview/panels/AgentPanel.tsx:166-169,184,634 | W13（全新）或 W11（恢复） | W18 | before input | P0（全新）/ 改为 P1 即时（恢复） | 恢复路径不必等 list，可以先按持久化的意图可写。 | 见 W11 / W13 |
| W17a | session/subscribe（connState=connected 时由 effect 触发，随后有 replay） | AgentPanel.tsx:111-117; connectionStore.ts:470-476; turnStore.ts:539,575,593 | W10、sessionId | replaying 标志（挡发送） | after open | P1 | 历史回放是必要的，但只挡发送。 | 未测 |
| W18 | AgentChatInput / MentionEditor 渲染；编辑器本身从不 disabled。connBlocked 只禁用发送和 ModelSwitcher | components/AgentChatInput.tsx:424-429,737-750,808-810 | W17 | 用户 | 可打字 = T_int | P0 | 终点。 | 见 W13 |
| W18a | 首次 focus 时 knowledge load | AgentChatInput.tsx:751-756 | 用户 focus | @mention | lazy | P2 | 已经是按需加载。 | 未测 |
| W19 | 文件树 loadRoot → HTTP fetchTree(path, depth=1) | components/FileTree.tsx:418,469-470; stores/treeStore.ts:133-146; api/workspace.ts:52 | FileTree 挂载（W5） | 文件树面板 | after open（挂载即发） | P1 | 面板就绪需要它，但应排在 W13 之后。 | 未测 |
| W20 | LSP：编辑器打开文件时发 lsp/request；收 lsp/diagnostics 推送 | lib/litecodeLsp.ts:185,258,275; connectionStore.ts（dispatchEnvelope 的 lsp 分流） | 打开编辑器 | 诊断/补全 | lazy（按需） | P2 | 启动路径上没有主动初始化。 | 未测 |
| W21 | lazy import：Milkdown、PdfPreview（pdfjs）、mermaid、knowledgeStore 里的 tree/editor store | components/EditorPane.tsx:50-56; components/fileview/PdfPreview.tsx:41; lib/mermaid.ts:24; stores/knowledgeStore.ts:327-343 | 按需 | 对应视图 | lazy | P2 | 已经是按需，保持不变。 | 未测 |
| W22 | i18n | 无（web/src 里没有 i18n 初始化，字符串硬编码，见 WP-151） | — | — | 不存在 | — | 启动路径上没有这一项。 | — |

## D. 头号建议
1. 窗口先建：createWindow 和 show 提到 sidecar spawn 之前或与之并行，READY 加 /health 只阻塞 loadURL（main.ts:1369-1384）。
2. 恢复布局时 root 面板不再等 session/list 才可写，避开 replay 造成的 240–280ms（AgentPanel.tsx:169）。
3. 全新 profile 下把 hello 和 session/new 的两段串行 RTT 合成一段（例如 hello 预分配 root session，或连上后立即发 session/new）（centreChat.ts:23-28）。
4. monaco 从首屏 bundle 移除改为 lazy，字体只首载 400 字重（main.tsx:1-15）。
5. engines、settings summary 和 auth 帧移出启动路径：前两者改为 P2 按需取，auth 帧删除（WP-159）；文件树 loadRoot 排在 session/new 之后。
