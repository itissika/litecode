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

- 可用：`capability_catalog` 汇总 core、引擎卡、custom 定义与 MCP 配置，并把每个能力映射到成员函数；引擎状态只进 `status` 投影，不参与选择（[capability_catalog](src/tool/capability.rs#capability_catalog)、[available_tools](src/tool/availability.rs#available_tools)）。
- 可见：`selected_for_agent` 只做 `binding.enabled ∩ scope`，`build_tool_list` 按能力实例化所有已启用成员（子 agent 深度门由 scope 表统一表达，MCP 成员在本轮握手后产出）并装入唯一的顶层传输工具 `run_code`；`RuntimeContext::tool_defs` 仅向模型暴露 `run_code` 及其契约摘要与 schema（[CapabilityCatalog.selected_for_agent](src/tool/capability.rs#CapabilityCatalog.selected_for_agent)、[build_tool_list](src/tool/registry.rs#build_tool_list)、[RuntimeContext.tool_defs](src/runtime/context.rs#RuntimeContext.tool_defs)）。
- 快照：列表在该 agent 首次 `run_with_turn` 时惰性构建，之后固定（[AgentRuntime.run_with_turn](src/runtime/mod.rs#AgentRuntime.run_with_turn)）。
- 执行查找：按 name 在本轮工具快照内找实现，未命中回错误结果；查找发生在执行入口（见 [tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)）。
- 边界：统一入口只表示调用从同一处进入、事实参数由管线注入，不等于授权，也不等于沙箱。

父节点：[tool-call-contract](knowledge/contracts/tool-call-contract.md)。
相关：[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)、[tool-identity](knowledge/contracts/tools/tool-identity.md)。
