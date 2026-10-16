# WP-166 启动排位整改 · Session 域 — 实施方案（session）

父包 WP-165（GM 已批）。阶段定义见 .tmp/wp154/plan.md:5（P0 打开前必须做 / P1 打开后马上做 / P2 打开后可延迟按需做）。参考 .litecode/ops/artifacts/WP-154/session.md、backend.md。
行号均已对照当前 HEAD（3ddc643）核实。功能行为不变；批准前不改代码。注：任务描述里的 "connection.rs" 实际路径为 src/client_protocol/connection.rs，"state.rs" 为 src/serve/state.rs。

## 0. 现状核实（HEAD）
- 串行循环：src/client_protocol/connection.rs:1626 run_session_loop，select! 在 :1642；JsonRpc 分派在 :1767 / :1919 / :1969 → handle_jsonrpc（:226）。
- session/list：connection.rs:891 → controller/session_lifecycle.rs:410 list_sessions（list_sessions_blocking 全表 + 每行 is_session_empty / is_turn_running / session_status / list_binding_projection，N+1）；回包前先 take_all_outgoing（controller/mod.rs:1197）冲刷所有 projection 待发帧。
- session/subscribe：connection.rs:715（await 在 :748）→ controller/mod.rs:996 subscribe_checked → manager.rs:714 ensure_entry → :674 resume_session（SQLite meta/revision）→ attach → Projection → proj.snapshot → push Snapshot；回包前 take_outgoing_for（controller/mod.rs:1212）先发 Snapshot，再发 ok 回包。
- 请求入口：src/client_protocol/ws.rs:377 读循环，:391 已有 workspace 方法旁路，:490 其余送 request_tx。
- maintain_snapshots：src/serve/state.rs:97-109（ServeState::with_project 内、SessionData::open 之后同步执行）→ src/session/snapshot.rs:1258。**当前不取 workspace_snapshot_lock**。
- warm_snapshot_repo：定义 src/session/snapshot.rs:1003（已取锁 :1004）；唯一生产调用点 src/config/workspace.rs:513，位于 load_workspace_state（:491）末尾，serve 与 CLI 共用，在 bind 之前同步执行。
- workspace_snapshot_lock：snapshot.rs:112（每工作区一把 std::sync::Mutex，SNAPSHOT_LOCKS 表锁 :105 仅取 Arc 时短暂持有）。
- 会话索引 tick：src/serve/router.rs:32 SESSION_INDEX_TICK=30s；spawn 在 :182-207，interval 首 tick 立即完成（:189-193 注释），首 tick 跑 spawn_sparse_maintenance（engines/session_search/lexical.rs:289）与 consume_session_index_work（engines/mod.rs:865）。
- gc_stale_empty_sessions：router.rs:172-178（spawn 后立即跑一次，然后每 60min）→ session/manager.rs:2014。

## 1. 先回 list，再做回放（subscribe/回放 P0→P1）

### 改动位置
1. 抽纯函数：controller/session_lifecycle.rs:410 list_sessions 的主体抽成 `pub(crate) fn build_session_list(sessions: &SessionManager, resolved: &ResolvedConfig) -> anyhow::Result<Vec<SessionInfo>>`（逐行逻辑、过滤、字段完全不动）；原 `list_sessions(&mut self)` 改为调用它（保留，供测试与 deferred 路径）。
2. list 快车道：ws.rs:490 之前（与 :391 workspace 旁路同层）拦截 `methods::SESSION_LIST`：clone `state.sessions`(Arc<SessionManager>) 与 resolved，`tokio::task::spawn_blocking` 里跑 build_session_list，直接 emit 回包/错误到 response_tx（同一 writer 任务，保证整帧原子）。错误分支保持现有 -32000 + operation_error(ListSessions) 两帧（operation_error 需要的 first_session_id 改由 SessionManager 读取，等价）。
3. connection.rs:891 原分支保留（loop 内 fallback / 单测路径），行为不变。
4. subscribe 本身不改（仍在串行循环、仍 await subscribe_checked），因此回放仍由同一循环串行产出 —— 这就是"回放降到 P1"：它不再挡 list 回包。

### 保证行为不变
- **同一会话回放的顺序与内容**：subscribe_checked、proj.snapshot、take_outgoing_for、ok 回包的代码路径一字不改，仍在唯一的 session 循环内执行；一个会话的 Snapshot → 增量（merged_rx 分支 :1645）→ 回包的相对顺序只由该循环决定，快车道不产生任何会话事件帧，所以不会插进回放序列。
- **Snapshot 与后续增量的 seq 连续性**：增量由 subscribe_checked 里 spawn 的 forward_task 经 merged_tx 进循环，Projection 用 proj 内状态去重/接续；快车道不碰 Projection、不碰 merged_rx，连续性条件与现在相同。
- **同一连接上消息相对顺序**：唯一变化是 list 回包可能早于之前已发出、尚未完成的 subscribe 回包。JSON-RPC 按 id 配对，允许乱序；前端中层已确认 unlock 以 list 回包为准、根面板不等 list（WP-165"不做"项），回放消费 Snapshot 不依赖 list 先后。除 list 回包外，其他所有帧的相对顺序不变。
- **take_all_outgoing 冲刷**：快车道不再在 list 回包前冲刷 projection 待发帧。核对：循环的每个产生帧的分支（merged_rx :1645、perm :1660、subscribe :748、各 RPC）都会在本分支末尾自行冲刷，list 前的冲刷只是兜底；loop 内 fallback 分支保留冲刷。用测试 T1-5 断言没有帧滞留。
- **并发 subscribe / unsubscribe / send**：三者仍走串行循环，处理顺序 = 到达顺序，与今天完全一致；快车道只读（list_sessions_blocking、is_session_empty、is_turn_running、session_status 都是读），与它们的竞争和今天"多连接同时 list"的竞争相同（session_status/is_turn_running 本就是瞬时读）。list 中某会话 running/status 的读取时刻可能比原来更早（在 subscribe 前），这与另一窗口在同一时刻 list 的结果等价，不是新语义。
- 跨连接：每个连接各自快车道，无共享可变状态。
- spawn_blocking 避免同步 SQLite 读压在 async worker 上（今天在循环里同步读，属于既有问题，顺带不恶化）。

### 测试（connection.rs tests 模块 / ws 层）
- T1-1 `session_list_not_blocked_by_inflight_subscribe`：cfg(test) 钩子在 resume_session 前挂 barrier，使 subscribe 卡住；发 subscribe 再发 list；断言 list 回包先到且 sessions 内容正确，放开 barrier 后 subscribe 回包到达。
- T1-2 `session_list_fastlane_matches_controller_list`：同一 DB（含空父会话、空子会话 WP-091、运行中会话），断言 build_session_list 与 controller.list_sessions 结果逐字段相等；保留并继续通过 `wp091_list_sessions_includes_empty_child`（:2756）。
- T1-3 `subscribe_replay_frames_unchanged`：有回合的会话，subscribe 后收集帧，断言顺序为 [可选 session_attached] → Snapshot → ok 回包，Snapshot 内容与改动前 golden（同一 fixture 生成）相等。
- T1-4 `snapshot_then_delta_seq_contiguous_with_concurrent_list`：subscribe 时并发跑回合产生增量并穿插 list，断言 Snapshot 最后 seq + 1 == 第一条增量 seq，无重复无缺口。
- T1-5 `no_projection_frames_stranded_after_fastlane_list`：在有待发帧时发 list，断言后续一次循环迭代内所有帧被发出。
- T1-6 `subscribe_unsubscribe_send_order_preserved`：同一连接依次 subscribe A → agent/run A → unsubscribe A → list，断言前三者回包与事件顺序与改动前一致，list 不影响。
- T1-7 `session_list_error_emits_operation_error_and_rpc_error`：DB 读失败时仍是两帧、code 不变。
- 现有 `ask_does_not_block_other_session_rpc`（:2286）保持绿。

## 2. maintain_snapshots 与 warm_snapshot_repo：P1 后台、持 workspace_snapshot_lock

### 改动位置
1. snapshot.rs:1258 maintain_snapshots 签名改为接收 workspace 与一个 id 提供闭包：`maintain_snapshots(workspace, snapshots_dir, active_ids: impl FnOnce() -> Vec<String>)`；函数内先 `workspace_snapshot_lock(workspace).lock()`，**持锁后**再调用闭包读活跃 id、再枚举 refs 与删除。删除逻辑、判定（孤儿/陈旧/空 refs）不变。
2. state.rs:97-109 的同步调用删除，改为在 router listen 完成 bind 之后（router.rs 现有后台 spawn 区，:172 前）`tokio::task::spawn_blocking`，闭包为 `session_data.list_session_ids_blocking()`（session/data/mod.rs:503 / :360），日志与错误处理文案保持原样。
3. warm_snapshot_repo：config/workspace.rs:513 的调用从 load_workspace_state 移出——新增 `load_workspace_state_with(override, WarmSnapshot::{Now, Deferred})`，原 `load_workspace_state` = Now（CLI 及其他调用方行为不变）；serve 启动路径（serve/run.rs:~56-91 经 ConfigManager 取 workspace state 处）用 Deferred，并在 bind 后与 maintain 同一后台任务里先 warm 再 maintain（顺序与今天一致：今天 warm 在 load_workspace_state 里，maintain 在 ServeState 里，warm 先）。warm 本身已持锁，不改函数体。
4. 首回合先于 warm 的兜底：snapshot.rs 增加每工作区 `WARMED` 标记（与 SNAPSHOT_LOCKS 同表或并列 LazyLock<HashSet>），warm 成功后置位；snapshot_track（:1023）/ snapshot_record_patch / snapshot_restore 在**已持锁**后若未置位，先执行 `clear_stale_index_lock(repo.path())`（warm 里唯一 track 不做的副作用；write-tree 由 track 自己做）。这样即使首回合抢在 warm 前拿到锁，也与今天"warm 已完成"等价。

### 保证不竞争 / 不死锁
取 workspace_snapshot_lock 的全部位置（HEAD）：
| 位置 | 函数 | 执行线程 |
|---|---|---|
| snapshot.rs:1004 | warm_snapshot_repo | 现：load_workspace_state 同步；改后：spawn_blocking |
| snapshot.rs:1029 | snapshot_track | runtime/mod.rs:1122 spawn_blocking |
| snapshot.rs:1075 | snapshot_record_patch | runtime/mod.rs:1214 后台 |
| snapshot.rs:1134 | snapshot_restore | controller/mod.rs:1368 block_in_place+spawn_blocking；runtime/pending_auto_turn.rs 测试 |
| 新增 | maintain_snapshots | spawn_blocking |
| snapshot.rs:1875/1902/1915 | 测试 | — |
未取锁、同样改 refs 的：snapshot.rs:1245 delete_session_snapshots（session/data/sqlite/session.rs:1642 删除会话时调用）。它与 maintain 都只是删 refs/patch，幂等，不会产生错误结果；为不"顺手改别的"，本包**不**给它加锁，作为观察项列出，是否另立包由审批人定。
- 死锁分析：锁为不可重入 std::Mutex；以上持锁函数体内均不调用另一个持锁函数（maintain 只调内部 delete_session_refs/delete_session_patches，不调 delete_session_snapshots）；SNAPSHOT_LOCKS 表锁在拿到 Arc 后即释放，不与工作区锁嵌套；maintain 的 id 闭包只读 SQLite（SessionData 自己的连接锁），SQLite 侧任何路径都不会反向等待快照锁 → 无环，无死锁。所有持锁方都在 blocking 线程，不会卡 async worker。
- 正确性竞争：今天 maintain 在任何连接之前跑，所以"活跃 id 列表"与 refs 一致。后台化后若先读 id 再枚举 refs，会话在两者之间新建并 track 会被误判孤儿删除。方案在锁内读 id：track 需要同一把锁，所以锁内枚举期间不可能新增 refs；锁前已 track 的会话其行在 session/new（manager.rs:555 mutate Create）时就已写入 → 一定在活跃 id 里。删除会话与 maintain 并发：两边都删同一 refs，幂等。
- 中毒语义与现有持锁方一致（expect）。

### 测试（snapshot.rs tests、config/workspace.rs tests、serve 测试）
- T2-1 `maintain_snapshots_waits_for_workspace_lock`：测试线程持锁，后台跑 maintain，断言未返回；释放后完成。
- T2-2 `maintain_reads_active_ids_under_lock_no_false_orphan`：在锁内模拟"新会话建行 + track"，断言新会话 refs 不被删除。
- T2-3 现有 maintain 相关测试（孤儿/陈旧/空 refs 计数）全部保持断言值不变，仅改调用签名。
- T2-4 `track_before_warm_clears_stale_index_lock`：放一个超龄 index.lock，未 warm 直接 track，断言成功且 ref 写入；与先 warm 再 track 的结果 tree oid 相同。
- T2-5 `load_workspace_state_deferred_does_not_warm`、`load_workspace_state_default_still_warms`（扩展 :745 load_workspace_state_uses_external_snapshots_dir）。
- T2-6 `serve_state_does_not_run_snapshot_maintenance_inline`：构造 ServeState 后 refs 仍在；触发后台任务后孤儿被删，日志计数一致。
- 现有 `workspace_snapshot_lock_is_shared_per_workspace`（:1350）与 :1875-1941 并发测试保持绿。

## 3. 会话索引首次 tick 与 gc_stale_empty_sessions：P1→P2

### 改动位置（src/serve/router.rs）
- 新增 `startup_settled: Arc<tokio::sync::Notify>`（或 watch<bool>）放 ServeState；在任一连接首个 session/list 或 session/new **回包发出后**触发一次；兜底超时 `P2_FALLBACK = 10s`（无客户端连接的 headless 场景也会执行）。
- router.rs:172-178：gc 首次执行前 `await settled_or_timeout`，之后每 60min 不变；EMPTY_SESSION_TTL 不变。
- router.rs:182-207：循环前先 `await settled_or_timeout`，再用现有 interval（首 tick 立即）进入原循环；循环体（sparse maintenance 每 tick、dense turn-gated、consume_session_index_work）一字不改。

### 语义不变
- gc：判定（空会话 + TTL）不变，只推迟首跑若干秒；session/list 本就过滤空父会话（session_lifecycle.rs:417），所以 gc 早晚不影响 list 输出；与 session/new 的竞争和今天每小时那次一样（TTL 保护新建空会话）。
- 索引：搜索路径本就只读索引现状（router.rs:178-180 注释"stale is fine, wrong is not"，build_agent_view 复核命中），首次修复推迟到 P2 只会让启动后最初几秒的搜索看到与今天首 tick 前相同的旧索引；修复内容、频率、turn 门控不变。
- 跨域：tick 调用的是 IDE 域 engines API（spawn_sparse_maintenance、consume_session_index_work、session_reader），**本包不改 engines 代码**，只改 router.rs 的调用时机；需 IDE 知会/会签。

### 测试
- T3-1 `session_index_first_tick_waits_for_startup_settled`（tokio::time::pause）：未触发 settled 时推进 9s，断言 consume 未被调用（计数钩子）；触发后立即调用一次。
- T3-2 `session_index_first_tick_falls_back_after_timeout`：不触发，推进 10s，断言执行。
- T3-3 `gc_stale_empty_sessions_deferred_until_settled`：同上两路断言；首跑后 60min 周期不变。
- T3-4 现有 manager.rs gc 测试与 engines/mod.rs:1709 `consume_session_index_work_does_not_mark_code_refreshing` 保持绿（不改）。

## 4. 跨域项（单独标出）
- src/config/workspace.rs（model-config 域，无 owner_agent → 需后端中层/GM 指定会签）：新增 load_workspace_state_with + WarmSnapshot 枚举，默认路径行为不变。
- src/serve/router.rs 会话索引 tick：代码在 serve（本域），但调用对象是 engines（IDE 基座域），语义由 IDE 定义 → 请 IDE 会签"推迟首 tick"。
- 与 WP-167（IDE）在 router.rs / serve 启动路径可能同文件改动（EngineManager::reconcile P0→P2 在 serve/run.rs:84-88），需后端中层排定合并顺序以免冲突。
- 不涉及前端代码；前端只依赖 list 回包可能先于 subscribe 回包（已确认）。

## 5. 风险与回退
- R1 前端隐式依赖 list 在 subscribe 之后：前端中层已确认不依赖；仍以 T1-1/T1-6 + WP-168 联调验证。回退：ws.rs 拦截处加开关 `LITECODE_LIST_FASTLANE=0` 回到循环内分支（原代码保留）。
- R2 快车道漏冲刷帧：T1-5 覆盖；回退同 R1。
- R3 maintain 后台误删：锁内读 id（T2-2）；回退：把调用移回 state.rs:97 原位（函数仍持锁，安全）。
- R4 首回合早于 warm：WARMED 兜底（T2-4）；回退：serve 改用 WarmSnapshot::Now。
- R5 P2 推迟导致首几秒搜索略旧：与今天首 tick 前一致；回退：P2_FALLBACK 设 0。
- 每项独立 commit、diff 落 .litecode/ops/artifacts/WP-166/，可单项 revert。

## 6. 执行与护栏
批准后才改代码。编译/测试为重型任务，由后端中层放行：-j2、单重型 ≥4.5GiB、RSS watchdog。只跑相关模块测试（client_protocol::connection、session::snapshot、config::workspace、serve::router、session::manager）后再按放行跑全量。不 commit 他人在途改动。

## 7. 修订说明（实施时，2026-10-15，按后端中层批准条件）
批准条件：① 去掉 LITECODE_LIST_FASTLANE 开关，快通道是唯一行为，不加新机制层；② 去掉 10s 兜底，信号只在首个 list 或 new 回包时发出。签字：config/workspace.rs:513 归 Runtime；router.rs tick 推迟归 IDE。顺序：WP-166 先合，WP-167 rebase。delete_session_snapshots 不在范围（ISSUE-063），未改动。

相对第 1–5 节的变更：
- 1：无开关。list 快通道在 ws.rs 读循环（SESSION_LIST 拦截，位于 request_tx.send 前）用 tokio::spawn 跑 `build_session_list`（async，不是 spawn_blocking——行内 await is_session_empty 等）。读失败时把原请求转交 session 循环的 connection.rs SESSION_LIST 分支，错误帧（operation_error + -32000）与改动前逐字相同。resolved 取自 `state.runtime`（共享 RuntimeHandle），原来取的是本连接 controller 的 runtime 副本；二者只在“设置已改但本连接尚未 apply”这一窗口内不同，此时快通道给出的是更新后的 label/model，与同时推送的 models/changed 一致。请审批人确认可接受。
- 2：不新增 WARMED 标记。核实发现 snapshot_track / snapshot_record_patch 都经 snapshot_write_tree（snapshot.rs:590），它本身先 clear_stale_index_lock 并在 Locked 时强清重试，open_or_init_repo 也会建库，所以首回合抢在 warm 前完全自足；warm 只是预热。改为加测试 T2-4 证明两种顺序结果一致。
- 2：maintain_snapshots 原本无单测，“现有 maintain 测试不变”一条作废，改为新增 T2-1/T2-2。WP-155 harness（.litecode/ops/artifacts/WP-155/harness）用旧签名，属归档产物，未改。
- 2：warm 延后的贯通路径实际为 cli.rs:93 → ConfigManager::load_runtime_bundle_with（config/manager.rs）→ load_workspace_state_with（config/workspace.rs）。serve 子命令传 Deferred，其余调用方行为不变。因此 config/manager.rs、cli/cli.rs 也是跨域文件，与 workspace.rs 一起请 Runtime 签字。
- 3：无 10s 兜底。信号放在 SessionManager（`startup_settled: watch::Sender<bool>`，提供 mark/is/wait 三个方法），在快通道 list 成功回包后、loop 内 SESSION_LIST/SESSION_NEW 分支回包后（成功或失败均算“回包”）置位。gc spawn 抽成 router::spawn_empty_session_gc；snapshot P1 抽成 router::spawn_snapshot_upkeep（bind 之后启动，先 warm 再 maintain）。
- 测试：删去 T3-2（兜底）和开关相关用例。T1 放在 src/client_protocol/ws_wp166_tests.rs（真实 WS；manager.rs 里新增 cfg(test) 的 resume_gate_for_test，可把 subscribe 卡在 resume_session），T2-6/T3 也在这个文件。会话索引首 tick 的推迟没有单独的行为测试：tick 体调用 engines，内部不可观测，只由 T3-1 覆盖信号本身，接线靠评审核对，请 IDE 签字时一并看。
- 风险更新：去掉兜底后，不发 list/new 的客户端下 P2 不会启动（见 §8）。

## 8. 去掉兜底后的依赖核查（只读）
- CLI：没有依赖。只有 Serve 子命令进入 serve::run → router::listen（cli.rs:744-757），tick 与 gc 只在 listen 里起（router.rs 内 spawn_empty_session_gc 与会话索引 tick），CLI 从来不跑这两项。
- 经 HTTP 创建会话：没有这样的路由。router 只挂 /health、/ws、/api/workspace、/api/settings、/api/media（router.rs:70-74）；会话只能由 WS session/new（connection.rs:680）或回合内子代理创建，子会话本来就被 gc 跳过（manager.rs gc_stale_empty_sessions 首个条件 is_child_session）。
- 走 HTTP、不走 WS 的检索：**有依赖**。POST /api/workspace/retrieval/search（workspace/routes.rs:207 → post_retrieval_search :394 → WorkspaceEngines::human_search engines/mod.rs:509）。其中 sparse lane 每次查询都会自己触发维护（engines/session_search/lexical.rs:151-156 prepare_index → spawn_sparse_maintenance），不依赖 tick；dense 会话 lane 只靠 tick 的 consume_session_index_work（router.rs tick 体；engines/mod.rs:865）或手动 POST /api/workspace/retrieval/refresh（routes.rs:206/374 → engines/mod.rs:815/829）刷新。所以只走 HTTP、从不发 list/new 的客户端，dense 会话结果在手动 refresh 之前一直不更新（以前启动时首 tick 就会刷新）。
- headless serve / 不发 list、new 的 WS 客户端：**有依赖**。ws.rs:138 的 `?session=` 提示会自动订阅，随后可以直接 agent/run（connection.rs:176 resolve_sid）而从不发 list/new；web/scripts/ws-smoke.mjs 不带 `list` 参数时也是这样。此时 gc 与会话索引 tick 都不会启动，回合里 session_search 工具（tools/session_search.rs:77 → engines.search_sessions）的 dense lane 也一样不刷新。gc 不跑只会让空会话多留，list 本身会过滤（session_lifecycle.rs build_session_list），对功能没有影响。
- 结论：正式前端（打开时必发 list 或 new）不受影响；上面两类非前端路径确实依赖 tick（dense 会话 lane）。是否接受由审批人决定，可选：在 agent/run 或 retrieval/search 首次调用时也置位（会扩大信号来源，需要再次批准）。

## 9. 扩大 settled 信号来源（后端中层批准，2026-10-15）
- 选择：首个 agent/run 和首个 retrieval/search 也发信号，不选“任一客户端请求”。理由：改动更小（两行调用，复用已有的 `mark_startup_settled`，幂等，只有首次生效）；而且行为不变——正式前端打开时先发 hello 再发 list/new，P2 启动时刻与 §3 相同，只有不发 list/new 的客户端才会从这两个入口启动 P2。如果改成“任一请求”，前端握手的第一条请求就会提前启动 P2，会和 P0 的 list 重叠，违背排位目标。
- 位置：connection.rs AGENT_RUN 分支开头（参数校验之前，所以空输入被拒的 run 也算）；workspace/routes.rs post_retrieval_search 开头（**跨域：src/workspace 属 IDE 基座，请 IDE 会签**）。
- 测试：ws_wp166_tests.rs `startup_settled_fires_without_list_or_new`。① `?session=` 连接只发 agent/run，断言信号置位；② 不开 WS，只 POST /api/workspace/retrieval/search（Bearer），断言信号置位。
- 不在范围：session_search 工具调用都在回合内，已经被 agent/run 覆盖。
- 补充（IDE 评审后）：后端中层已明确要求去掉兜底超时；没有客户端时 tick 和 gc 不跑是有意为之；非前端路径由首个 agent/run 和首个 retrieval/search 覆盖。routes.rs 的入口先判 `is_startup_settled()`，未置位才调用 mark。
