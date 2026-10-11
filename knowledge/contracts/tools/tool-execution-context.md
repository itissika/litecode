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

- 字段：`path_mode`、`workspace_root`、`call_id`、`cancel`、`output_limit`、`session_id`、`session` [@ file="src/tool/trait_.rs" symbol="struct ToolExecutionContext"]；未注入 session 时 `session_reader()` 返回错误，不静默回退 [@ file="src/tool/trait_.rs" symbol="impl ToolExecutionContext › fn session_reader"]。
- 装配点在执行入口：`path_mode` 取自 `permission.path_mode(工具名)`，`output_limit = tool.max_result_size()`，`cancel` 与 turn 共享（入口生命周期见 [@ key="tool-call-lifecycle"]）。
- 消费逐工具而异，不是统一消费：read 用 `workspace_root`/`path_mode` 解析路径 [@ file="src/tools/read.rs" symbol="impl ReadTool › fn call_for_execution"]；bash 取 `cancel`/`session_id`/`call_id`/`workspace_root` [@ file="src/tools/bash.rs" symbol="impl Tool for BashTool › fn execute"]。
- Custom / MCP 是否消费上下文由各适配决定，见 [@ key="tool-adapters"]；本节点只钉上下文装配本身。
- `is_cancellable` 默认 false、部分工具覆写为 true；该标记不参与分流，取消令牌与 join 行为见 [@ key="tool-call-lifecycle"]。

父节点：[@ key="tool-control-boundary"]。
相关：[@ key="tool-control-boundary"]、[@ key="tool-scheduling"]、[@ key="tool-call-lifecycle"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 结构体字段与 session_reader 硬失败 —— [@ file="src/tool/trait_.rs" symbol="struct ToolExecutionContext"]、[@ file="src/tool/trait_.rs" symbol="impl ToolExecutionContext › fn session_reader"]。
- 装配在 run_tool；消费逐工具（read/bash）—— 与主张「提供≠消费」一致。
