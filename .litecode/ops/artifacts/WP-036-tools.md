## WP-038 Tools 方案（tools，direct）— 统一 custom+MCP 进程生命周期
父包 WP-036；产物本文件。状态：方案填写，**未 start、未改代码**。ISSUE-034 明确不在本包。

### 0 目标与范围
- **目标**：统一 custom 与 MCP stdio 子进程生命周期——超时/取消在期限内返回并杀干净进程组；Windows 父子绑定；覆盖 ISSUE-039 / ISSUE-040。
- **文件串行（硬门禁）**：①先 `src/tools/custom.rs`；②再 `src/mcp/{client,pool}.rs`（必要时 `mod.rs` 仅转发，不扩面）。
- **只调用、不改** `src/proc_lifetime.rs`（IDE 拥有）。`bind_child_to_parent` 按 LSP/code_search 既有用法调用。
- **不碰**：协议层/请求匹配（ISSUE-034）、前端、引擎产品、symlink 产品行为、`take_changes`、不 commit/push。
- **不合并调用面**：custom 与 MCP 各自保留现有 API；对齐的是 spawn/kill/超时取消语义，不是抽公共 wrapper 强行合并。

### 1 ISSUE-039（custom：X1+X2+X3 / B7）修复方向
证据（WP-027 交叉审接受）：
- `wait_child_cancellable`：`Ok(Some(status))` 后 `rx.recv()` 无界（~:449-457）——孙进程持 stdout 时超时/取消失效。
- `run_sync_with_lifecycle`：`WaitEnd::Finished` 不 `kill_child_tree`（~:334-366）；stdin writer `join()` 无界（~:339-346）。

复现（登记）：`sh -c 'sleep 30 & echo done'`，timeout=2 → 约 30s 才返回；结束后 sleep 仍在。

**修法（本包实施，批后再动）**：
1. **有界 drain**：`Ok(Some)` / wait 兜底拿到 status 后，用剩余 deadline（及 cancel）做 `recv_timeout`；超时或取消 → 走「杀组 + 短等 drain」再返回 TimedOut/Cancelled（不再无界 hang）。
2. **Finished 后组杀**：直接子进程已退出后对组 `kill_child_tree`（组号仍被存活孙进程占用，不会复用；与 WP-027 Tools 回应一致）。杀后再有界 join stdin writer（杀组关闭继承的 stdin → EPIPE，避免 X3）。
3. **Cancelled/TimedOut**：保持现有「先组杀再 `child.wait()`」；读线程/writer 仍 detach，依赖组杀后 EOF/EPIPE；若仍超时则记录但不阻塞返回。
4. **不改** `kill_child_tree` 的 unix `kill(-pid)` / Windows `taskkill /T /F` 语义（WP-027 已对齐 pty）；本包只补调用点与有界等待。
5. X5（WaitFailed 后 killpg 理论误杀）低优先级：本包顺手在 WaitFailed 路径确认 pid 仍为本 child 的 id 再杀；不扩大范围。

### 2 ISSUE-040（custom：bind_child_to_parent）
- spawn 成功后立即 `crate::proc_lifetime::bind_child_to_parent(child.id())`（对照 `lsp/server.rs:168`、`engines/code_search_ipc/client.rs:51`）。
- Unix 为 no-op（文档已说明）；主要收益在 Windows Job Object。不改 `proc_lifetime` 实现。
- 与 `process_group(0)` 并存：组杀负责 timeout/cancel 清理；Job 负责宿主崩溃级联。

### 3 MCP 对齐（client.rs → pool.rs，第二步）
现状缺口：
- `McpStdioClient::new`：spawn 后无 `process_group`、无 `bind_child_to_parent`。
- `kill` / `start_kill` / `Drop`：仅 `child.kill()` / `start_kill()`，Unix 不杀孙进程；Windows tokio kill 也不保证 /T 树杀。
- pool：`stop_inner` / `kill_child` / Drop 均委托上述 kill；timeout 路径（`mcp_tool` / pool 握手 timeout）依赖 `stop_on_hub` → 同一缺口。

**对齐方案（不碰协议读写）**：
1. **spawn（client.rs）**：unix `process_group(0)`（tokio Command 的 unix extension）；spawn 后 `bind_child_to_parent(child.id().unwrap_or(0))`。
2. **杀干净**：抽出与 custom 同语义的组杀（client 内私有 helper，或复用同模式代码；**不**改 `proc_lifetime`、**不**抽跨 crate 大重构）：
   - Unix：`kill(-pid, SIGKILL)` + `child.start_kill()`/`wait` 回收；
   - Windows：`taskkill /PID /T /F`（与 custom/pty 一致）再 `start_kill`/`kill` 兜底。
   - `kill()`、`start_kill()`、`Drop` 均走该路径。
3. **pool.rs**：不改调度/锁模型；确认 `stop_inner` / `kill_child` / Drop / 超时后 `stop_on_hub` 落到新 kill。必要时仅加注释或极小转发，不改协议。
4. **mod.rs**：若需对外签名不变则不动；仅当枚举转发必须时改一行。
5. **明确不做**：`send_request` 无超时/错号（ISSUE-034）——只在产物注明「生命周期与协议分离」。

### 4 与现有 helper 的关系
| Helper | 归属 | 本包用法 |
|---|---|---|
| `proc_lifetime::bind_child_to_parent` | IDE | custom + MCP spawn 后调用；不改实现 |
| `kill_child_tree`（custom 私有） | Tools | 保留；Finished/超时/取消/WaitFailed 调用；MCP 侧平行实现同语义（tokio Child vs std Child，不硬共用） |
| `wait_child_cancellable` + deadline | Tools | 补 recv_timeout / Finished 组杀 |
| terminal/pty 组杀 | IDE | 语义对照，不改 pty |

### 5 测试（批后写；先 custom 测绿再动 MCP）
**custom.rs（ISSUE-039/040）**：
1. `timeout_returns_while_grandchild_holds_stdout`：`sh -c 'sleep 30 & echo done'`，timeout=2 → 在约 deadline+裕量内返回 TimedOut/或 Finished（echo 先出则 Finished+组杀），且 `pgrep`/临时 marker 证明 sleep 被杀（扩展现有 `timeout_kills_grandchild_process`）。
2. `finished_kills_orphaned_grandchild`：子进程先退出、孙进程仍跑 → 调用返回后孙进程不在。
3. `finished_stdin_writer_join_bounded`（可选，unix）：大 stdin + 孙进程继承不读 → 不 hang（依赖组杀 EPIPE）。
4. bind：编译期保证调用点存在；Windows 行为依赖 Job，本 box（Linux）以调用点+注释为准，不伪测 no-op。

**mcp（client / 必要时 pool）**：
5. spawn 后进程在组内：timeout/kill 后无残留孙进程（stdio 假 server：`sh -c 'sleep 30 & …'` 或最小可执行）。
6. `kill()` / Drop 杀树：子+孙均退出。
7. 既有 mcp/pool 测回归全过；不改协议测断言。

实跑：`cargo test`（box rustc 1.99.0，**勿**再前缀 `PATH=/home/box/.cargo/bin`）；共享 target 排队；尽量只跑本域 `tools::custom` / `mcp::`。

### 6 交叉审（与 IDE 基座，模式同 WP-027）
- **交 IDE 审**：custom 有界 recv + Finished 组杀 + `bind_child_to_parent`；MCP spawn 组+Job + 组杀 kill/Drop；是否与 pty/LSP 生命周期口径一致。
- **审 IDE**：对照 `WP-022-ide.md` / pty / `proc_lifetime` 文档；确认我们只调用不改 `proc_lifetime`；若 IDE 在 WP-040 改 workspace，不审其 037/038 文件竞态（非本包）。
- 双方在对方产物末尾附「交叉审」段；IDE 未回则 deliver 前标「待补」。

### 7 工具需求
- **无额外工具**。rustc 1.99.0 已就绪；`cargo test` / `rustc -V` 直接用。
- 若共享 target 锁长时间占满，排队等待，不改并行策略。

### 8 禁区与验收
- 禁：前端、引擎产品、symlink 产品行为、`take_changes`、改 `proc_lifetime.rs`、修 ISSUE-034、commit/push。
- done-when 预期：ISSUE-039/040 关闭或证据闭合；custom→mcp 串行改完；上述回归测绿；交叉审段落写入本产物；不 commit。

### 9 执行顺序（仅批示 start 后）
1. custom.rs：有界 drain + Finished 组杀 + bind → 单测绿。
2. mcp/client.rs：spawn 组+bind + 组杀 kill/Drop → 单测绿。
3. mcp/pool.rs（±mod.rs）：确认委托路径，必要时最小改动。
4. 本域回归 + 写交叉审 + 私信中层 → deliver WP-038。

---
## 执行记录（WP-038 start 后，2026-10-12 19:44–20:xx HKT，rustc 1.99.0，未 commit）

### A. custom.rs（先做，ISSUE-039/040）
1. **ISSUE-040**：spawn 成功后调用 `crate::proc_lifetime::bind_child_to_parent(child.id())`（只调用，未改 `proc_lifetime.rs`）。
2. **ISSUE-039 X1/X2**：`Ok(Some(status))` / wait 兜底不再无界 `rx.recv()`；经 `finish_after_child_exit`：**先组杀**（关闭孙进程持有的管道）再 `drain_output_bounded`（deadline+500ms grace，可响应 cancel）。
3. **ISSUE-039 X3**：Finished 路径 stdin writer 改为 `join_writer_bounded(..., 2s)`，超时放弃 join（组杀后应 EPIPE）。
4. Cancelled/TimedOut 路径仍先 `kill_child_tree` 再 `child.wait()`（原语义保留）。

**新增测（unix）**：
- `finished_reaps_grandchild_holding_stdout`：`sleep 30 & echo …`，timeout=2，<5s 返回且拿到 stdout。
- `finished_kills_orphaned_grandchild_before_side_effect`：Finished 后孙进程不得 touch marker。

**实跑**：`cargo test --lib tools::custom` → **23 passed / 0 failed**（含上述新测与既有 timeout/EPIPE/B5/B6）。

### B. mcp（custom 绿后）
1. **client.rs spawn**：unix `process_group(0)`；spawn 后 `bind_child_to_parent(child.id().unwrap_or(0))`。
2. **杀树**：新增私有 `kill_process_tree`（unix `kill(-pid)` / Windows `taskkill /T /F`）；`kill` / `start_kill` / `Drop` 均先组杀再 tokio kill。
3. **pool.rs**：`kill_child` 仍委托 `client.kill()`（现已杀树）；仅补文档注释。`mod.rs` 未改。
4. **ISSUE-034** 未碰（协议/错号/无超时 read_line）。

**新增测（unix）**：
- `mcp::client::tests::kill_reaps_grandchild_process`
- `mcp::client::tests::drop_reaps_grandchild_process`

**实跑**：`cargo test --lib mcp::` → **6 passed / 0 failed**（含 2 条新生命周期测）。

### C. 交叉审（IDE WP-036-ide.md / WP-040 已出，2026-10-12 19:5x HKT）

#### C1. Tools → IDE（审 WP-040 工作区改动；本包无新子进程）
| # | 位置/主题 | 判定 | 说明 |
|---|---|---|---|
| I1 | `file_ops::atomic_create(_bytes)`：tmp + `hard_link` 排他，AlreadyExists 不覆盖；硬链不可用回退 `create_new` | **接受** | 消除先查后写竞态，契约正确；与 symlink 禁区无关（未走 copy_recursive） |
| I2 | `service::create_file` / `write_file_bytes(overwrite=false)` 改走 atomic_create* | **接受** | 映射 `WorkspaceError::AlreadyExists` 清晰；overwrite=true 仍 atomic_write 合理 |
| I3 | `rename_path`：`replace_file` EXDEV → `move_via_copy`；失败且目标原先存在则不 `cleanup_incomplete_dest` | **接受** | ISSUE-038 口径正确，避免误删原目标 |
| I4 | 探针 `wp028_probe_s2_create_race` / `s1_xdev_move` 去 ignore 作正式回归 | **接受** | 关闭条件可核对 |
| I5 | 禁区：symlink / engines / take_changes / 前端 | **认可** | 产物声明与 diff 范围一致（本审未重跑 IDE 测，采信其绿表） |
| I6 | 生命周期 | **无异议** | IDE 本包无新增子进程；`proc_lifetime` 未改。Tools 侧 WP-038 仅**调用** `bind_child_to_parent`，符合 IDE 模块归属 |

无新 issue。不要求 IDE 改代码。

#### C2. 回应 IDE 对 Tools 的备注（沿用 WP-028 X1/X2/X4）
IDE 写「沿用 WP-028 对 custom.rs 的 X1/X2/X4；本包无新发现」。回应：
- **X1+X2**（孙进程持管道时超时失效 / Finished 不组杀）→ 已在本包修入 ISSUE-039：`finish_after_child_exit` 先组杀再有界 drain；测 `finished_reaps_grandchild_holding_stdout` / `finished_kills_orphaned_grandchild_before_side_effect` 绿。
- **X4**（未 `bind_child_to_parent`）→ 已修 ISSUE-040（custom + MCP spawn 后调用）；`proc_lifetime.rs` 未改。
- MCP 已对齐组/树杀（kill/Drop）；IDE 未对 MCP 提新点，无额外反驳。
- 请 IDE 若再审，以 **WP-038 后** 的 custom/mcp 为准，WP-028 原文作历史。

#### C3. 仍请 IDE 可选补一眼（不阻塞 deliver）
Finished 先组杀再 drain、MCP `kill_process_tree` 与 pty/LSP 组杀口径是否一致——有异议私信中层即可。

### D. 禁区核对
未改：frontend、引擎产品、symlink 产品行为、`take_changes`、`proc_lifetime.rs`、ISSUE-034。未 commit/push。
