```node
node : tool-execution-context
status : pending
summary : 执行事实一次性装配成 ToolExecutionContext；提供上下文不等于每个工具都消费它。
x : 2322
y : 396
w : 480
h : 600
```

**主张：** executor 在调用前把一次调用的执行事实装配成 `ToolExecutionContext`，与模型给的 JSON 分离；管线提供上下文，是否消费由具体工具决定。

- 字段：`path_mode`、`workspace_root`、`call_id`、`cancel`、`output_limit`、`session_id`、`session` [@ file="src/tool/trait_.rs" symbol="struct ToolExecutionContext"]；未注入 session 时 `session_reader()` 返回错误，不静默回退 [@ file="src/tool/trait_.rs" symbol="impl ToolExecutionContext › fn session_reader" lines="24-29"]。
- 装配点是 `fn run_tool`：`path_mode` 取自 `permission.path_mode(工具名)`，`output_limit = tool.max_result_size()`，`cancel` 与 turn 共享 [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="377-385"]。
- 消费逐工具而异，不是统一消费：read 用 `workspace_root`/`path_mode` 解析路径 [@ file="src/tools/read.rs" symbol="impl ReadTool › fn call_for_execution"]；bash 取 `cancel`/`session_id`/`call_id`/`workspace_root` [@ file="src/tools/bash.rs" symbol="impl Tool for BashTool › fn execute"]。
- CustomTool 忽略上下文（`_execution`）[@ file="src/tools/custom.rs" symbol="impl Tool for CustomTool › fn execute"]；McpTool 同样不消费上下文，但已覆写 `execute`、在返回的 future 内 await hub [@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn execute" lines="60-75"]。
- `is_cancellable` 默认 false、部分工具覆写为 true；当前取消路径按 `cancel` 令牌与 join 等待结果处理，该标记不参与分流 [@ file="src/tool/trait_.rs" symbol="trait Tool › fn is_cancellable"]、[@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn execute_batch_cancellable" lines="123-131"]、[@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn join_tool_handle" lines="245-267"]。

相关：[@ key="tool-control-boundary"]、[@ key="tool-scheduling"]。
