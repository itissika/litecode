```node
node : tool-input-validation
status : pending
summary : 参数先过 schema 子集与语义两层校验；未知顶层参数默认只是 Warning。
x : 2124
y : 810
w : 434
h : 554
domain : tools-permission
```

**主张：** 参数在业务实现前被两层检查：公共 schema 子集 + 工具自己的语义校验；未知顶层键默认提醒而不拦截。

- 解析：`parse_tool_arguments` 要求严格 JSON，失败即 Error，不降级为 Null 放行（[parse_tool_arguments](src/tool/schema_validate.rs#parse_tool_arguments)）。
- 子集：公共校验覆盖 type / enum / required / additionalProperties / items / minItems，不做类型强转；空 schema 视为不约束（[schema_validate.rs:116-166](src/tool/schema_validate.rs#L116-L166)）。
- 未知顶层键：schema 有约束且未放行/未禁止 additionalProperties 时，`unknown_top_level_properties` 收集、`attach_unknown_param_warning` 附 Warning、调用继续；schema 显式 `additionalProperties:false` 时在 `validate_object` 直接 Error（[unknown_top_level_properties](src/tool/schema_validate.rs#unknown_top_level_properties)、[validate_object](src/tool/schema_validate.rs#validate_object)、[attach_unknown_param_warning](src/tool/executor.rs#attach_unknown_param_warning)）。
- 语义与复核：`check_tool_input` 先 schema、再 `Tool::validate_input`（[check_tool_input](src/tool/schema_validate.rs#check_tool_input)、[Tool.validate_input](src/tool/trait_.rs#Tool.validate_input)）。授权前再次解析校验由 [tool-authorization](knowledge/contracts/tools/tool-authorization.md) 拥有（同一 `authorize` 入口）。
- 边界：未知键的 Warning 不掩盖被工具语义校验或授权拒绝的调用。

父节点：[tool-call-contract](knowledge/contracts/tool-call-contract.md)。
