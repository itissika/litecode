```node
node : tool-authorization
status : pending
summary : 授权先解析校验、再 floor 与首命中策略求值；Allow 只表示不阻塞，不等于沙箱。
x : 2124
y : 486
w : 416
h : 471
domain : tools-permission
```

**主张：** `run_tool` 先解析并校验参数，再让 `authorize` 求值策略；`Allow` 只表示不阻塞，不等于被沙箱约束。

- 顺序：`parse_tool_arguments` → `check_tool_input` → `evaluate_tool`，失败即 Denied；仅当求值为 Ask 且存在 runtime grant(Allow) 时才软化 [authorize](src/tool/authorize.rs#authorize)。
- 豁免与 floor：`core_none_tools`（plan/todo/ask_user/knowledge/subagent_tools/litecode_workspace/bot）及其点号成员（如 `plan.*`、`todo.*`、`knowledge.*`、`subagent.*`、`workspace.*`、`bot.*`）经 `member_owner` 归属到能力卡 binding 直接 Allow，不再按成员名短路。其余先过 floor 再求值策略 [PermissionEngine.evaluate_tool](src/permission/engine.rs#PermissionEngine.evaluate_tool)；floor 只有两条——write/edit 命中敏感系统路径、bash 命中危险命令，命中即无条件 Deny [check_floor](src/permission/floor.rs#check_floor)。
- 策略来源：内置工具用 binding 存储的 `ToolPolicy`，无 binding 时 `unwrap_or_default()` 即 allow_all，不会因缺 binding 而拒绝；`path_mode` 取自同一 binding。custom 工具规则取自定义（workspace 覆盖 global），只有“Safe 档且规则非空”才逐条求值，无档位/All/空规则都走 allow_all [PermissionEngine.custom_tool_rules](src/permission/engine.rs#PermissionEngine.custom_tool_rules)、[PermissionEngine.effective_policy](src/permission/engine.rs#PermissionEngine.effective_policy)。
- 求值首命中规则，否则 default [evaluate](src/permission/evaluate.rs#evaluate)；subagent view 在 engine 把 Ask 变 Deny（子权限创建时固定，运行中需审批一律拒绝，不弹给任何人） [PermissionEngine.evaluate_tool](src/permission/engine.rs#PermissionEngine.evaluate_tool)。`always` 授权是进程内 static、按 agent+tool+rule 保存，TTL 1h、每 agent 上限 100，不落盘 [grant_runtime](src/permission/grants.rs#grant_runtime)、[grants.rs:73-107](src/permission/grants.rs#L73-L107)。
- engine 持有 resolved 快照（首次 turn 构造），“当前”指该快照，不随磁盘配置逐次刷新 [PermissionEngine.resolver](src/permission/engine.rs#PermissionEngine.resolver)；turn 入口时机见 [tool-request-entry](knowledge/contracts/tools/tool-request-entry.md)。

父节点：[tool-control-boundary](knowledge/contracts/tool-control-boundary.md)。
相关：[tool-control-boundary](knowledge/contracts/tool-control-boundary.md)、[tool-execution-context](knowledge/contracts/tools/tool-execution-context.md)、[tool-input-validation](knowledge/contracts/tools/tool-input-validation.md)。
