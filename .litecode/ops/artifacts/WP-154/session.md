# WP-154 启动工作全清单 — Session 域（src/session, src/client_protocol, src/serve, src/engines/session_search）

- 方式：只读代码（未编译、未起 serve、无 git 写）。位置均按当前 HEAD（3ddc643）核对。
- 阶段定义取自 `.tmp/wp154/plan.md:5`：**P0 打开前必须做 / P1 打开后马上要做 / P2 打开后可延迟按需做**。
  “打开”按 WP-156 的 unlock 理解：A 路径 = session/new 回包，B 路径 = session/list 回包。READY 之前和 unlock 关键路径上的工作都算“现在 P0”。
- 已测耗时：只用 `.litecode/ops/artifacts/WP-156/session.md` 里的数（ms，相对 T0，取中位数），其余写“未测”。
- 不覆盖 `warm_snapshot_repo` / `load_runtime_bundle`（属 IDE 域）。注意：下面的 `maintain_snapshots` 是 Session 域的快照保留清理，和 `warm_snapshot_repo` 不是一回事。

## 一、不一致项（现在阶段 ≠ 建议阶段）

1. **先回 list、再做回放**（connection.rs:891 / :715）
   - 现状：每个连接只有一个串行的 select 循环（connection.rs:1642）。SESSION_SUBSCRIBE 在循环里直接 `await subscribe_checked`（controller/mod.rs:996），会走 `ensure_entry`→`resume_session` 从 SQLite 恢复会话（manager.rs:714），再建 Projection、推 Snapshot。在这期间 session/list 只能排队。
   - 实测：B 路径 list 回包 238–281，A 路径只有 64。
   - 建议：list 保持 P0；恢复会话的 subscribe 和回放降到 P1，排在 list 回包之后（或者移出串行循环）。
2. **JSON-RPC `auth`**（connection.rs:1449-1456 返回 -32601）
   - 现状：前端每次连接都发一次，在串行循环里被处理，属于 P1 的白占用。
   - 建议：前端删除这次发送，serve 不加方法（按 backend_mid 的裁定）。鉴权已经在 WS 握手时由 token 完成（ws.rs:52-56）。
3. **`maintain_snapshots`**（state.rs:94-108 → snapshot.rs:1258）
   - 现状：在 ServeState::new 里同步执行，排在 READY 之前（P0）。要列出全部 session id，再打开快照仓库逐个遍历快照会话。
   - 建议：P2，挪到后台任务。它只做保留期清理，没有人等它的结果。
4. **session index 首个 tick 立即执行**（router.rs:32、约 :180-207；lexical.rs:289；engines/mod.rs:865）
   - 现状：listen 一开始就跑首个 tick，含 sparse maintenance 和 dense consume，时间和首连接的 hello/list/subscribe 重叠（P1）。
   - 建议：P2，首个 tick 延后到首屏之后（或者第一次 session_search 时）再跑。
5. **`gc_stale_empty_sessions` 启动立即执行**（router.rs:172-178 → manager.rs:2014）
   - 现状：P1。
   - 建议：P2。它每小时跑一次，和打开无关。
6. **stale Running 清理时机**（semantic_index.rs:1412，只在 load_session_index :1705-1706 里调用）
   - 现状：P2。只有 worker 真正 load 的时候才会清理，在这之前 job.json 可能一直显示 Running。
   - 建议：P2 不变，但是任何读 job 状态的入口（session_work_now :1695）都要先识别 stale。这一条是语义提醒，阶段本身不用改。

## 二、必须留在 P0、凭代码就能判断很慢（或会随规模变慢）的项

1. **session/list**（connection.rs:891 → session_lifecycle.rs:410）
   - 在 async 线程上同步做全表查询 `list_sessions_blocking`（read.rs:235-247，不按 project 过滤）。
   - 之后每一行再 `is_session_empty`（manager.rs:1944，内部是同步的 `seq_cursor_blocking`）、`is_turn_running`、`session_status`、`list_binding_projection`，是典型的 N+1，耗时 O(会话数)。
   - 实测：0 个会话 15，18 个会话 64。
2. **SessionData::open + schema 校验/迁移**（state.rs:90 → data/mod.rs:48/62 → conn.rs:27/54 `ensure_session_schema` schema.rs:102）
   - 每次启动都要做：`synchronous=FULL` 的 WAL 打开、多次 COUNT(*) 校验（含对 sessions 全表做 `schema_version !=`）。
   - 升级后首次启动会跑 `migrate_reminders_v7`（schema.rs:264），要扫描并改写 transcript_items，大库上会很慢。
3. **WS 连接时 `runtime.apply(DocId::ALL)`**（ws.rs:124）
   - 每个连接都重新应用全部配置文档，hello 依赖它，耗时随配置/模型目录增长。未测，量级待确认。
4. （如果不采纳第一节第 1 条）**恢复会话的 subscribe/回放**（connection.rs:715 → controller/mod.rs:996）：大会话耗时 O(条目数)，B 路径实测 240/282/269。

## 三、全清单

| 工作项 | 代码位置(file:line) | 它依赖什么 | 谁依赖它 | 现在所在阶段 | 建议阶段(P0/P1/P2) | 一句话理由 | 已测耗时 |
|---|---|---|---|---|---|---|---|
| serve 入口 run()：解析参数、bind 校验、resolve_web_dist | serve/run.rs:50 | 进程启动 | 以下全部 | P0 | P0 | 没有它就无法监听 | spawn→READY 小 C1 89/107、大 C1 486/498、小 C2 69/74（整段） |
| 解析鉴权 token（LITECODE_TOKEN） | serve/run.rs:19-24 | 环境变量 | auth 中间件、ws_handler | P0 | P0 | 握手校验要用 | 未测 |
| 清理孤儿 LSP 进程 sweep_orphan_lsp_processes | serve/run.rs:50 起（run 内） | 进程表 | 无硬依赖 | P0 | P0（可议 P2） | 只是清理，可以放后台；越界提示，不归本域 | 未测 |
| WorkspaceWriteLease::acquire | serve/state.rs:84 | 工作区目录 | SessionData::open | P0 | P0 | 写库前必须独占 | 未测 |
| SessionData::open（WAL/pragma、ensure_session_schema、迁移） | serve/state.rs:90 → session/data/mod.rs:48,62 → sqlite/conn.rs:27,54 → sqlite/schema.rs:53,102,264 | Lease | SessionManager、list、subscribe、session index reader | P0 | P0 | 所有会话读写的前提；见第二节第 2 条 | 未测 |
| maintain_snapshots（快照保留清理） | serve/state.rs:94-108 → session/snapshot.rs:1258 | SessionData（列出 session id）、snapshots 仓库 | 无 | P0 | P2 | 纯清理，没有人等它 | 未测 |
| SessionManager::from_data + install_parent_status_hook + set_session_reader | serve/state.rs:133 起 | SessionData | 所有 session RPC、session index | P0 | P0 | 会话状态中枢 | 未测 |
| router listen：watcher 重启、knowledge index 启动 | serve/router.rs:101-115 | ServeState | 变更总线 | P0（listen 前段） | P0（knowledge 本身是后台） | knowledge 不阻塞首轮（注释 :113） | 未测 |
| gc_stale_empty_sessions（立即一次，之后每小时） | serve/router.rs:172-178 → session/manager.rs:2014 | SessionManager | 无 | P1 | P2 | 与打开无关 | 未测 |
| session index tick（30s，首个 tick 立即执行） | serve/router.rs:32、约 :180-207 | SessionManager reader、turn_guard | session_search 结果新鲜度 | P1 | P2（首 tick 延后） | 避免和首连接争资源 | 未测 |
| sparse maintenance（每个 tick） | engines/session_search/lexical.rs:289（失败日志 :332） | session reader | lexical 检索 | P1 | P2 | 按需即可；外部 blob 缺失时每 30s 报一次 warn（ISSUE-062） | 未测 |
| dense consume（无 turn 运行时）consume_session_index_work | engines/mod.rs:865 → spawn_scoped_refresh :888 | worker 存活、session_work_now | semantic 检索 | P1/P2 | P2 | 被 turn 闸住，本来就是后台 | 未测（建索引峰值：小库 2.07GB/MemAvailable 最低 4.32GB；大库约 3.07GB 时失败，最低 3.39GB） |
| session_work_now（读 store 判断 work） | engines/session_search/semantic_index.rs:1695 | reader、job.json/queue.json | dense consume | P1 | P2 | 跟着 tick 走 | 未测 |
| queue_session_dirty（写 queue.json） | semantic_index.rs:1806 | 会话写入 | worker load | 运行时 | P2 | 增量标脏 | 未测 |
| load_session_index（读 semantic_index、job.json/queue.json） | semantic_index.rs:1705、ensure :1908 | worker 进程 | session_search 语义检索 | P2 | P2 | 只在 worker 里按需加载 | 未测 |
| stale Running 清理 recover_stale_session_job | semantic_index.rs:1412（调用点 :1706） | load_session_index | job 状态展示 | P2 | P2 | 见第一节第 6 条 | 未测 |
| HTTP auth 中间件（/ws 允许 query token） | serve/auth.rs:46-76（:66-72） | token | ws_handler | P0 | P0 | 连接准入 | 未测 |
| WS 握手：Origin 检查 + token 检查 | client_protocol/ws.rs:32-57（token :52-56） | token | handle_socket | P0 | P0 | 真正的鉴权点 | 未测 |
| LITECODE_READY 打印（listener bind 之后） | serve/router.rs:255 | listen bind | 前端/父进程开始连接 | P0 | P0 | 就绪信号 | 同 spawn→READY |
| handle_socket：runtime.apply(DocId::ALL) | client_protocol/ws.rs:124 | 配置文档 | hello、session binding | P0 | P0 | hello 的内容来自 runtime；见第二节第 3 条 | 未测 |
| SessionController::with_turn_guard | client_protocol/ws.rs:130 | runtime、SessionManager | 所有 RPC | P0 | P0 | 连接级控制器 | 未测 |
| 连接附属任务：stats ticker、terminal attach、workspace changes、knowledge registered、settings | ws.rs:153、:154、约 :200、:266 | 连接 | 侧栏/状态显示 | P1 | P1 | 打开后马上要用，不阻塞 hello | 未测 |
| 服务端推送 hello（handshake_frames，读循环之前入队） | ws.rs:339-341 → controller/mod.rs:1190、server_hello :1165 → project.rs:621 | runtime.apply、controller | 前端初始化 | P0 | P0 | 首帧 | 未测（单独值）；落在 unlock 之前 |
| 读循环 + 串行 session loop | ws.rs:377；connection.rs:1642 | handle_socket | 所有 RPC | P0 | P0 | 单循环串行，是 list 被回放堵住的根因 | — |
| JSON-RPC auth → -32601 | connection.rs:1449-1456（没有 auth handler） | 前端发送 | 无 | P1 | 删除（前端不发） | 白占一次串行处理 | 已测：返回 -32601 |
| session/new | connection.rs:680 → session_lifecycle.rs:322 → manager.rs:540 open_session | SessionManager、runtime | A 路径 unlock | P0 | P0 | A 路径的打开点 | A unlock 约 58–66；subscribe 回包约 105 |
| session/list | connection.rs:891 → session_lifecycle.rs:410 → sqlite/read.rs:235-247 | SessionData、SessionManager | B 路径 unlock、侧栏 | P0 | P0 | B 路径的打开点；N+1，见第二节第 1 条 | 0 会话 15；18 会话（A）64；B 路径 238–281（B unlock C1 239/281、C2 268） |
| session/subscribe（恢复 + Projection + 回放） | connection.rs:715 → controller/mod.rs:996（subscribe :1107）→ manager.rs:714 ensure_entry、:908 attach | SessionManager、SQLite | 历史显示 | P0（在 B 路径堵住 list） | P1 | 先回 list、再做回放 | B 路径回包 240/282/269；A 路径新会话约 105 |
| Snapshot 推送 | controller/mod.rs:266 / snapshot_for :1151（subscribe 时 take_outgoing_for 发出） | subscribe | 会话视图 | P0（随 subscribe） | P1 | 跟随 subscribe 一起后移 | A 约 57–64；B 239/281/268 |
| buffer/load | connection.rs:1391 | subscribe | 编辑区缓冲 | P1 | P1 | 打开后按需加载 | 未测 |
| 热重连（warm reconnect） | 同 hello/list/subscribe 路径 | serve 已在运行 | — | P0 | P0 | — | A 57–65；B 184–191 |

## 备注
- 大库 C2 索引在 4GiB 门限下没能建成（plan 第 4 条记为缺口），所以表里 dense 相关的只有峰值内存，没有耗时。
- ISSUE-061（SIGTERM 后 code-search-worker 成为孤儿）不属于启动阶段，只作记录。
