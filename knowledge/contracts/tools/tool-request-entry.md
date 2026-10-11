```node
node : tool-request-entry
status : pending
domain : tools-permission
summary : 依赖可用、agent 可见、执行查找三个不同问题，不能混用。
x : 2790
y : 396
w : 398
h : 531
```

**主张：** 一次调用要穿过三个不同问题——配置/依赖是否可用、当前 agent 是否可见、执行时是否找到实现。

- 可用：`available_tools` 汇总 core、引擎就绪的可选工具、custom 定义与 MCP 配置；MCP 只查配置存在，不代表进程已运行（[@ file="src/tool/availability.rs" symbol="fn available_tools"]、[@ file="src/tool/availability.rs" symbol="fn is_available"]）。
- 可见：`build_tool_list` 逐 id 过 `should_include_in_llm_list`（要求该 agent 绑定 enabled 且可用），再实例化——子 agent 深度门、MCP 需本轮握手成功才产出工具；`RuntimeContext::tool_defs` 把 name / description / schema 送进模型请求（[@ file="src/tool/availability.rs" symbol="fn should_include_in_llm_list"]、[@ file="src/tool/registry.rs" symbol="fn build_tool_list"]、[@ file="src/runtime/context.rs" symbol="impl RuntimeContext › fn tool_defs"]）。
- 快照：列表在该 agent 首次 `run_with_turn` 时惰性构建，之后固定（[@ file="src/runtime/mod.rs" symbol="impl AgentRuntime › fn run_with_turn"]）。
- 执行查找：按 name 在本轮工具快照内找实现，未命中回错误结果；查找发生在执行入口（见 [@ key="tool-call-lifecycle"]）。
- 边界：统一入口只表示调用从同一处进入、事实参数由管线注入，不等于授权，也不等于沙箱。

父节点：[@ key="tool-call-contract"]。
相关：[@ key="tool-call-lifecycle"]、[@ key="tool-identity"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 可用/可见/快照 —— [@ file="src/tool/availability.rs" symbol="fn available_tools"]、[@ file="src/tool/availability.rs" symbol="fn should_include_in_llm_list"]、[@ file="src/tool/registry.rs" symbol="fn build_tool_list"]。
- MCP is_available 只查配置键 —— 与「≠ Warm」主张一致。
