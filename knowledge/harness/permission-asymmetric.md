```node
node : permission-asymmetric
status : pending
summary : 权限只有 Allow/Ask/Deny：匹配面做重、动作面简单；floor 不可放宽、grant 只软化 Ask、subagent Ask→Deny。
x : 1224
y : 900
```

**动作枚举保持简单，匹配面做重；收紧单向、放松受限。**

- 三动作：Allow / Ask / Deny；per-(agent, tool) 有序规则表，首条命中即赢 [@ file="src/permission/evaluate.rs" label="evaluate.rs"]。
- 安全 floor 先于策略：敏感路径等不可编辑硬 Deny，用户 Allow / Always 都不能放宽 [@ file="src/permission/floor.rs" label="floor.rs"]。
- 进程内 grant 只把 Ask 软化为 Allow（按 rule_id 粒度），不覆盖 Deny [@ file="src/tool/authorize.rs" label="authorize.rs"]。
- 视图差异：Primary 的 Ask → 交互卡；Subagent 静态视图，Ask 视为 Deny，且不继承父侧 grant [@ file="src/permission/engine.rs" label="engine.rs"]。
- Preset（ALL / SAFE）是整表替换的写表模板，不是第二套引擎 [@ file="src/permission/presets.rs" label="presets.rs"]。
- 人类侧形态见 [@ id="collaboration-cards" label="协作卡"]。


