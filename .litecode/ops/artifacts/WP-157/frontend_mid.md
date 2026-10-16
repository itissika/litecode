# WP-157 前端握手→可交互（frontend_mid）— 静态部分 + 测量方法
状态：静态梳理完成；探针就绪；**实测未跑**（等后端中层排测量窗口）。所有行号基于 HEAD 3ddc643。

## 1. web 首屏链路（main.tsx → 输入框）
| # | 步骤 | 位置 |
|---|---|---|
| W1 | 同步 import 字体 CSS ×7、dockview.css、theme；`./lib/monaco` 同步导入（首屏 bundle 体积疑点） | web/src/main.tsx:1-16 |
| W2 | applyTheme → createRoot.render(<App/>) | main.tsx:19-25 |
| W3 | App useEffect: installDebugConsole → **connectionStore.init()** → embedInstall watcher | App.tsx:14-17 |
| W4 | AppShellDockview onReady → startWorkbench：恢复 localStorage 布局（schemaVersion 匹配才 fromJSON，含 popout 等待）→ finishChromeAndSettle → noteLayoutSettled | dockview/hooks/useDockviewConfig.ts:19-21; dockview/workbench/lifecycle.ts:128-175; lib/centreChat.ts:31 |
| W5 | 中心区无面板 且 已收 hello → newSession()（每页一次） | lib/centreChat.ts:23-28,41-44 |
| W6 | `session/new` RPC → openSessionPanel(sessionKind:"root") | stores/sessionStore.ts:591-597 |
| W7 | AgentPanel 挂载；writable 判定 → 渲染 AgentChatInput | dockview/panels/AgentPanel.tsx:169,184,634 |
| W8 | MentionEditor（aria "Message the agent"） | components/AgentChatInput.tsx:737-750 |

### 输入框隐藏 / 禁用条件（全部）
- **隐藏 H1**：中心区没有 agent 面板。全新浏览器 profile（无布局）时，面板要等 `server/hello` **且** `session/new` 回复后才出现（centreChat.ts:24 + sessionStore.ts:591-597）。
- **隐藏 H2**：恢复的布局里面板 `writable = !knownChild && (explicitRootIntent || session!==undefined)`（AgentPanel.tsx:169）。恢复的旧 `agent-*` 面板一般没有 sessionKind:"root"（AgentPanel.tsx:99），所以要等 **`session/list` 回复**里出现这个会话，否则走只读分支（AgentPanel.tsx:184）。
- **禁用 D1** `connBlocked = connection!=="connected" || isRunning || compacting || replaying`（AgentChatInput.tsx:427-428）。
- **禁用 D2** `isBlocked = connBlocked || !hasModel`，其中 `hasModel = sessionModelId || availableModels.length>0`。availableModels 来自 hello.models（sessionStore.ts:360-368）。这个条件挡的是发送，不挡输入（AgentChatInput.tsx:426,429,571,810）。
- 注意：MentionEditor 本身没有 disabled 属性。connBlocked 只禁用 ModelSwitcher 和发送等按钮（:589-677,808-810），可以在 connected 之前打字。所以 T_int 的「可打字」实际上由 H1/H2 决定，「可发送」由 D1/D2 决定。探针会同时记录这两个时刻（见 §5 待补）。
- **`state==="connected"` 在 ws.onopen 时就会置位，早于 server/hello**（agentWs.ts:135-139）。因此 D1 在 hello 之前就会解除。

## 2. 到 serve 的连接与首批请求（顺序 + await 链）
**WS 之前的 HTTP：无。** init 前没有 fetch，token 取自 preload、?token= 或 env（api/auth.ts:213-225），WS 是首个对 serve 的请求，**T0 = new WebSocket(/ws)**（agentWs.ts:131，URL 由 :327 buildConnectUrl 拼出）。静态资源（index.html/JS/CSS/字体）由 serve ServeDir 提供（src/serve/router.rs:75），记在 T0 之前。embedInstall watcher（App.tsx:17）是否发 HTTP 尚未核实，探针会捕获。

| 序 | 方向 | 消息/请求 | 触发点 | 并发/串行 |
|---|---|---|---|---|
| 0 | → | WS upgrade /ws；同时启动 2000ms hello 超时，超时后按 500→8000ms 退避重连 | agentWs.ts:114-131, 26, 87-88 | — |
| 1 | onopen | setConnectionState("connected") 同步回调： | agentWs.ts:139 → connectionStore.ts:157-172 | |
| 1a | → | knowledge refresh（仅 hydrated 时，首开跳过） | connectionStore.ts:161; knowledgeStore.ts:642-645 | 不等 |
| 1b | → | **`session/list` RPC** | connectionStore.ts:164 → sessionStore.ts:658-680 | fire-and-forget，单飞合并 |
| 1c | → | **`session/subscribe`**（仅恢复的布局里已有 AgentPanel 时；由 connState 变化触发的 effect） | AgentPanel.tsx:111-117 → connectionStore.ts:470-476 | 与 1b 并发 |
| 2 | → | `auth {token}`（仅在有 token 时），**排在 1b/1c 之后**发出 | agentWs.ts:140-141 | ⚠ 顺序疑点：session/list 会先于 auth 上线，需与 Session 核对服务端是否会拒绝或排队 |
| 3 | ← | **`server/hello`** 通知 → handshakeComplete | agentWs.ts:435-438 → connectionStore.ts:255-279 | |
| 3a | HTTP | `GET /api/workspace/engines` (engineStore.ensureLoaded) | connectionStore.ts:264 → engineStore.ts:108-110 → api/workspace.ts:581-582 | void，并发 |
| 3b | — | noteTransportReady → 若中心为空则 **`session/new` RPC** | settingsStore.ts:420-423 → centreChat.ts:41 → sessionStore.ts:591 | 与 3a/3c 并发；**面板依赖其回复** |
| 3c | HTTP | `GET /api/settings`（settings summary） | settingsStore.ts:425 → api/settings.ts:327 | void，并发 |
| 3d | IPC | desktop notifyWorkspace | connectionStore.ts:277 | 不等 |
| 4 | → | session/new 回复 → openSessionPanel → AgentPanel 挂载 → `session/subscribe` | sessionStore.ts:594-597; AgentPanel.tsx:111-117 | 串行于 3b |
| 5 | 按需 | knowledge load 在 composer **首次 focus** 时触发 | AgentChatInput.tsx:751-756 | P2，不挡 |

**前端解锁输入框所等待的后端消息（要求 a）：**
- 全新 profile：`server/hello`（agentWs.ts:435, connectionStore.ts:265）→ **`session/new` 的回复**（sessionStore.ts:591-597）。这是两段串行的 RTT，不等 SessionSnapshot，也不等 session/list。
- 恢复布局：**`session/list` 的回复**（connectionStore.ts:164 → AgentPanel.tsx:166-169）。在它到达前面板是只读分支，没有输入框。
- 可发送还额外依赖 `server/hello.models`（hasModel，AgentChatInput.tsx:426），以及 subscribe 后的 replay 结束（replaying，turnStore.ts:539/575/593）。
- 不在关键路径上：/api/settings、/api/workspace/engines、knowledge。

## 3. 桌面壳（只静态，耗时全部「待真机测」）
| 步骤 | 位置 | 等待点 |
|---|---|---|
| app.whenReady → boot() | desktop/src/main.ts:1825-1832 | 待真机测 |
| local：createToken → resolveBootWorkspace → **startSidecar** spawn(serve, cwd=workspace, LITECODE_TOKEN) | main.ts:1365-1374; sidecar.ts:107-113 | 待真机测 |
| waitForReady：扫描 stdout 中的 `LITECODE_READY http://127.0.0.1:N/`，**超时 90s**，没有轮询 | sidecar.ts:36,115-116,166-190; 后端打印 src/serve/router.rs:255 | 待真机测 |
| verifySidecarAttached → GET /health **每 250ms 轮询，截止 15s**，要求 ok、workspace_id、root 匹配 | main.ts:616-660 | 待真机测 |
| 以上全部完成后才 createWindow → loadURL(handshake ?token=) → ready-to-show 时 show | main.ts:1384, 310-335, 292-307 | **窗口创建与 serve 启动串行**（疑点：可以先建窗口显示骨架） |
| remote 模式：waitForWorkspaceHealth(remote) 后 loadURL | main.ts:1355-1360 | 待真机测 |
| sidecar 意外退出后重启 → loadLocalWorkbench（同文档时 reload） | main.ts:786-802, 1215-1229 | — |
壳的 T0 前耗时 = spawn→READY + READY→health ok + createWindow→web T0。

## 4. 关键路径疑点（按 Andy 五级，待实测确认）
1. 【疑似高】全新 profile 下输入框要等 hello→session/new 两段串行 RTT 才出现（centreChat.ts:24-27）。session/new 慢的话（如等 ide/engine），首个可交互时刻会被直接拖后。
2. 【疑似高】恢复布局时面板 writable 依赖 session/list 回复（AgentPanel.tsx:169）。大仓 session 多时 list 的耗时直接进关键路径。
3. 【疑似中】桌面窗口等 READY 和 /health 之后才创建（main.ts:1369-1384），壳启动全程白等。
4. 【疑似中】session/list、subscribe 先于 auth 帧发出（agentWs.ts:139-141），需 Session 侧确认是否会被拒后重试。
5. 【疑似低】main.tsx 同步引入 monaco 和 7 份字体 CSS，会推迟 FCP 和 T0（main.tsx:1-16）。

## 5. 测量方法（可复跑）
- 探针目录：`.tmp/wp154/fe/`。playwright@1 和 chromium-headless-shell 只装在这里（PLAYWRIGHT_BROWSERS_PATH=.tmp/wp154/fe/browsers），共约 700MB，测完 `rm -rf .tmp/wp154/fe`。
- `probe.mjs`：记录 navigationStart（timeOrigin）、FCP、T0（首个 /ws 或 /api 请求）、每个 HTTP/WS 请求相对 T0 的 start/end/dur、每个 WS 帧的 method、方向和时刻，以及 T_int（`[aria-label="Message the agent"]` 可 focus，且按下 x 后能回显）。serve 启动时刻经 SERVE_START_JSON 传入，含 wall_ms 和单调 mono_ms，可用于对齐 IDE/Session 日志。
- `run.sh <cold-small|cold-big|warm> [N=3]`，`median.mjs` 计算中位数。启动方式：`cd <ws> && LITECODE_WEB_DIST=/workspace/litecode/web/dist target/debug/litecode serve --bind 127.0.0.1:7591 --loopback-only`，等 stdout 出现 LITECODE_READY 后打开 `http://127.0.0.1:7591/`。**不用 vite preview**，因为 serve 自己托管 web/dist（router.rs:75; web_dist.rs:7-11）。
  - cold-small：每次新建 serve，删掉小仓的 .litecode，用新 context。
  - cold-big：每次新建 serve 指向 /workspace/litecode，用新 context。
  - warm：同一个 serve，先 prime 一次，再用持久化 profile 新开页面（有布局，走 H2 路径）。
- 原始数据在 `raw/<group>/run-N.json`、`serve-N.log`，中位数在 `median.json`。
- 待补：「可发送」时刻（发送按钮 enabled）在实测前加进探针。

## 6. 构建需求
- `web/dist` **不存在**，需要 `cd web && /exec-daemon/node node_modules/typescript/bin/tsc -b && /exec-daemon/node node_modules/vite/bin/vite.js build`（等同 npm run build），估计峰值 1.5–2.5GiB，几分钟。
- `target/debug/litecode` 存在（10:55 构建），**早于 HEAD 3ddc643（15:59）**。是否重编 serve 由后端中层决定（cargo build -j2，需 MemAvailable ≥6GiB；当前约 5.9GiB，略低于门槛）。
- 测量窗口估计：build web 约 5min，cold-small 3 次 + cold-big 3 次 + warm 1+3 次约 5–10min（取决于大仓 serve 启动），合计约 **15–20 分钟**，期间独占 serve 和大仓。
