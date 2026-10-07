```node
node : tool-mcp-blocking-timeout-issue
status : pending
summary : [问题] MCP 默认 execute 在返回 future 前就同步跑完 call_inner 并阻塞在 hub 等待上，执行器的公共 timeout 包不到这段阻塞。
x : 4662
y : 396
w : 480
h : 512
```

**问题：** McpTool 不覆写 `execute`，走默认链 `execute → call_async → call → call_inner`；`call_async` 在返回 future 之前就同步执行 `call`，而 `call_inner` 用 `pool.block_on_hub` 同步等 hub 结果。`run_tool` 先调用 `execute` 构造 `tool_fut`，之后才包 `tokio::time::timeout`——阻塞发生在构造期，包裹在后的公共 timeout 无法中断它。

**触发条件**
1. turn 内调用任一 MCP 工具（McpTool 恒非并发安全 → 串行批）；
2. 服务器卡住不回包，或调用耗时接近内层 timeout；
3. turn 跑在 `spawn_turn` 的 current_thread runtime 上，其唯一线程被 `orx.recv()` 占用。

**实际后果**
- 公共 timeout 对 MCP 实际无效：`timeout()` 返回配置值 + 15，但包住的是一个已完成 future，不会触发 [@ file="src/tools/mcp_tool.rs" lines="102-104"]。
- 执行期间 turn 线程阻塞：取消令牌可被设置，但处理要等 hub 返回——串行批在调用之间才检查取消 [@ file="src/tool/pipeline.rs" lines="166-193"]，`run_tool` 只在执行前三处检查 [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="299-372"]。不涉及其它 session、hub 线程或整个进程。
- 真正的限时来自 MCP 内层 timeout（默认 60s）；超时后还要 `await stop_on_hub` 收尾，所以不是严格固定的时限，而是「内层 timeout + 收尾耗时」。

**证据链**
1 默认链：McpTool 只实现 name/schema/call_inner/timeout/description/is_concurrency_safe [@ file="src/tools/mcp_tool.rs" lines="45-113"]；默认 [@ file="src/tool/trait_.rs" symbol="trait Tool › fn execute" lines="62-69"]，而 call_async 先求值再返回 ready future [@ file="src/tool/trait_.rs" symbol="trait Tool › fn call_async" lines="54-60"]。
2 同步阻塞：[@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn call_inner" lines="71-96"] → [@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn block_on_hub" lines="141-145"]（另起线程 + 调用方 `orx.recv()`）。
3 包裹顺序：先构造 future、后包 timeout [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="386-399"]。
4 内层 timeout 与收尾 [@ file="src/tools/mcp_tool.rs" lines="73-95"]、[@ file="src/mcp/pool.rs" symbol="impl McpConnectionPool › fn stop_on_hub" lines="292-294"]；默认值与配置 [@ file="src/config/schema.rs" symbol="impl McpServerDefinition › fn call_timeout_secs" lines="318-326"]、[@ file="src/config/schema.rs" lines="279-280"]。
5 线程与取消：turn 专用线程 + current_thread runtime [@ file="src/runtime/mod.rs" symbol="fn spawn_turn" lines="470-476"]；串行批的检查点 [@ file="src/tool/pipeline.rs" lines="166-193"]；执行前的三处检查 [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="299-372"]。

**核证程度：** 源码静态核证，未动态复现（未启动 MCP、未跑测试、未改配置）。既有单测演示的是「把阻塞放进 spawn_blocking 才可被 timeout 打断」的修法，不覆盖 MCP 默认路径 [@ file="src/tool/executor.rs" lines="639-741"]。

**关联：** [@ key="tool-adapters"]、[@ key="tool-execution-context"]、[@ key="tool-identity"]。
