```node
node : tool-input-validation
status : pending
summary : 参数先过 schema 子集与语义两层校验；未知顶层参数默认只是 Warning。
x : 3024
y : 396
w : 434
h : 554
```

**主张：** 参数在业务实现前被两层检查：公共 schema 子集 + 工具自己的语义校验；未知顶层键默认提醒而不拦截。

- 解析：`parse_tool_arguments` 要求严格 JSON，失败即 Error，不降级为 Null 放行（[@ file="src/tool/schema_validate.rs" symbol="fn parse_tool_arguments"]）。
- 子集：公共校验覆盖 type / enum / required / additionalProperties / items / minItems，不做类型强转；空 schema 视为不约束（[@ file="src/tool/schema_validate.rs" lines="116-166"]）。
- 未知顶层键：schema 有约束且未放行/未禁止 additionalProperties 时，`unknown_top_level_properties` 收集、`attach_unknown_param_warning` 附 Warning、调用继续；schema 显式 `additionalProperties:false` 时在 `validate_object` 直接 Error（[@ file="src/tool/schema_validate.rs" symbol="fn unknown_top_level_properties"]、[@ file="src/tool/schema_validate.rs" symbol="fn validate_object" lines="240-248"]、[@ file="src/tool/executor.rs" symbol="fn attach_unknown_param_warning"]）。
- 语义与复核：`check_tool_input` 先 schema、再 `Tool::validate_input`；`authorize` 在权限求值前重复解析与校验，失败直接 Denied（[@ file="src/tool/schema_validate.rs" symbol="fn check_tool_input"]、[@ file="src/tool/trait_.rs" symbol="trait Tool › fn validate_input"]、[@ file="src/tool/authorize.rs" symbol="fn authorize"]）。
- 边界：未知键的 Warning 不掩盖被工具语义校验或授权拒绝的调用。

父节点：[@ key="tool-call-contract"]。
