```node
node : tool-adapters
status : pending
summary : 三种接入共用同一 Tool 输入/结果契约；差异在各家的协议适配与业务实现，不在契约本身。
x : 2394
y : 702
w : 380
h : 500
domain : tools-permission
```

**主张：** 内置、Custom、MCP 共用同一 `Tool` 输入/结果契约；差异是各家的协议适配与业务实现，而不是契约。

- 共同面：`name` + `schema` 声明输入，`execute` 返回 `ToolCallResult`，`timeout()` 给可选上限 [@ file="src/tool/trait_.rs" symbol="trait Tool › fn execute"]、[@ file="src/tool/trait_.rs" symbol="trait Tool › fn timeout"]。公共 finalize/截断见 [@ key="tool-output-shaping"]；调用生命周期见 [@ key="tool-call-lifecycle"]。
- 内置：进程内 Rust 实现，不经过 Custom/MCP 那套外部工具协议适配；bash 自身仍会起 PTY 子进程，那是业务实现 [@ file="src/tool/registry.rs" symbol="fn builtin_tool"]、[@ file="src/terminal/pty.rs" symbol="fn spawn_inner"]，并按需消费 `ToolExecutionContext`（工作区路径、会话、取消等）[@ file="src/tool/trait_.rs" lines="13-22"]。
- Custom：子进程适配——参数 JSON 写 stdin，stdout 文本直取；带 `media`/`level` 的 JSON 走信封解析，畸形信封硬失败 [@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn call_inner"]、[@ file="src/tools/custom.rs" symbol="fn result_from_stdout"]。
- MCP：JSON-RPC 适配——覆写 `execute` 在 future 内 await 连接池 [@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn execute"]；agent 面命名与装配见 [@ key="tool-identity"]，连接/call 出口见 [@ key="mcp-connection-pool"]。
- 边界：异步接口不保证构造期非阻塞——在返回 future 前同步完成工作，会让后包的公共 timeout 包住已完成 future 而失效。Custom 把阻塞等待放进 `spawn_blocking` [@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn execute"]；MCP 的 await 点见上条。

父节点：[@ key="tool-implementation-contract"]。
相关：[@ key="tool-implementation-contract"]、[@ key="tool-identity"]、[@ key="tool-output-shaping"]、[@ key="tool-call-lifecycle"]。
契约展开：[@ key="mcp-connection-pool"]、[@ key="custom-tool-hub"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 共同 Tool 面 —— [@ file="src/tool/trait_.rs" symbol="trait Tool › fn execute"]。
- Custom spawn_blocking + 信封 —— [@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn execute"]、[@ file="src/tools/custom.rs" symbol="fn result_from_stdout"]。
- MCP await 池 —— [@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn execute"]；连接出口见 [@ key="mcp-connection-pool"]。
