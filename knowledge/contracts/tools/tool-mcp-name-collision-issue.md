```node
node : tool-mcp-name-collision-issue
status : pending
summary : [问题] MCP 原始名可与内置/Custom 同名：tool 数组重名不去重，first-match 遮蔽一份；若 MCP 先命中，还按同名工具的策略授权。
x : 4428
y : 396
w : 480
h : 560
```

**问题：** MCP 工具的 `name` 就是服务器原始工具名（`tools/list` 的 `name` 原样保留），可与内置/Custom 工具同名；工具数组按目录 id 排序装配、不去重，运行期按 name first-match。重名时排后面的一份被广告却不可达；具体命中谁由装配顺序决定，不同目录项之间按目录 id 字节序排列。

**触发条件（同时满足）**
1. 某个 MCP 服务器暴露了与其它已装配工具同名的工具；
2. `mcp_<server_id>` 对该 agent 可见（binding enabled），且 `allowed_tools` 为 `None` 或含该名 [@ file="src/tool/registry.rs" symbol="fn instantiate_tool" lines="92-99"]；
3. 同名的另一份也对同一 agent 可见（内置需其目录项 binding enabled）[@ file="src/tool/availability.rs" symbol="fn should_include_in_llm_list" lines="122-143"]；
4. 二者出现在同一 turn 的 `build_tool_list` 输出里。

**实际后果**
- 模型 tool 数组出现两条同名条目（不去重）；provider 是否接受未验证，不作断言。
- first-match 遮蔽：`find` 决定实际运行、调度属性与输入校验用的 schema [@ file="src/tool/executor.rs" symbol="fn run_tool" lines="303-319"]、[@ file="src/tool/executor.rs" symbol="fn partition_tool_calls" lines="43-52"]。谁在前由装配顺序决定（不同目录项按目录 id 字节序）：`edit` < `mcp_<server_id>` < `read`，所以撞 `read` 时先命中 MCP 副本，撞 `edit` 时仍命中内置；被遮蔽的一份在同一 turn 内不可达（两个 MCP 服务器之间同理，按 server_id 序）。
- 授权按模型给的 name：命中 MCP 副本时，求值用的仍是「该名字」的绑定（同名内置/Custom 的策略与 path_mode 被套到 MCP 实现上）[@ file="src/tool/authorize.rs" symbol="fn authorize" lines="51-62"]、[@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn effective_policy" lines="129-142"]；若该名字在 `core_none_tools` 豁免名单内（plan/todo/knowledge/subagent_*，完整清单 [@ file="src/config/global_db/tools.rs" lines="22-33"]），`evaluate_tool` 提前返回 Allow，策略与 floor 都不求值 [@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn evaluate_tool" lines="78-83"]。

**证据链**
1 来源：[@ file="src/tools/mcp_tool.rs" symbol="impl Tool for McpTool › fn name"]、[@ file="src/mcp/client.rs" lines="124-152"]。
2 接入：[@ file="src/tool/registry.rs" symbol="fn instantiate_tool" lines="83-120"]、目录 id 排序后逐个 push、无去重 [@ file="src/tool/registry.rs" symbol="fn build_tool_list" lines="204-274"]。
3 投影：[@ file="src/runtime/context.rs" symbol="impl RuntimeContext › fn tool_defs"]、[@ file="src/runtime/exec.rs" lines="508-517"]、[@ file="src/llm/codec/responses.rs" lines="109-120"]。
4 消费：[@ file="src/tool/executor.rs" symbol="fn run_tool" lines="303-319"]、[@ file="src/tool/executor.rs" symbol="fn partition_tool_calls" lines="43-52"]。
5 授权：[@ file="src/tool/authorize.rs" symbol="fn authorize" lines="51-62"]、[@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn evaluate_tool" lines="78-83"]、[@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn effective_policy" lines="129-142"]、[@ file="src/permission/policy.rs" symbol="impl Default for ToolPolicy" lines="28-36"]。

**核证程度：** 源码静态核证，未动态复现（未启动 MCP、未改配置）。未验证：provider/模型面对重名 tool 数组的行为，以及模型采用哪份 schema。

**关联：** [@ key="tool-identity"]、[@ key="tool-adapters"]、[@ key="tool-authorization"]。
