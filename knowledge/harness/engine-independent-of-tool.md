```node
node : engine-independent-of-tool
status : pending
summary : 引擎独立于 Tool 生命周期：engines.json 驱动，configured ≠ Warm，多消费者共享实例。
x : 748
y : 900
w : 380
h : 377
```

**LSP / 语义检索是工作区长生命周期服务，不归任何 tool 所有。**

- 真相源：`.litecode/engines.json` 表达 desired；reconcile 后进入 Idle | Warming | Warm | Failed | Stopped [@ file="src/engines/mod.rs" label="mod.rs"]。
- configured（配置意图）≠ Warm（运行时可用）；tool 目录 readiness 由 desired 派生，不反向写回 [@ id="tool-aci" label="Tool 统一抽象"]。
- 门闸纪律：最终门闸是 agent 绑定——可见即尽量可用；未 Warm 尽量不进入 LLM tool list（软约束），Warm 表示能调用并返回结果，返回「启动中」还是真结果取决于启动成本。
- 共享消费：agent tool 与人类 UI（编辑器语言能力 / 搜索导航）共享同一实例 [@ id="workspace-shared-substrate" label="工作区基座"]。
- 横向可拓：新引擎遵循同一模型（配置位 + 生命周期 + 多消费者）。
