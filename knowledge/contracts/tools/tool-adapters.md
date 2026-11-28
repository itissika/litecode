```node
node : tool-adapters
status : enabled
summary : 三种来源共用一个工具契约：内置、Custom、MCP 的差异只在适配层，不在契约本身。
x : 2394
y : 702
w : 380
h : 500
domain : tools-permission
```

**解决什么：** 工具来源有三种——内置、Custom 子进程、MCP 服务——但对模型和执行器必须是同一种东西：同一个名字、同一份 schema、同一种结果。差异只能留在各自的适配层，否则每加一种来源，管线就要多一套分支。

**必须做到：** 三种来源都实现同一个 `Tool`：`name` 声明身份，schema 声明输入，`execute` 返回 `ToolCallResult`；协议怎么谈，是各家自己的事。[Tool.execute](src/tool/trait_.rs#Tool.execute)

- **内置：** 进程内 Rust 实现，直接用执行上下文（工作区、会话、取消）；`exec_command` 会起 PTY 子进程，那是它的业务实现，不是协议适配。
- **Custom：** 子进程协议。参数 JSON 写 stdin，stdout 弹性解析——对象、数组、纯文本都收，对象里可以带 `level` 和 `media`；非零退出先看 `{"error":{code,message,retryable,details}}` 信封。[result_from_stdout](src/tools/custom.rs#result_from_stdout)
- **MCP：** JSON-RPC 协议。`execute` 在 future 里 await 连接池，不在构造期阻塞；阻塞等待属于同步路径，不能放进 `execute` 的返回之前。[McpTool.execute](src/tools/mcp_tool.rs#McpTool.Tool.execute)

**绝对不允许：**

- 为某一种来源在管线里加分支——分支只许出现在适配层。
- 在返回 future 之前把活干完：外层 timeout 会包住一个已经完成的 future，等于没有超时。
- 让适配层决定结果语法；前缀、截断、落盘、媒体物化都归出口。

上游：[tool-implementation-contract](knowledge/contracts/tools/tool-implementation-contract.md)。相关：[tool-identity](knowledge/contracts/tools/tool-identity.md)、[tool-output-shaping](knowledge/contracts/tools/tool-output-shaping.md)、[mcp-connection-pool](knowledge/contracts/tools/mcp-connection-pool.md)。
