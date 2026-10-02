```node
node : kernel-l0-frozen
status : pending
summary : L0 控制流冻结：loop 只编排「取消/步数 → 视图 → 模型 → 工具或停止」，能力由 AgentDeps 注入。
x : 986
y : 900
```

**一段 while 循环，所有业务能力由依赖注入。**

- 控制流：每步先查取消 → max_steps 检查 → compact / 准备视图 → 调模型 → 执行工具或停止 [@ file="src/agent/core.rs" label="core.rs"]。
- 终止优先级：显式取消 > max_steps 超限 > 模型正常终止。
- 依赖注入边界：`AgentDeps` 是唯一依赖面；视图准备、持久化、seam 同步等具体实现由 L1 提供 [@ file="src/agent/deps.rs" label="deps.rs"]。
- 扩展纪律：新能力走 L1 二支柱与管线，不修改 loop 语义 [@ id="two-pillars" label="Context 与 Tool 二支柱"]。
- 注意：deps 面含 seam / prepare_view / persist / begin_step 钩子；以代码为准。


