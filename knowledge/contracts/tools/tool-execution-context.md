```node
node : tool-execution-context
status : pending
summary : 执行事实一次性装配成 ToolExecutionContext；提供上下文不等于每个工具都消费它。
x : 2124
y : 378
w : 480
h : 600
domain : tools-permission
```

**主张：** executor 在调用前把一次调用的执行事实装配成 `ToolExecutionContext`，与模型给的 JSON 分离；管线提供上下文，是否消费由具体工具决定。

- 字段：`path_mode`、`workspace_root`、`call_id`、`cancel`、`output_limit`、`session_id`、`session` [ToolExecutionContext](src/tool/trait_.rs#ToolExecutionContext)；未注入 session 时 `session_reader()` 返回错误，不静默回退 [ToolExecutionContext.session_reader](src/tool/trait_.rs#ToolExecutionContext.session_reader)。
- 装配点在执行入口：`path_mode` 取自 `permission.path_mode(工具名)`，`output_limit = tool.max_result_size()`，`cancel` 与 turn 共享（入口生命周期见 [tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)）。
- 消费逐工具而异，不是统一消费：read 用 `workspace_root`/`path_mode` 解析路径 [ReadTool.call_for_execution](src/tools/read.rs#ReadTool.call_for_execution)；bash 取 `cancel`/`session_id`/`call_id`/`workspace_root` [RunTool.Tool.execute](src/tools/run.rs#RunTool.Tool.execute)。
- Custom / MCP 是否消费上下文由各适配决定，见 [tool-adapters](knowledge/contracts/tools/tool-adapters.md)；本节点只钉上下文装配本身。
- `is_cancellable` 默认 false、部分工具覆写为 true；该标记不参与分流，取消令牌与 join 行为见 [tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)。

父节点：[tool-control-boundary](knowledge/contracts/tool-control-boundary.md)。
相关：[tool-control-boundary](knowledge/contracts/tool-control-boundary.md)、[tool-scheduling](knowledge/contracts/tools/tool-scheduling.md)、[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)。
