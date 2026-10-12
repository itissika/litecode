## WP-055 Tools 方案（tools，direct）— ISSUE-034 MCP stdio 超时 + id 匹配
父包 WP-053（in_progress）← WP-052；产物本文件 `.litecode/ops/artifacts/WP-052-tools.md`。
状态：**方案填写，未 start、未改代码、不 commit**。等 backend_mid 批 plan 后再 start。

### 0 目标与范围
- **目标**：修 ISSUE-034——MCP stdio `send_request`：(1) 每请求有界超时；(2) 按 JSON-RPC `id` 匹配响应，跳过 notification / 无 id 行；(3) 取消或超时后不得让残留响应错号给下一次请求（连接标脏重建，优先于盲排空）。
- **主改**：`src/mcp/client.rs`（协议读写核心）、`src/mcp/pool.rs`（持锁贯穿调用、超时/脏连接重建）。必要时 `src/mcp/mod.rs` 仅转发签名，不扩面。
- **可复用**：WP-038 已落地的组杀 / `kill_child` / `stop_on_hub` / `bind_child_to_parent`（生命周期层）；本包只补**协议层**，不重做杀树。
- **禁区**：引擎产品、symlink 产品行为、前端/桌面、ISSUE-032（`take_changes`）、不改 `proc_lifetime.rs`、不 commit/push。

### 1 现状证据（只读核对，与 ISSUE-034 一致）
| 点 | 位置 | 问题 |
|---|---|---|
| 无超时读 | `client.rs` `send_request` ~:81-85：`write+flush` 后无界 `read_line` | 服务器不回 → 永久挂起 |
| 不按 id | 同函数 ~:92-103：解析下一行即当响应，无 `response.id == request_id` | notification / 日志行 → Null/错结果；真响应残留 → 错号 |
| 锁贯穿 | `pool.rs` ~:285-289：`client.lock().await` 后 `initialize`/`call_tool` 无额外 timeout | 一次挂死 → 同 server 后续调用全堵在 Mutex |
| 外层超时不全 | `mcp_tool.rs` 已有 `tokio::time::timeout` + 超时 `stop_on_hub` | 仅覆盖工具调用路径；`send_request` 自身仍无界；**turn cancel** 若只 drop future、未标脏/重建，BufReader 残留仍致错号 |
| 握手有界 | `pool.rs` ~:404 `START_TIMEOUT` 仅握手 | 与每次 `tools/call` 无关 |

复现口径（批后用假 server 固化为测）：
1. tools/call 先发 `notifications/message` 再发真响应 → 现实现当 Null/错；真响应留给下一请求。
2. tools/call 不回复 → 挂死 + pool 锁堵死同 server。
3. tools/call 延迟回复期间取消回合 → 下一 `tools/list` 读到上一 call 的响应（错号）。

### 2 修复设计（批后实施）

#### 2.1 按 id 循环读（client.rs）
改写 `send_request`：
1. 分配 `id`，写出请求（保持现有 AtomicU64）。
2. **循环** `read_line`：
   - 空读（EOF）→ 明确错误（server 关闭）。
   - JSON 解析失败 → 记日志/计次，可跳过或标脏（建议：连续非法行达阈值 → 标脏返回错误，避免死循环吞噪声）。
   - 无 `id` 字段（JSON-RPC notification）或 `method` 存在且无匹配 result/error → **跳过**，继续读。
   - 有 `id` 但 `id != request_id` → **跳过**（乱序/过期响应），继续读；可选 debug 计数。
   - `id == request_id` → 按现逻辑取 `error` / `result` 返回。
3. 不把 notification 的缺失 `result` 当成 `Null` 成功。

#### 2.2 每请求有界超时（client + pool）
- **client**：`send_request` 内对「整段读循环」包 `tokio::time::timeout(request_timeout)`（或每轮 `read_line` 用剩余 deadline）。超时 → 返回可识别错误（如 `ToolExecution("MCP request timed out…")`），并 **标记连接脏**。
- **超时值来源**（方案偏好，批时可裁）：
  - 默认常量（如 60s）与现有 `McpServerConnection.timeout_secs` / `mcp_tool` 外层超时对齐；
  - 优先让 `pool.call_tool` / `send_request` 接受可选 `Duration`，由 pool/mcp_tool 传入 `conn.timeout_secs`，避免「外层 60s + 内层更长」双超时打架；
  - 若签名改动面大：client 用保守默认 + pool 在 `lock` 期间再包一层与 `timeout_secs` 一致的 timeout，超时后 `kill_child` + 移除条目（与现 `mcp_tool` 超时杀进程一致）。
- **锁**：超时/错误路径必须释放 `Mutex`（自然 drop guard）；不得在无界 read 上持锁。

#### 2.3 取消/超时后防错号（脏连接重建，优先）
根因：请求已写出、响应未按 id 读完就被 cancel/timeout → 字节留在 `BufReader` → 下次 `read_line` 错号。

**首选策略：标脏 + 重建**（比盲排空更稳）：
1. `McpStdioClient` 增 `dirty: bool`（或 `session_epoch`）；timeout / cancel / 未匹配到 id 就返回前设脏。
2. pool：调用前后若 `dirty` 或超时/取消错误 → `kill_child`（复用 WP-038 组杀）+ 从 map 剔除 → 下次 `get_or_create` 新进程、空缓冲。
3. turn cancel：若调用 future 被 cancel，在 `mcp_tool` / pool 路径用 `CancellationToken` 或 `Drop` guard 确保「取消 ⇒ 标脏/停 hub」，不能只靠外层 timeout。
4. **不优先**「排空缓冲直到空闲」作为唯一手段（notification 可能持续到来；排空无终止条件）。可作重建前的 best-effort，但不替代杀进程重建。

#### 2.4 与现有 mcp_tool 外层超时的关系
- 保留 `mcp_tool` 的 timeout + `stop_on_hub`（已测 `execute_stays_pending_while_hang_blocks_on_hub`）。
- 本包补齐：**协议层自身有界 + id 匹配 + 脏重建**，使非 mcp_tool 入口（直接 `list_tools` / pool 内部）与 cancel 路径同样安全。
- 文档写清：外层超时负责「用户可见 deadline + 杀进程」；内层负责「不错号、不无界占锁」。

### 3 文件与串行
| 顺序 | 文件 | 改动要点 |
|---|---|---|
| 1 | `src/mcp/client.rs` | id 循环读；请求 timeout；dirty 标记；单测用假 server/脚本 |
| 2 | `src/mcp/pool.rs` | 传播 timeout；脏/超时/取消 → kill + 剔除重建；持锁时间仍跨一次 call 但 call 有界 |
| 3 | `src/mcp/mod.rs` | 仅当枚举转发需要；默认不动 |
| 4 | `tests/fixtures/mock_mcp_server.py` 或新 fixture | 模式：notify-first / hang / slow-then-reply（供 cancel 测） |
| 5 | `src/mcp/client.rs` 单测 + `tests/mcp_integration.rs` | 三条回归 + 既有 echo/hang 回归 |

不改：`custom.rs`、前端、runtime seam、`proc_lifetime`。

### 4 回归测试（假 MCP server，必过）
| # | 场景 | 期望 |
|---|---|---|
| R1 | notification 先行再真响应 | 返回真 `result`；下一请求仍正确（不错号） |
| R2 | 不回复 / hang | 在 timeout 内返回错误；同 server 后续调用可再建连成功（锁不永久堵死） |
| R3 | 慢响应期间取消（或模拟 timeout 后立刻下一请求） | 下一 `tools/list`/`echo` 不得收到上一 call 的响应；连接已重建或等效干净 |
| R4 | 既有 | `mcp::` 单测 + `mcp_integration` echo/hang/catalog 全绿 |

实现建议：扩展 `tests/fixtures/mock_mcp_server.py`（env 开关 `MCP_MOCK_MODE=notify_first|hang|slow`）或并列小脚本；client 单测可 `Command::new("python3").arg(fixture)`。

实跑（批后；共享 target 排队；直接 `cargo`，rustc 1.99.0）：
```
cargo test --lib mcp::
cargo test --test mcp_integration
```

### 5 交叉审
- **Tools ↔ IDE（只读）**：确认本包不改生命周期假设（WP-038 组杀/bind 仍成立）；协议超时触发的 `kill_child` 与 IDE `proc_lifetime` 无冲突。要点写入本产物「交叉审」节；IDE 未回标「待补」，不挡 deliver。
- **不审** Runtime ISSUE-032 / 前端。

### 6 工具需求
- **无新增工具**。rustc 1.99.0；`cargo test` 直接用；假 server 依赖系统 `python3`（已有 mock）。
- 共享 target 锁：排队，只跑 `mcp::` / `mcp_integration`。

### 7 禁区与验收（done-when）
- 禁：引擎产品、symlink、前端、ISSUE-032、commit/push、改 `proc_lifetime` 实现。
- done-when：
  1. `send_request` 有界超时 + 按 id 匹配 + 跳过 notification；
  2. 取消/超时后无错号（脏重建证据）；
  3. R1–R3 回归绿 + 既有 mcp 测绿；
  4. ISSUE-034 → closed（或经中层/GM 的 blocked 说明）；
  5. 产物本文件写全证据；交叉审节齐；不 commit。

### 8 执行顺序（仅批准 start 后）
1. `wp.mjs start --id WP-055 --as tools`
2. client.rs：id 循环 + timeout + dirty → 单测 R1/R2
3. pool.rs：超时/脏重建 + cancel 路径 → R3
4. 扩展 fixture + mcp_integration 回归
5. 写交叉审 → 更新 ISSUE-034 history → deliver WP-055 给 backend_mid

### 9 ISSUE 状态备忘
| ID | status | 本包 |
|---|---|---|
| ISSUE-034 | open | **本包修** |
| ISSUE-039/040 | closed | 不重做；只复用 kill |
| ISSUE-032 | （Runtime WP-054） | 禁区 |


---
## 执行记录（WP-055 start 后，2026-10-12T20:49:47+08:00，rustc 1.99.0，未 commit）

### A. client.rs（ISSUE-034 协议层）
1. **按 id 匹配**：`read_matching_response` 循环 `read_line`；无 id / null id（notification）跳过；错 id 跳过；匹配后取 error/result。
2. **有界超时**：`send_request` 对读循环包 `tokio::time::timeout(request_timeout)`；默认 `DEFAULT_MCP_TOOL_TIMEOUT_SECS`（60s）；`set_request_timeout` 可覆盖。
3. **防错号**：写成功后 `DirtyOnDrop` 武装——超时/错误/cancel drop future → `dirty=true`；脏连接拒绝再发请求。
4. **不盲排空**：明确不做 drain；重建靠 kill+respawn。

### B. pool.rs + mcp_tool.rs
1. `call_on_hub(..., timeout_secs)`：调用前 `set_request_timeout`；失败且 dirty → `stop_inner`（组杀+剔除）。
2. `start_inner` Running：`alive && !dirty` 才复用，否则 stop 重建。
3. `mcp_tool::hub_call` 把 `timeout_secs` 传入 `call_on_hub`（内外超时对齐）。

### C. 假 server
`tests/fixtures/mock_mcp_server.py`：`MCP_MOCK_MODE=notify_first|slow`（+ `MCP_MOCK_SLOW_SECS`）。

### D. 回归
| 测 | 结果 |
|---|---|
| `mcp::client::tests::r1_skips_notification_before_response` | ok |
| `mcp::client::tests::r2_hang_times_out_and_marks_dirty` | ok |
| `mcp::client::tests::r3_cancel_marks_dirty_next_client_ok` | ok（内层作用域修 E0502/E0505） |
| `cargo test --lib mcp::` | **9 passed** |
| `cargo test --test mcp_integration` | **10 passed** |
| `cargo test --lib --no-run` | **compile ok**（恢复全库可编） |

### E. 交叉审
- Tools ↔ IDE：本包只动协议层；kill 路径仍走 WP-038 组杀/`kill_child`；未改 `proc_lifetime`。标待补（不挡 deliver）。

### F. 禁区确认
未碰引擎产品、symlink、前端、ISSUE-032；未 commit。

### G. ISSUE-034
已标 `closed`（tools）；证据见上。
