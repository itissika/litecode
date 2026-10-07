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
- MCP：JSON-RPC 适配——`tools/list` 取原始名与 schema 装配成工具 [@ file="src/tool/registry.rs" symbol="fn instantiate_tool" lines="97-120"]，`tools/call` 执行 [@ file="src/mcp/client.rs" lines="124-161"]、[@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn call_inner"]。
- 边界：异步接口不等于构造过程非阻塞。Custom 把阻塞等待放进 `spawn_blocking` [@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn execute"]，MCP 默认 `execute` 在返回 future 前就同步做完工作，公共 timeout 覆盖不到（后果见问题节点）。

相关：[@ key="tool-implementation-contract"]、[@ key="tool-identity"]、[@ key="tool-mcp-blocking-timeout-issue"]。
