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

- 共同面：`name` + `schema` 声明输入，`execute` 返回 `ToolCallResult`，`timeout()` 给可选上限 [Tool.execute](src/tool/trait_.rs#Tool.execute)、[Tool.timeout](src/tool/trait_.rs#Tool.timeout)。公共 finalize/截断见 [tool-output-shaping](knowledge/contracts/tools/tool-output-shaping.md)；调用生命周期见 [tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)。
- 内置：进程内 Rust 实现，不经过 Custom/MCP 那套外部工具协议适配；bash 自身仍会起 PTY 子进程，那是业务实现 [builtin_tool](src/tool/registry.rs#builtin_tool)、[spawn_inner](src/terminal/pty.rs#spawn_inner)，并按需消费 `ToolExecutionContext`（工作区路径、会话、取消等）[trait_.rs:13-22](src/tool/trait_.rs#L13-L22)。
- Custom：子进程适配——参数 JSON 写 stdin；stdout 弹性解析（JSON 对象/数组/纯文本，以及可选 `level`/`media` 字段；非零退出优先解析 `{"error":{"code","message",...}}` 信封） [CustomTool.Tool.call_inner](src/tools/custom.rs#CustomTool.Tool.call_inner)、[result_from_stdout](src/tools/custom.rs#result_from_stdout)。
- MCP：JSON-RPC 适配——覆写 `execute` 在 future 内 await 连接池 [McpTool.Tool.execute](src/tools/mcp_tool.rs#McpTool.Tool.execute)；agent 面命名与装配见 [tool-identity](knowledge/contracts/tools/tool-identity.md)，连接/call 出口见 [mcp-connection-pool](knowledge/contracts/tools/mcp-connection-pool.md)。
- 边界：异步接口不保证构造期非阻塞——在返回 future 前同步完成工作，会让后包的公共 timeout 包住已完成 future 而失效。Custom 把阻塞等待放进 `spawn_blocking` [CustomTool.Tool.execute](src/tools/custom.rs#CustomTool.Tool.execute)；MCP 的 await 点见上条。

父节点：[tool-implementation-contract](knowledge/contracts/tool-implementation-contract.md)。
相关：[tool-implementation-contract](knowledge/contracts/tool-implementation-contract.md)、[tool-identity](knowledge/contracts/tools/tool-identity.md)、[tool-output-shaping](knowledge/contracts/tools/tool-output-shaping.md)、[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)。
契约展开：[mcp-connection-pool](knowledge/contracts/tools/mcp-connection-pool.md)。
