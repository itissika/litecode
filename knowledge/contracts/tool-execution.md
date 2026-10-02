```node
node : tool-execution
status : pending
summary : 工具执行：模型参数与执行事实分离；按资源冲突调度；结果按 call_id 配对、按调用顺序落地。
x : 1224
y : 530
w : 398
h : 720
```

**工具实现可以不同，生产执行入口、授权和输出契约不能绕过。**

### 数据
- `FunctionToolCall` 提供 name、arguments 与 call_id；`Tool` 提供 schema、execute、并发安全声明和资源键。
- `ToolExecutionContext` 的 session、workspace、path_mode、cancel 等由 executor 注入，不由模型 JSON 决定。
- `ToolCallResult` 经统一整形成为对应 call_id 的 FunctionCallOutput。

### 流转
按声明／资源键拆批 → 参数校验与授权 → 资源锁 → execute → 信号／截断／大结果落盘 → 按调用顺序构造 outputs → 写会话日志。

### 不变量
- 同批只并发执行声明 safe 且资源不冲突的调用；不是“读”这个名字就自动安全，串行批构成调度屏障。
- 同 step 的 resource_keys 控制冲突；跨 session 的互斥当前只覆盖 write/edit，失败返回 resource busy，不保证任意 bash 写操作被同一把锁兜住。
- 模型产出的 FunctionCall 已在日志中，工具管线只产生结果，不从字符串合成第二份消息真相。
- 按 call_id 配对、按 invocation 顺序输出；每个调用都要有结果，取消时缺失者补中断结果。
- 取消后等待在途 handle 收尾，不 abort-and-forget；工具是否真能停止底层工作取决于其取消能力，不能把超时包装当副作用回滚。
- 已返回的结果保留为事实，不能被晚到的取消覆盖；panic 转工具错误，但错误也不保证未发生外部副作用。

依据：[@ file="src/tool/trait_.rs" symbol="struct ToolExecutionContext" label="执行事实"]、[@ file="src/tool/executor.rs" symbol="fn partition_tool_calls" label="批次与资源"]、[@ file="src/tool/executor.rs" symbol="fn run_tool" label="生产入口"]、[@ file="src/tool/executor.rs" symbol="fn outputs_from_tool_results" label="结果配对"]、[@ file="src/tool/pipeline.rs" label="取消收尾与输出"]。
验证：[@ file="src/tool/executor.rs" symbol="mod tests › fn completed_tool_result_wins_over_late_cancellation" label="晚取消保留结果"]。

上层：[@ id="tools" label="tools"]。
