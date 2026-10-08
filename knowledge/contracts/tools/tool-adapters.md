```node
node : tool-adapters
status : pending
summary : 三种接入共用同一 Tool 输入/结果契约；差异在各家的协议适配与业务实现，不在契约本身。
x : 4194
y : 396
w : 380
h : 500
```

**主张：** 内置、Custom、MCP 共用同一 `Tool` 输入/结果契约；差异是各家的协议适配与业务实现，而不是契约。

- 共同面：`name` + `schema` 声明输入，`execute` 返回 `ToolCallResult`，`timeout()` 给可选上限 [@ file="src/tool/trait_.rs" symbol="trait Tool › fn execute" lines="32-69"]、[@ file="src/tool/trait_.rs" symbol="trait Tool › fn timeout" lines="73-75"]，结果由 `run_tool` 统一 finalize/截断/输出处理 [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="413-429"]。
- 内置：进程内 Rust 实现，不经过 Custom/MCP 那套外部工具协议适配；bash 自身仍会起 PTY 子进程，那是业务实现 [@ file="src/tool/registry.rs" symbol="fn builtin_tool"]、[@ file="src/terminal/pty.rs" symbol="fn spawn_inner" lines="308-320"]，并按需消费 `ToolExecutionContext`（工作区路径、会话、取消等）[@ file="src/tool/trait_.rs" lines="13-22"]。
- Custom：子进程适配——参数 JSON 写 stdin，stdout 文本直取；带 `media`/`level` 的 JSON 走信封解析，畸形信封硬失败 [@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn call_inner"]、[@ file="src/tools/custom.rs" symbol="fn result_from_stdout"]。
- MCP：JSON-RPC 适配——`tools/list` 取 schema 与原始名装配；agent 面名加 `mcp_{server_id}_` 前缀、原始名只用于 `tools/call` [@ file="src/tools/mcp_tool.rs" symbol="impl McpTool › fn new" lines="33-48"]、[@ file="src/tool/registry.rs" symbol="fn instantiate_tool" lines="97-120"]，经 hub 执行 [@ file="src/tools/mcp_tool.rs" symbol="fn hub_call" lines="98-140"]、[@ file="src/mcp/client.rs" lines="124-161"]。
- 边界：异步接口不保证构造期非阻塞——在返回 future 前同步完成工作，会让后包的公共 timeout 包住已完成 future 而失效。Custom 把阻塞等待放进 `spawn_blocking` [@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn execute"]；MCP 据此改为覆写 `execute` 在 future 内 await hub [@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn execute" lines="60-75"]。

相关：[@ key="tool-implementation-contract"]、[@ key="tool-identity"]。
