```node
node : tool-authorization
status : pending
summary : 授权先解析校验、再 floor 与首命中策略求值；Allow 只表示不阻塞，不等于沙箱。
x : 2556
y : 396
w : 416
h : 471
```

**主张：** `run_tool` 先解析并校验参数，再让 `authorize` 求值策略；`Allow` 只表示不阻塞，不等于被沙箱约束。

- 顺序：`parse_tool_arguments` → `check_tool_input` → `evaluate_tool`，失败即 Denied；仅当求值为 Ask 且存在 runtime grant(Allow) 时才软化 [@ file="src/tool/authorize.rs" symbol="fn authorize"]。
- 豁免与 floor：`core_none_tools`（plan/todo/knowledge/subagent_*）直接 Allow，其余先过 floor 再求值策略 [@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn evaluate_tool"]；floor 只有两条——write/edit 命中敏感系统路径、bash 命中危险命令，命中即无条件 Deny [@ file="src/permission/floor.rs" symbol="fn check_floor"]。
- 策略来源：内置工具用 binding 存储的 `ToolPolicy`，无 binding 时 `unwrap_or_default()` 即 allow_all，不会因缺 binding 而拒绝；`path_mode` 取自同一 binding。custom 工具规则取自定义（workspace 覆盖 global），只有“Safe 档且规则非空”才逐条求值，无档位/All/空规则都走 allow_all [@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn custom_tool_rules"]、[@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn effective_policy"]。
- 求值首命中规则，否则 default [@ file="src/permission/evaluate.rs" symbol="fn evaluate"]；subagent view 在 engine 把 Ask 变 Deny [@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn evaluate_tool"]。`always` 授权是进程内 static、按 agent+tool+rule 保存，TTL 1h、每 agent 上限 100，不落盘 [@ file="src/permission/grants.rs" symbol="fn grant_runtime"]、[@ file="src/permission/grants.rs" lines="73-107"]。
- engine 持有 resolved 快照（首次 turn 在 `run_with_turn` 里用 `runtime.resolved.clone()` 调用 `resolver` 构造），“当前”指该快照，不随磁盘配置逐次刷新 [@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn resolver"]、[@ file="src/runtime/mod.rs" symbol="impl AgentRuntime › fn run_with_turn" lines="879-883"]。

相关：[@ key="tool-control-boundary"]、[@ key="tool-execution-context"]。
