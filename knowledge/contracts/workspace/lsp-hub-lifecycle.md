```node
node : lsp-hub-lifecycle
status : pending
summary : LSP hub 生命周期：activate 只登记可运行命令；按需 spawn；stop 清空并 shutdown 全部实例。
x : 5094
y : 1620
domain : ide-substrate
```

**准入：** A2/A4。`LspHub` 状态机与引擎 `EngineState` 配合但不等价：`activate` 元数据就绪且不 spawn，语言服务按路径懒启动；`stop` 置 inactive、排空 servers 并逐个 shutdown。人类编辑与 agent 工具消费同一 hub 实例（经 `LspEngine`/`IdeBaseHandle`）。坏了会出现双写文档、幽灵语言服务或 configured 被当成 Warm。

依据：[LspHub.activate](src/lsp/hub.rs#LspHub.activate)、[LspHub.stop](src/lsp/hub.rs#LspHub.stop)、[LspEngine.stop](src/engines/lsp_engine.rs#LspEngine.stop)、[LspEngine.warmup](src/engines/lsp_engine.rs#LspEngine.warmup)。
上层：[engine-lifecycle](knowledge/contracts/workspace/engine-lifecycle.md)。
