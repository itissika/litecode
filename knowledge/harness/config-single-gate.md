```node
node : config-single-gate
status : pending
summary : 配置唯一写入门：所有设置写路径都走 ConfigGate.commit；运行时状态是投影，永不写文档。
x : 272
y : 900
w : 362
h : 382
```

**设置是文档 CRUD + 带 generation 的提交通知；运行时状态是投影平面，两者不共享写路径。**

- 所有写路径（SettingsWriter / HTTP API / CLI config set）都走唯一 `ConfigGate.commit`；SQLite 与 `.litecode/*.json` 只是该门的存储后端 [@ file="docs/adr/0002-config-gate.md" label="ADR 0002"]。
- 提交才触发消费者：committed docs 含 engines 才 reconcile；MCP start 只按 id 读已存定义。
- 投影（可用状态 / 进程状态 / catalog）由 owner 发布，UI 可读；投影永不水合文档、永不进入 commit。
- 词汇与词义 [@ file="CONTEXT.md" label="CONTEXT.md"]。
