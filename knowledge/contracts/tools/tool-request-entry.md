```node
node : tool-request-entry
status : pending
summary : 依赖可用、agent 可见、执行查找三个不同问题，不能混用。
x : 2124
y : 702
w : 398
h : 531
domain : tools-permission
```

**主张：** 一次调用要穿过三个不同问题——配置/依赖是否可用、当前 agent 是否可见、执行时是否找到实现。

- 可用：`capability_catalog` 汇总 core、引擎卡、custom 定义与 MCP 配置，并把每个能力映射到成员函数；引擎状态只进 `status` 投影，不参与选择（[@ file="src/tool/capability.rs" symbol="fn capability_catalog"]、[@ file="src/tool/availability.rs" symbol="fn available_tools"]）。
- 可见：`selected_for_agent` 只做 `binding.enabled ∩ scope`，`build_tool_list` 按能力实例化成员——子 agent 深度门由 scope 表统一表达，MCP 成员在本轮握手后产出；`RuntimeContext::tool_defs` 把 name / description / schema 送进模型请求（[@ file="src/tool/capability.rs" symbol="impl CapabilityCatalog › fn selected_for_agent"]、[@ file="src/tool/registry.rs" symbol="fn build_tool_list"]、[@ file="src/runtime/context.rs" symbol="impl RuntimeContext › fn tool_defs"]）。
- 快照：列表在该 agent 首次 `run_with_turn` 时惰性构建，之后固定（[@ file="src/runtime/mod.rs" symbol="impl AgentRuntime › fn run_with_turn"]）。
- 执行查找：按 name 在本轮工具快照内找实现，未命中回错误结果；查找发生在执行入口（见 [@ key="tool-call-lifecycle"]）。
- 边界：统一入口只表示调用从同一处进入、事实参数由管线注入，不等于授权，也不等于沙箱。

父节点：[@ key="tool-call-contract"]。
相关：[@ key="tool-call-lifecycle"]、[@ key="tool-identity"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 可用/可见/快照 —— [@ file="src/tool/capability.rs" symbol="fn capability_catalog"]、[@ file="src/tool/capability.rs" symbol="impl CapabilityCatalog › fn selected_for_agent"]、[@ file="src/tool/registry.rs" symbol="fn build_tool_list"]。
- MCP 配置存在不等于进程已运行 —— 状态只影响执行反馈，不影响卡片可见性。
