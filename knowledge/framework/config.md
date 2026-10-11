```node
node : config
status : enabled
domain : model-config
summary : config：唯一写入门（ConfigGate）；运行时状态是投影，永不写文档。
x : 990
y : 350
w : 362
h : 337
```

- 唯一写入门：UI / CLI / API 同一写面，没有第二扇门；提交带 generation，表示保存世代而非乐观锁。
- 配置是意图，运行态是投影：投影由 owner 发布、只读，永不回写文档。
- 分层覆盖：Builtin < Global < Workspace，就近生效。
- engines.json 表达引擎启停意图；工具 readiness 从配置意图派生，运行状态与执行可用性另由 owner 发布。

契约展开：[@ key="config-commit"]、[@ key="engine-lifecycle"]、[@ key="provider-catalog"]。
上层：[@ key="engine-design"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一写入门 ConfigGate / SettingsWriter —— 见 [@ key="config-commit"]；分层 Builtin < Global < Workspace 由 ResolvedConfig 兑现。
- 运行态是投影、永不写文档 —— ADR docs/adr/0002-config-gate.md 与实现一致。
