# WP-166 交叉评审（IDE 基座）—— router.rs tick 推迟接线 + workspace/routes.rs settled 信号

评审人：IDE 基座。方式：只读审 `.litecode/ops/artifacts/WP-166/impl.diff`、`WP-166-plan.md` 与当前工作区（工作区已含 WP-166 改动）。未编译、未跑 cargo、未做 git 写。

## 0. 先澄清范围：被推迟的不是 code-index tick
- WP-166 推迟的是 **session-index tick**（`SESSION_INDEX_TICK`，30s，router.rs:176-209，等待点 router.rs:183）和 **空会话 GC**（router.rs:313-322），不是 code-index tick。
- **code-index 30s tick**（`CODE_INDEX_TICK`，router.rs:213-225（:216））、**Lagged → `request_reconcile`**（router.rs:156-166）和 **watcher rescan/overflow → `request_reconcile`**（router.rs:226-246）这三段 diff 都没改。WP-141 的裁定（tick 和 Lagged 都走 request_reconcile）仍然成立。code_search 的后台刷新行为不变。

## 1. 发现

### F1【(4) 常见普通 bug，阻断】`mark_startup_settled` 被误挂上 `#[cfg(test)]`，非测试构建编译不过
- 位置：src/session/manager.rs:522-527（diff 在 impl.diff 的 manager.rs 第 3 个 hunk）。
- 现象：新代码插到了 `ephemeral_registry` 的 doc 注释和它的 `#[cfg(test)]` 之间。于是 `#[cfg(test)]` 落到了 `mark_startup_settled` 上，"Private in-memory registry…"这段注释也跟着挂了过去；`ephemeral_registry` 反而失去了 `#[cfg(test)]`。
- 后果：release/dev 构建里 `mark_startup_settled` 不存在。routes.rs:400、connection.rs:241/720/935、ws.rs:515 都会报 E0599，cargo build 失败。`cargo test` 在 cfg(test) 下能编过，测试会掩盖这个问题。`ephemeral_registry` 也会泄漏进生产 API。
- 修复：把 `/// Private in-memory registry…` 两行注释和 `#[cfg(test)]` 移回 `pub fn ephemeral_registry()` 正上方，让 `mark_startup_settled` / `is_startup_settled` / `wait_startup_settled` 保持无 cfg。放行编译后，除了 `cargo test` 还要跑一次非 test 的 `cargo check --bins`。

### F2【(5) 罕见边缘 bug】settled 永不触发时，session-index 维护和 GC 会无限期不跑
- 位置：router.rs:183、router.rs:315；manager.rs:545-548（`wait_startup_settled` 不设超时）。
- 触发源目前有四个：session/list 回包（connection.rs:720、ws.rs:515 快路径）、session/new 回包（connection.rs:935）、agent/run（connection.rs:241）、HTTP `/retrieval/search`（routes.rs:400）。
- 漏网场景：没有任何客户端连接的 serve（例如只开着进程、或只用 /api 的文件/git/LSP 路由）；只做 `session/subscribe` 回放、从不 list/new/run 的客户端；session/list 走慢路径且出错的情况（connection.rs:714-720 在 err 分支之后也 mark，这条没问题，ws.rs 快路径失败会回落到慢路径，也会 mark）。
- 影响：
  - (a) session-index 的"启动时修复缺失/损坏/旧 schema 索引"被推迟到首个信号。但所有 session 搜索都发生在 agent/run 之后，而 agent/run 会先 mark，所以对搜索结果的影响只是"首个回合时修复与回合并发"，和今天"索引陈旧可接受"的约定一致。
  - (b) 空会话 GC 和 dense lane（`consume_session_index_work`）在无人使用时不跑，无可见后果。
  - code_search 引擎刷新不受影响（见 §0）。后台刷新仍然静默，且只要有任一信号就会立刻开始（interval 首拍即时）。
- 修复建议（可选、最小）：给 `wait_startup_settled` 加一个兜底，例如 `tokio::select!{ wait_for, sleep(STARTUP_SETTLE_FALLBACK=60s) }`，超时视同 settled，保证"最终一定发生"。是否接受"无人连接就永不维护"属于 Session/后端中层的取舍；不加的话请在 plan 里写明这一点。

### F3【(5) 罕见边缘 bug，说明性】settled 与首拍之间的竞态：无问题
- `wait_startup_settled` 用 `watch::Sender::subscribe()` 再 `wait_for`，`wait_for` 会先检查当前值，所以先 mark 再 wait 也不会丢唤醒。
- Sender 存放在 `SessionManager` 里，而两个等待任务各自持有 `Arc<SessionManager>`，Sender 不会提前 drop，`Err` 分支（会直接放行）实际走不到。
- interval 在 settled 之后才创建，首拍立即触发，与旧行为（启动后立即一次维护）等价，只是起点后移。
- 无需修改。可以在 `wait_startup_settled` 注释里补一句"Sender 由 self 持有，Err 不可达"。

### F4【(5) 罕见边缘 bug】关停时序
- 位置：router.rs:259-267（graceful shutdown）；router.rs:255 `spawn_snapshot_upkeep`。
- 未 settled 时，两个等待任务停在 `wait_for` 上，runtime 退出时直接被取消，安全。
- `spawn_snapshot_upkeep` 是 `spawn_blocking`，被 drop 的 JoinHandle 无法取消它。若在 warm/maintain 进行中关停，tokio runtime drop 会等它跑完。持有 `workspace_snapshot_lock` 的大仓首次 warm 约 300-480ms（WP-155 实测），会相应拖慢退出，但不会造成损坏（和旧版"warm 在启动前同步跑完"相比只是挪了位置）。
- 若 settled 恰好在 shutdown 期间触发，会有一次 sparse maintenance 与 `workspace_engines.stop_all()` 并发。这在旧版的 30s tick 下同样可能发生，不算新增问题。
- 修复：无需修改；如果想要更干净的关停，可以让等待任务额外 `select!` 一下 `shutdown_watch`。

### F5【(5) 罕见边缘 bug / 轻微】routes.rs:398-400 的 settled 信号放置
本节针对 IDE 域，逐条回答：
- **放在这里对吗**：对。HTTP 侧检索是纯 /api 客户端唯一"用户已在用"的明确信号，也符合 plan §9 的意图。
- **幂等**：是，`send_if_modified` 只在 false→true 时通知。
- **便宜 / 不阻塞**：基本是。但每次搜索都会拿一次 watch 的写锁（std RwLock，纳秒级、不 await），首次以后是纯浪费。建议加快速路径：`if !state.sessions.is_startup_settled() { state.sessions.mark_startup_settled(); }`，后续调用只走读锁。
- **改不改响应 / 错误**：不改。它在空 query 的 400 检查之前，因此一个 400 请求也会触发 settled。这只影响后台 P2 何时开始，不影响响应，可以接受；如果要语义更严格，可以挪到 `if body.query.trim().is_empty()` 之后。
- **Andy 规则（code_search 的可见性只由 bind 决定，engine off/Warming 只在调用结果里体现）**：不受影响。这一行不读不写 engine 状态，也不改 bind 或可见性。
- **更好的位置**：可以考虑在 `/api` router 上加一层 tower middleware，任何 /api 请求都 mark 一次，从而覆盖文件/git/LSP-only 客户端，并顺带缓解 F2。最小方案就是保留当前这一行加上快速路径。我这边接受当前放置。

### F6【(4) 常见普通 bug，合入卫生】与 WP-167 的重叠：ide.diff 夹带了 WP-166 的 routes.rs hunk
- 位置：`.litecode/ops/artifacts/WP-167/ide.diff:729-742`。它和 WP-166 impl.diff:916-929 逐字相同（index c9ebcbb..c29629c）。
- 原因：WP-167 的 diff 是在已经含有 WP-166 改动的工作区上生成的。
- 后果：按"WP-166 先合"的顺序，在 WP-166 之上应用 ide.diff 时，routes.rs 这个 hunk 会因"已应用"而冲突或被重复应用。
- 修复（由我负责）：重新生成 WP-167 ide.diff，剔除 src/workspace/routes.rs，且以 WP-166 合入后的树为基线。

### 其他重叠核对（无冲突）
- WP-166 不改 src/serve/run.rs。serve 的 Deferred 是经 src/cli/cli.rs:93-100 传入的，所以 WP-167 的 cross-domain/session.diff（run.rs:71-78 的 sweep、:84 的 defer_reconcile）在文本上不冲突，WP-166 合入后只需按行号 rebase。
- WP-166 不改 src/lsp/*、src/engines/*、tests/startup_order_death_list.rs，与 WP-167 的文件无交集。
- 语义层面：`tests/startup_order_death_list.rs` 若断言 READY 之前的步骤顺序，需要加入 WP-166 的变化：warm/maintain 移到 bind 之后，session tick/GC 等 settled。合入 WP-166 后我会更新这个测试的期望。

## 2. 结论
**附条件签署（sign with conditions）**：
1. 必须修 F1（cfg(test) 错位），并在放行编译时跑一次非 test 构建验证。
2. F2：要么加兜底超时，要么在 WP-166 plan 中明确写入"无任何客户端信号时 P2 维护不跑"并由后端中层认可。
3. F6 由 IDE 侧在 rebase 时处理，不阻塞 WP-166。

F3/F4 无需修改。F5 的快速路径建议可选。
