```node
node : tool-request-entry
status : enabled
summary : 工具来源杂（内置 / 引擎 / 子代理 / custom / MCP），但杂不能乱：每个工具归一张卡，每个成员名只认一个主人。
x : 2124
y : 702
w : 398
h : 531
domain : tools-permission
```

**解决什么：** 工具来自好几处——内置、引擎、custom、MCP——来源杂，但必须码得清：每个工具归哪张卡、哪个名字对应谁，要能一条一条数出来。设置页、agent 选工具、权限归属看的是同一份目录，才不会出现「设置里看得见、agent 调不到」。[capability_catalog](src/tool/capability.rs#capability_catalog)

**必须做到：** 目录只生成一份，成员名只认一个主人；撞名不藏着——记成错误照样报出来，但名字归先注册的那张卡，后来的成员直接失效，别让一个坏定义废掉整轮。[push_capability](src/tool/capability.rs#push_capability)

**绝对不允许：** 谁要工具清单就自己翻配置拼一份——第二份目录就是「设置说有、agent 调不到」的源头；选工具只能从这份目录里筛。[build_tool_list_with_resolution](src/tool/registry.rs#build_tool_list_with_resolution)

**守它的测试：** 手改一个和内置同名的 custom、两张卡都启用，内置工具仍然在 SDK 里，custom 到不了面上。[colliding_custom_name_cannot_drop_the_builtin_tool](src/tool/registry.rs#colliding_custom_name_cannot_drop_the_builtin_tool) 目录侧的同一条不变量：[duplicate_member_names_are_reported_not_merged](src/tool/capability.rs#duplicate_member_names_are_reported_not_merged)

上游：[tool-call-contract](knowledge/contracts/tools/tool-call-contract.md)。相关：[tool-identity](knowledge/contracts/tools/tool-identity.md)、[tool-execution-context](knowledge/contracts/tools/tool-execution-context.md)。
