```node
node : lsp-hub-lifecycle
status : pending
domain : ide-substrate
summary : LSP hub 生命周期：activate 只登记可运行命令；按需 spawn；stop 清空并 shutdown 全部实例。
```

**准入：** A2/A4。`LspHub` 状态机与引擎 `EngineState` 配合但不等价：`activate` 元数据就绪且不 spawn，语言服务按路径懒启动；`stop` 置 inactive、排空 servers 并逐个 shutdown。人类编辑与 agent 工具消费同一 hub 实例（经 `LspEngine`/`IdeBaseHandle`）。坏了会出现双写文档、幽灵语言服务或 configured 被当成 Warm。

依据：[@ file="src/lsp/hub.rs" symbol="impl LspHub › fn activate"]、[@ file="src/lsp/hub.rs" symbol="impl LspHub › fn stop"]、[@ file="src/engines/lsp_engine.rs" symbol="impl LspEngine › fn stop"]、[@ file="src/engines/lsp_engine.rs" symbol="impl LspEngine › fn warmup"]。
上层：[@ key="engine-lifecycle"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- `activate` 仅登记可运行命令（不 spawn）—— [@ file="src/lsp/hub.rs" symbol="impl LspHub › fn activate"]；warmup 注释 metadata-only 见 [@ file="src/engines/lsp_engine.rs" symbol="impl LspEngine › fn warmup"]。
- `stop` 置 inactive、排空 servers 并 shutdown —— [@ file="src/lsp/hub.rs" symbol="impl LspHub › fn stop"]；引擎门面 [@ file="src/engines/lsp_engine.rs" symbol="impl LspEngine › fn stop"]。
- ISSUE-004（suspect）旁听 —— findReferences 上下文块主责更可能在 tools 输出整形，不否定 hub 状态机。
