# WP-154 打开工作区：启动工作全清单（汇总）

- 方向（Andy 定）：目标是把启动工作排到合适的阶段，不再追求逐项测清耗时。release 编译、种子重建、W4、C2 等测量全部停止，不补测。
- 来源：`WP-154/backend.md`（IDE 基座，43 行）、`WP-154/session.md`（Session）、`WP-154/frontend.md`（前端中层，desktop 壳 12 行 + web 28 行）。三份均为只读代码整理，位置按 HEAD 3ddc643。
- 阶段口径（`.tmp/wp154/plan.md:5`）：**P0 打开前必须做 / P1 打开后立即做 / P2 可推迟或按需做**。“打开”按 WP-156 的解锁口径：A 路径（新 profile）= session/new 回包，B 路径（恢复布局）= session/list 回包。
- 耗时：只引用 `WP-155-ide.md`（§号）和 `WP-156/session.md` 的已测数字，单位 ms，小仓/大仓 = 93/1,505 个文件；未测写“未测”，desktop 写“待真机测”。

## 一、建议阶段与现状不一致的项（去重后 19 行；按来源计：后端 6、Session 5、前端/壳 9，auth 与 maintain_snapshots 跨域重复）

| 工作项 | 所属域 | 代码位置 | 现阶段 → 建议 | 理由 | 已测耗时 |
|---|---|---|---|---|---|
| D5/D6 createWindow + show 提前 | 壳 | desktop/src/main.ts:1384（现排在 :1369-1378 之后） | 打开前最后一步 → P0 起点，与 sidecar 并行 | 窗口先出骨架，体感收益最大 | 待真机测 |
| D3 等 LITECODE_READY、D4 /health 轮询 | 壳 | main.ts:1369-1374, 616-660；sidecar.ts:107-116 | P0 挡窗口 → 只挡 loadURL（P1 并行） | serve 必须先起，窗口不必等 | 待真机测 |
| 恢复布局时 root 面板可写不等 session/list | 前端 | web/src/dockview/panels/AgentPanel.tsx:169 | 等 list（P0）→ 按持久化 sessionKind 先可写（P1 即时），list 回来再校正 | 避开回放造成的 240–280 | B 解锁 C1 239/281 |
| 先回 list、再做回放（恢复会话 subscribe + Snapshot） | Session | src/client_protocol/connection.rs:891 / :715 → controller/mod.rs:996；串行循环 :1642 | P0 → P1 | 单连接串行循环，list 排在回放之后 | B 路径 list 238–281，A 路径 64 |
| auth 帧（serve 回 -32601） | 前端+Session | web/src/api/agentWs.ts:135-141；connection.rs:1449-1456 | P1 → 删除（前端不发，WP-159） | 鉴权已在 WS 握手完成（ws.rs:52-56） | 已确认回 -32601 |
| warm_snapshot_repo（含 3a/3b git 子进程） | 后端（归属待定，见第四节） | src/config/workspace.rs:513 → src/session/snapshot.rs:1003 | P0 → P1 | 只有首轮 snapshot_track 需要，二者同持 workspace_snapshot_lock | 首次打开约 300–480，之后约 5（§11、§12） |
| EngineManager::reconcile（同步建 webfetch/websearch client） | 后端 | src/serve/run.rs:84 → src/optional/mod.rs:149/230 | P0 → P2 | 只有 web 工具调用需要，首用时再建 | 0.4（§5） |
| sweep_orphan_lsp_processes | 后端 | run.rs:75 → src/proc_lifetime.rs:48 | P0 → P1（后台） | 只清理崩溃遗留，须在首个 language server 启动前完成 | 2.2（§5） |
| maintain_snapshots + list_session_ids_blocking | 后端+Session | src/serve/state.rs:97-99 → snapshot.rs:1258 | P0 → P1（IDE）/ P2（Session），见第四节 | 纯清理，无人等结果；移后台须持 workspace_snapshot_lock | 7 个会话 0.15–0.2 + 0.3–0.4（§5） |
| remove_legacy_text_index | 后端 | run.rs:86 → src/engines/path_index/mod.rs:98/400 | P0 → P2 | 一次性迁移清理 | attach 整体 0.03–0.04（§5） |
| LSP 可用性探测（npm config get prefix、`<server> --version`） | 后端 | src/engines/lsp_engine.rs:48/74 → src/lsp/hub.rs:202 → src/lsp/deps.rs:490/423 | P1 → P2 | 首次需要（首个 language server spawn）时再探，语言通用；版本管理器可能因此做工具链工作 | npm 约 T0−18~−2；--version 约 +372~+565（§10、§13） |
| session index 首个 tick 立即执行 | Session | src/serve/router.rs:32、约 :180-207 | P1 → P2（首 tick 延后） | 与首个连接争资源 | 未测 |
| gc_stale_empty_sessions 启动即跑 | Session | router.rs:172-178 → session/manager.rs:2014 | P1 → P2 | 每小时一次，与打开无关 | 未测 |
| monaco 同步 import | 前端 | web/src/main.tsx:15 | P0 → P2（打开编辑器时 lazy） | 首屏没有编辑器 | 未测 |
| 7 份字体 CSS | 前端 | main.tsx:1-7 | 全部 P0 → Inter/JetBrains Mono 400 两份 P0，其余 P2 | 其余字重首屏用不到 | 未测 |
| GET /api/workspace/engines | 前端 | stores/connectionStore.ts:264；stores/engineStore.ts:108-110 | P1 → P2 | 只有 Engines 视图/状态处用 | 未测 |
| GET /api/settings summary | 前端 | stores/settingsStore.ts:424-430 | P1 → P2 | 失败也不挡中心区 | 未测 |
| 文件树 loadRoot | 前端 | components/FileTree.tsx:418,469-470 | P1 不变，顺序排到 session/new 之后 | 大仓上避免抢 serve（仅顺序调整） | 未测 |
| D5 recordRecent 写盘 | 壳 | main.ts:1377 | P0 → P2 | 与首屏无关 | 待真机测 |

另：Session 的 stale Running 清理（semantic_index.rs:1412）阶段不变（P2），但读 job 状态的入口（:1695）需先识别 stale，属语义提醒，不计入不一致项。

## 二、全清单（按进程阶段）

同一工作项在多份来源中出现时只列一次，域标在工作项后。列序：工作项 | 代码位置 | 依赖什么 | 谁依赖它 | 现阶段 | 建议阶段 | 理由 | 已测耗时/未测。

### 2.1 desktop 壳（前端中层）

| 工作项 | 代码位置 | 依赖 | 被依赖 | 现阶段 | 建议 | 理由 | 耗时 |
|---|---|---|---|---|---|---|---|
| D1 whenReady → boot() | desktop/src/main.ts:1825 | Electron 启动 | D2–D7 | P0 | P0 | 起点 | 待真机测 |
| D2 createToken + resolveBootWorkspace | main.ts:1365-1367,1379-1380 | D1 | D3 | P0 | P0 | sidecar 需 token 和 cwd | 待真机测 |
| D3 startSidecar 等 LITECODE_READY（超时 90s） | main.ts:1369-1373；sidecar.ts:36,107-116,166-190 | D2、serve 冷启动 | D4 | P0（挡窗口） | P0 启动，只挡 D6 的 loadURL | 窗口不必等 | 待真机测 |
| D4 verifySidecarAttached（250ms 轮询 /health，15s） | main.ts:616-660,1374 | D3 | D5 | P0（挡窗口） | P1 并行 | 只是确认 attach | 待真机测 |
| D5 attachSidecarWatch、recordRecent、localWorkbenchUrl | main.ts:1375-1378 | D4 | D6 | P0 | P1（recents P2） | recents 与首屏无关 | 待真机测 |
| D6 createWindow → loadURL → ready-to-show | main.ts:1384,310-335,292-307 | 现状串行等 D3–D5 | 全部 web 项 | P0 最后一步 | P0，提前到 D2 之后 | 先出骨架 | 待真机测 |
| D7 InstanceRegistry.start、setWorkspace | main.ts:1385-1389 | D6 | 多实例 | 窗口后 | P1 | 不影响首屏 | 待真机测 |
| D8 remote 模式 waitForWorkspaceHealth | main.ts:1355-1360 | 远端 | D6 | P0 | P1（窗口先建） | 远端 RTT 不该让窗口白等 | 待真机测 |
| D9 preload 同步取 token、session mode | desktop/src/preload.ts:75-90 | D6 | W7 | P0 | P0 | WS 需要 token | 待真机测 |
| D10 preload load-layout | preload.ts:92；web lifecycle.ts:66-71 | D6 | W5 | P0 | P0 | 恢复布局必需 | 待真机测 |
| D11 preload get-ui-theme | preload.ts:238；web lib/theme.ts:29-30 | D6 | W2 | P0 | P0 | 防闪色 | 待真机测 |
| D12 sidecar 意外退出重启 | main.ts:786-802,1215-1229 | 运行期 | — | 异常路径 | 不在启动路径 | 备查 | — |

### 2.2 serve：spawn → READY（IDE 基座 + Session）

| 工作项 | 代码位置 | 依赖 | 被依赖 | 现阶段 | 建议 | 理由 | 耗时 |
|---|---|---|---|---|---|---|---|
| #1 CLI 解析 | src/cli/cli.rs:701-706 | 进程启动 | 以下全部 | P0 | P0 | 选运行模式 | spawn→log dir 60–82 / 379–566（含 #2–#3，§12） |
| #2 load_runtime_bundle（provider、migrate、配置解析） | cli.rs:733/93 → src/config/manager.rs:131-150 | 全局 litecode.db | 所有配置消费者 | P0 | P0 | 全局依赖 | 后续打开 10–12（§5）；大仓首次 373 基本是 #3 |
| #3 warm_snapshot_repo（3a open_or_init + git config/rev-parse；3b diff-files/ls-files/add/write-tree） | src/config/workspace.rs:513 → src/session/snapshot.rs:1003, :140-193, :563-700 | #4、PATH 上的 git | 首轮 snapshot_track（snapshot.rs:1023） | P0 | **P1** | 只有首轮需要，同锁可后移 | 首次 300–480，之后约 5（§11、§12） |
| #4 load_workspace_state（root、identity、路径、engines readiness） | workspace.rs:491-511 | #2 | #3、#5、#13–#20 | P0 | P0 | 路径和身份先定 | 含在 #2 |
| #5 setup_logging | cli.rs:737 → :115 | #4 | 诊断 | P0 | P0 | 便宜；#2–#4 在其前无日志 | log dir→READY 10–29（§10、§12） |
| #6 bind 校验 + 解析 token（LITECODE_TOKEN） | src/serve/run.rs:56-67；Session 记 run.rs:19-24 | CLI、环境变量 | #23、auth 中间件、ws_handler | P0 | P0 | 安全检查 | 未测 |
| #7 resolve_web_dist | run.rs:70 | 安装目录 | 静态 UI | P0 | P0 | UI 文件须在 | 0.01（§5） |
| #8 sweep_orphan_lsp_processes | run.rs:75 → src/proc_lifetime.rs:48（Linux :215） | lsp_dir | 须先于 #27 | P0 | **P1** | 崩溃清理，后台即可 | 2.2（§5） |
| #9 EngineManager::reconcile | run.rs:84 → src/optional/mod.rs:149/230 | #2 | web 工具 | P0 | **P2** | 首用时建 client | 0.4（§5） |
| #10 WorkspaceEngines::reconcile（起 code_search/lsp 线程） | run.rs:85 → src/engines/mod.rs:773/1105/1172 | #4、engines.json | #24–#33 | P0 | P0 | 只起线程 | 0.1（§5） |
| #11 path_index attach（remove_legacy_text_index + 起线程） | run.rs:86 → engines/path_index/mod.rs:83/98/400 | #4 | #34 | P0 | 起线程 P0；legacy 删除 **P2** | 一次性迁移 | 0.03–0.04（§5） |
| #12 ServeState::new → with_project（合计） | run.rs:91 → src/serve/state.rs:37/64 | #2、#4、#9、#10 | router | P0 | P0 | 见子项 | 3.3 / 3.5（§5） |
| #13 WorkspaceWriteLease::acquire | state.rs:84 | .litecode 目录 | #14 | P0 | P0 | 单写者 | 0.02（§5） |
| #14 SessionData::open（WAL、ensure_session_schema、迁移） | state.rs:90 → session/data/mod.rs:48,62 → data/sqlite/conn.rs:27,54 → data/sqlite/schema.rs:102,264 | #13 | SessionManager、list、subscribe、索引 reader | P0 | P0 | 会话读写前提；见第三节 | 1.8 / 2.0（§5） |
| #15–16 list_session_ids_blocking + maintain_snapshots | state.rs:97-99 → snapshot.rs:1258 | #14、影子仓库 | 无 | P0 | **P1/P2 待 GM 定**（第四节） | 纯清理 | 0.3–0.4 + 0.15–0.2（§5） |
| #17 WorkspaceService::new | state.rs:111 | 项目根 | #21、workspace API | P0 | P0 | 文件 API 与变更总线 | 0.01–0.02（§5） |
| #18 TerminalHub::new + install_hub | state.rs:112-113 | — | #35 | P0 | P0 | 空注册表 | 0.001（§5） |
| #19 IdeBase/Runtime/SessionManager 装配、auto_turn、set_session_reader | state.rs:114-176；src/runtime/mod.rs:83 | #13–#18 | 所有 session RPC、索引 | P0 | P0 | 只做装配，MCP 按需连 | 约 0.7 / 0.8（§10 A10g） |
| #20 tokio runtime 构建 | run.rs:110-112 | — | 全部 async | P0 | P0 | 必需 | 0.1–5.5（§5） |
| #21 restart_watcher（递归 watch 注册） | src/serve/router.rs:112 → src/workspace/watcher.rs:270/30/43 | #17、#20 | 变更总线、#33、#34 | P0 | P0 | 打开后的编辑不能漏；随目录数增长 | 4.0 / 5.0（§5） |
| #22 knowledge-index 线程 | router.rs:115 → src/knowledge/index.rs:1289 | #17 | 知识上下文 | P0 起线程 + P1 构建 | P1 | 不挡首轮 | 未测 |
| #23 bind → 打印 LITECODE_READY → axum::serve | router.rs:252-259 | #20、#21 | D3、前端连接 | P0 | P0 | 就绪信号 | 0.04；READY 早于 T0 2–13（§3）；spawn→READY 小 89/107、大 486/498（C1） |

### 2.3 连接：握手 / hello / list / new / subscribe（Session）

| 工作项 | 代码位置 | 依赖 | 被依赖 | 现阶段 | 建议 | 理由 | 耗时 |
|---|---|---|---|---|---|---|---|
| HTTP auth 中间件 | src/serve/auth.rs:46-76 | token | ws_handler | P0 | P0 | 准入 | 未测 |
| WS 握手 Origin + token | src/client_protocol/ws.rs:32-57 | token | handle_socket | P0 | P0 | 真正的鉴权点 | 未测 |
| runtime.apply(DocId::ALL)（每连接） | ws.rs:124 | 配置文档 | hello | P0 | P0 | 见第三节 | 未测 |
| SessionController::with_turn_guard | ws.rs:130 | runtime、SessionManager | 所有 RPC | P0 | P0 | 连接级控制器 | 未测 |
| 连接附属任务（stats、terminal attach、workspace changes、knowledge、settings） | ws.rs:153,154,约 200,266 | 连接 | 侧栏/状态 | P1 | P1 | 不挡 hello | 未测 |
| LSP diagnostics 订阅（#28） | ws.rs:282-283 | #23 | 前端诊断 | P2 | P2 | 只订阅 | 未测 |
| server/hello 推送 | ws.rs:339-341 → controller/mod.rs:1165/1190 | runtime.apply | 前端初始化 | P0 | P0 | 首帧 | hello 在 T0 后 1–9（WP-155 §2） |
| 读循环 + 串行 session loop | ws.rs:377；connection.rs:1642 | handle_socket | 所有 RPC | P0 | P0 | list 被回放堵的根因 | — |
| auth RPC → -32601 | connection.rs:1449-1456 | 前端发送 | 无 | P1 | 删除 | 白占一次串行处理 | 已确认 |
| session/new | connection.rs:680 → session_lifecycle.rs:322 → manager.rs:540 | SessionManager | A 路径解锁 | P0 | P0 | A 路径打开点 | A 解锁 58 / 61（C1）；热重连约 60 |
| session/list（N+1） | connection.rs:891 → session_lifecycle.rs:410 → data/sqlite/read.rs:235-247 | SessionData | B 路径解锁、侧栏 | P0 | P0 | B 路径打开点；见第三节 | 0 会话 15；18 会话 64；B 路径 238–281 |
| session/subscribe（恢复 + Projection + 回放）+ Snapshot 推送 | connection.rs:715 → controller/mod.rs:996/1107/1151 → manager.rs:714 | SessionManager、SQLite | 历史显示 | P0（B 路径堵 list） | **P1** | 先回 list | B 回包 240/282/269；A 新会话约 105 |
| buffer/load | connection.rs:1391 | subscribe | 编辑区 | P1 | P1 | 按需 | 未测 |
| 热重连 | 同上路径 | serve 已运行 | — | P0 | P0 | — | A 57–65；B 184–191 |

### 2.4 前端首屏（前端中层）

| 工作项 | 代码位置 | 依赖 | 被依赖 | 现阶段 | 建议 | 理由 | 耗时 |
|---|---|---|---|---|---|---|---|
| W1 首屏 bundle：dockview.css、theme、monaco | web/src/main.tsx:8-9,15 | D6 | W2 | P0 | dockview/theme P0；monaco **P2** | 首屏无编辑器 | 未测 |
| W1b 字体 CSS ×7 | main.tsx:1-7 | 同上 | 渲染 | P0 | 400 两份 P0，其余 **P2** | 首屏用不到 | 未测 |
| W2 applyTheme | main.tsx:19；lib/theme.ts:29 | D11 | W3 | P0 | P0 | 防闪色 | 未测 |
| W3 createRoot.render | main.tsx:21-25 | W1、W2 | W4–W7 | P0 | P0 | — | 未测 |
| W4 installDebugConsole | App.tsx:15 | W3 | 调试 | 渲染后 | P2 | 与首屏无关 | 未测 |
| W5 startWorkbench（读布局、fromJSON、settle） | dockview/workbench/lifecycle.ts:36-49,66-71,128-175；lib/centreChat.ts:31 | W3、D10 | W13、各面板 | P0 | P0（popout 重开 P1） | 中心区先定 | 未测 |
| W6 installEmbedInstallWatcher | App.tsx:17；engines/embedInstall.ts:25-36 | W3 | Engines 刷新 | 渲染后，无 HTTP | P2 | 装 embed 后才用 | 未测 |
| W7 connectionStore.init 取 token | connectionStore.ts:150-153；api/auth.ts:213-225 | D9 | W8 | P0 | P0 | WS 前置 | 未测 |
| W8 new WebSocket（= T0） | api/agentWs.ts:114-131,327 | W7 | W9–W12 | P0 | P0 | 首个请求 | 未测 |
| W9 auth 帧 | agentWs.ts:135-141 | W8 | 无 | P1 | **删除**（WP-159） | serve 不认 | -32601 |
| W10 onopen → connected | agentWs.ts:139；connectionStore.ts:157-160 | W8 | W11 | P0 | P0 | — | 未测 |
| W10a knowledge onReconnect | connectionStore.ts:161 | W10 | 知识面板 | P1 | P2 | 首开本就跳过 | 未测 |
| W11 session/list 请求 | connectionStore.ts:164 → sessionStore.ts:658-680 | W10 | W17（恢复路径） | P1（恢复路径挡输入） | P1 | 恢复路径关键依赖 | 240–280 |
| W11a telemetry | connectionStore.ts:172 | W10 | 遥测 | P1 | P2 | — | 未测 |
| W12 收 server/hello | agentWs.ts:435-438；connectionStore.ts:255-279 | W8 | W13–W16 | P0 | P0 | 新 profile 解锁前提 | 未测 |
| W12a 模型列表 | sessionStore.ts:360-368 | W12 | 发送按钮 | P1 | P1 | 只挡发送 | 未测 |
| W13 session/new → root 面板 | settingsStore.ts:420-423；centreChat.ts:23-28；sessionStore.ts:585-597 | W12 + W5 | W17（新 profile） | P0 | P0 | 新 profile 输入框等它 | 约 60 |
| W14 GET engines | connectionStore.ts:264；engineStore.ts:108-110 | W12 | 引擎 UI | P1 | **P2** | 按需 | 未测 |
| W15 GET settings summary | settingsStore.ts:424-430 | W12 | 设置面板 | P1 | **P2** | 按需 | 未测 |
| W16 notifyWorkspace IPC | connectionStore.ts:277 | W12 | 壳标题/recents | P1 | P2 | 不影响输入 | 待真机测 |
| W17 AgentPanel writable 判定 | dockview/panels/AgentPanel.tsx:166-169 | W13 或 W11 | W18 | P0 | 新 profile P0；恢复路径 **P1 即时** | 不等 list | 见 W11/W13 |
| W17a session/subscribe + 回放 | AgentPanel.tsx:111-117；turnStore.ts:539,575,593 | W10 | replaying（挡发送） | P1 | P1 | 只挡发送 | 未测 |
| W18 输入框渲染（= T_int） | components/AgentChatInput.tsx:424-429,737-750 | W17 | 用户 | P0 | P0 | 终点 | 见 W13 |
| W19 文件树 loadRoot | FileTree.tsx:418,469-470；treeStore.ts:133-146 | W5 | 文件树 | P1 | P1，排在 W13 之后 | 大仓上避免抢 serve | 未测 |

### 2.5 后台与按需（IDE 基座 + Session + 前端）

| 工作项 | 代码位置 | 依赖 | 被依赖 | 现阶段 | 建议 | 理由 | 耗时 |
|---|---|---|---|---|---|---|---|
| #24 LSP warmup → hub.activate | src/engines/lsp_engine.rs:48/74 → src/lsp/hub.rs:202 | #10 | 引擎状态 | P1 | **P2** | 首需时再查 | Warm 约 +0.4–0.6s（§10 C3） |
| #25 npm config get prefix | src/lsp/deps.rs:490 → :317/237/165/129 | #24 | #26 | P1 | **P2** | 仅 npm 分发的 server 需要，可缓存 | T0−18~−2（§10、§13） |
| #26 `<server> --version` | deps.rs:423 | #25 | #24 | P1 | **P2** | 版本管理器可能下载工具链 | +372~+565（§10 C3、§13） |
| #27 language server spawn + initialize | hub.rs:220 → server.rs:144/461 | #24、#8 | lsp/request、LSP 工具、诊断 | P2 | P2 | 已按需 | 未测 |
| #29 code_search warmup（起 worker、握手） | src/engines/code_search_engine.rs:374/399/457 | #10 | 首次搜索 | P1 | P1 | 不在关键路径 | Warm 无索引 1.2–1.5s，大索引 2.4–2.6s（§2、§10） |
| #30 worker 目录准备 | code_search_ipc/server.rs:87-99 | #29 | #31 | P1 | P1 | — | 未测 |
| #31 加载 ORT 模型 | server.rs:100 → code_search/embed.rs:319 | 模型文件 | #32 | P1 | P1 | — | 含在 Warm |
| #32 warmup_index（usearch、BM25） | server.rs:101-106 → code_search/mod.rs:813 | #31 | 搜索 | P1 | P1 | 随索引增长 | 6,257 块约 +1.2s（§2） |
| #33 queue_reconcile_dirty（全量 stat） | server.rs:110 → reconcile.rs:87 | #32 | 增量 embed | P1 | P1 | 随文件数增长，后台 | 未测 |
| #33s worker 会话语义索引（load_session_index、stale Running 清理） | server.rs:111-118；semantic_index.rs:1705/1412 | #29 | 会话搜索 | P2 | P2 | worker 内按需 | 小仓建索引峰值 2.07GB；大仓未建成 |
| #34 path-index fill | path_index/mod.rs:229-260 | #11、#21 | 路径补全 | P1 | P1 | 后台 | 未测 |
| #35 终端 PTY | client_protocol/workspace.rs:136 → terminal/mod.rs:297 | #18、#23 | 终端 UI、bash 工具 | P2 | P2 | 已按需 | 未测 |
| #36 code-index tick（30s，首 tick 立即） | router.rs:35/213-228 → engines/mod.rs:934/1096 | #20、#29 | 索引新鲜度 | P1 | P1 | 首次 embed 由第二个 tick 触发 | 首次 embed 约 +30s；首建 1,103s（§7） |
| #37 change-bus 订阅 | router.rs:116-165 | #21 | #22、#29、#34 | P1 | P1 | 保持索引最新 | 未测 |
| #38 rescan 订阅 | router.rs:230-249 | #21 | #29、#34 | P1 | P1 | 丢事件后恢复 | 未测 |
| #39a gc_stale_empty_sessions（启动即跑，之后每小时） | router.rs:172-178 → manager.rs:2014 | SessionManager | 无 | P1 | **P2** | 与打开无关 | 未测 |
| #39b session index tick（30s，首 tick 立即；sparse maintenance、dense consume） | router.rs:32、约 :180-207；lexical.rs:289；engines/mod.rs:865 | reader、turn_guard | session_search | P1 | **P2**（首 tick 延后） | 避免争首连接 | 未测 |
| #40 shutdown watch | router.rs:260-266 → shutdown.rs:67/81 | CLI | 退出 | P1 | P1 | 进程生命周期 | 未测 |
| W18a 首次 focus 加载 knowledge | AgentChatInput.tsx:751-756 | 用户 focus | @mention | 按需 | P2 | 已按需 | 未测 |
| W20 LSP 请求/诊断推送 | web/src/lib/litecodeLsp.ts:185,258,275 | 打开编辑器 | 诊断/补全 | 按需 | P2 | 启动时不初始化 | 未测 |
| W21 Milkdown、pdfjs、mermaid 等 lazy import | EditorPane.tsx:50-56 等 | 按需 | 对应视图 | 按需 | P2 | 保持 | 未测 |

前端 W22（i18n）在代码里不存在，不列。

## 三、必须留在 P0、凭代码判断偏慢的项（后续优化候选）

1. **session/list**（Session；connection.rs:891 → session_lifecycle.rs:410 → data/sqlite/read.rs:235-247）：在 async 线程上同步查询、不按 project 过滤，之后每行再查 is_session_empty / is_turn_running / session_status / binding，N+1。实测 0 会话 15，18 会话 64。注：查询带 `LIMIT 50` 且只取根会话，所以行数有上限，主要成本是逐行附加查询，不是无界全表。
2. **SessionData::open 的 schema 校验与迁移**（Session；state.rs:90 → data/sqlite/conn.rs:27,54 → schema.rs:102,264）：每次启动多次 COUNT(*)，含对 sessions 全表的 schema_version 检查；升级后首次启动跑 migrate_reminders_v7 改写 transcript_items，大库上会慢。常规打开实测 1.8 / 2.0（§5），迁移未测。
3. **每连接 runtime.apply(DocId::ALL)**（Session；ws.rs:124）：hello 依赖它，耗时随配置和模型目录增长。未测。
4. **壳等 READY + /health 后才建窗口**（壳；main.ts:1369-1384，sidecar.ts:115，main.ts:619-650）：serve 启动必须 P0，但窗口等它不是必须的，改法见第一节。待真机测。
5. **新 profile 的 hello → session/new 两段串行往返**（前端+Session；agentWs.ts:435，centreChat.ts:23-28 → sessionStore.ts:591-597）：约 60 解锁。候选改法见第四节第 3 条。
6. **首屏 bundle 解析**（前端；main.tsx:1-16）：解析本身 P0，monaco 和多余字重可移出（第一节）。未测。
7. **后端其余 P0 项**：IDE 判定没有偏慢项，均为固定开销。唯一随规模增长的是 restart_watcher（router.rs:112 → watcher.rs:43，Linux 每目录一个 inotify watch），1,505 个文件 5ms，不算慢；数万目录的仓库上未验证。

## 四、跨域冲突与去重（需 GM 裁定的标 ⚑）

1. ⚑ **maintain_snapshots 建议阶段不一致**：IDE 建议 P1，并要求移到后台时必须持 workspace_snapshot_lock（现在不持，可能与首轮 snapshot 竞争）；Session 建议 P2。两者都认为它不该留在 P0。汇总建议：取 P1 后台执行并加锁；它与 warm_snapshot_repo 共用同一把锁，阶段宜一起定。请 GM 定稿并指定归属域。
2. ⚑ **sweep_orphan_lsp_processes**：Session 表中列为 P0（可议 P2），并注明越出本域；IDE（本域）建议 P1，理由是须在首个 language server 启动前完成。汇总按 IDE 的 P1 处理，Session 那一行视为重复。
3. ⚑ **hello 直接下发根会话、合并 hello 与 session/new**：前端中层的建议，需要改 serve 侧握手协议，Session 尚未确认可行性和协议影响。请 GM 交 Session 评估。
4. **auth 帧**：前端（W9）与 Session（-32601）是同一项，结论一致：前端删除发送（WP-159），serve 不加方法。
5. **session/list 与回放**：Session 的“先回 list 再回放”（后端）与前端的“root 面板不等 list 即可写”（AgentPanel.tsx:169）针对同一段 200ms，两者可独立实施；做了前者，后者的收益会变小，建议 GM 定先后。
6. **重复行已合并**：WorkspaceWriteLease、SessionData::open、READY、gc/session index tick（IDE #39 交 Session）、worker 会话索引（IDE #33s 交 Session）、token 解析（IDE #6 与 Session run.rs:19-24）。

## 五、备注

- 测量已按 Andy 的决定停止；表中未测项不再补测。desktop 各项写“待真机测”。
- 大仓 C2（热索引）缺口：4 次均在 36–50 秒内 MemAvailable 跌破 4GiB（单 worker 峰值约 3.07GB，最低 3.39GB），没有建成；补测需起点内存约 7.5GiB 以上，脚本在 `WP-156` artifact。小仓 C2 解锁 A 66 / B 268，与 C1 基本一致。
- IDE 对 WP-155 §10 的修正：A7 EngineManager::reconcile 现阶段应为 P0（不是 P1），建议 P2；A10d maintain_snapshots 改为建议 P1 并补加锁要求；A6 sweep_orphan_lsp 由 P2 改为 P1；补列 knowledge 线程、change-bus/rescan、code-index tick、queue_reconcile_dirty、remove_legacy_text_index、3a 的 git 子进程、shutdown watch；W3 首次 embed 出现在 +31s 由 code-index tick 第二轮解释。§11–13 结论不变。
- 相关 issue：ISSUE-061（SIGTERM 后 code-search-worker 成孤儿，不属启动阶段）；ISSUE-062（外部 blob 缺失时 sparse maintenance 每 30s 报错）；ISSUE-060 内容是 client_protocol 遗留的无引用 LspNotification struct，与启动无直接关系，仅因 WP-156 相关讨论被引用。
- ⚑ **warm_snapshot_repo**：WP-155 §11 按 Andy 的严重度定为 (3) 架构设计错误。代码在 config（workspace.rs:513 调用）和 session（snapshot.rs 实现）两处，IDE 的清单在写但不是该代码的归属域。建议 GM 指定负责域后再立包。
- WP-156 原始 summary.json 因磁盘回滚丢失，Session 数字取自重写后的 session.md。
