```node
node : tool-permission
status : pending
summary : 授权契约：floor → 首条策略 → Ask grant／交互；Deny 不可软化，child 不向人类问询。
x : 34
y : 674
w : 398
h : 720
```

**权限判定控制一次调用是否执行，不把模型声明当作权限来源。**

### 数据
- 动作只有 Allow、Ask、Deny；`EvalResult` 带 action 与命中 rule_id。
- 每个 agent/tool 的 binding 提供策略与 path_mode；Primary／Subagent 视图决定 Ask 的处理。
- runtime grant 键为 `(agent, tool, rule_id)`，是进程内许可，不是持久配置修改。

### 流转
解析／schema 校验 → core-none 豁免或安全 floor → 有序策略首条命中／默认 → child 收紧 → Ask grant → 人类问询／直接执行或拒绝。

### 不变量
- core-none 工具是明确的免授权集合；其他工具的硬 floor 先于可配置策略，不能被 preset、Allow 或 Always 放宽。
- 策略首条匹配赢，无匹配才用默认；preset 是写表模板，不是另一套求值引擎。
- grant 只能把 Ask 软化为 Allow，不能覆盖 Deny，也不赋予额外 path_mode。
- child 的 Ask 在 engine 层变为 Deny，child turn 另用 deny sink；不继承父侧交互资格，不向人类弹卡。
- Primary 的 Ask 等待明确答复；拒绝产生工具错误，等待中取消是 Aborted，不允许继续执行。
- schema 有效、工具可见、授权通过分别是不同条件；可见不等于被允许执行。

依据：[@ file="src/permission/engine.rs" symbol="impl PermissionEngine › fn evaluate_tool" label="求值与 child 视图"]、[@ file="src/permission/floor.rs" symbol="fn check_floor" label="硬底线"]、[@ file="src/permission/evaluate.rs" symbol="fn evaluate" label="规则顺序"]、[@ file="src/tool/authorize.rs" symbol="fn authorize" label="grant 与交互"]。
验证：[@ file="tests/permission_pipeline.rs" label="授权管线测试"]、[@ file="tests/client_protocol_permission.rs" label="交互与取消测试"]。

上层：[@ id="tools" label="tools"]。
