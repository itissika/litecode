```node
node : platform-knobs
status : enabled
summary : 用户选的是「想多久、上下文多大」，不是厂商参数；压缩请求复用同一个供应商、同一段前缀，为的是缓存全命中。
x : 5094
y : 1728
domain : harness-core
```

产品只暴露两个意图：思考强度、上下文模式；厂商写法只在模型目录里对应，不进产品路径。压缩请求的首要目标是省钱：它必须复用同一个供应商、同一段前缀，让缓存全部命中，不为摘要再付一遍 miss。模型、指令、工具、思考强度都跟着主对话走，就是为了不把这个前缀弄断——第二套厂商字符串会绕过目录契约，第二套思考设置会让前缀对不上，缓存直接失效。

**入口只有一个字段**：会话面板和 headless 都写 `sessions.context_mode`，meter 只读这一行。headless 用 `litecode --context-mode <standard|max>`，不设就是 `standard`；不是第二套开关，也不是评测旁路。

依据：[ThinkingSpec](src/platform_knobs.rs#ThinkingSpec)、[ContextMode](src/platform_knobs.rs#ContextMode)、[effective_context_window](src/platform_knobs.rs#effective_context_window)、[CompactionRequestContext](src/llm/request.rs#CompactionRequestContext)、[TurnLlmBinding.compaction_context](src/runtime/llm_resolve.rs#TurnLlmBinding.compaction_context)、[death_list_gate.rs](src/types/death_list_gate.rs)。
上层：[agent-harness-philosophy](knowledge/harness/agent-harness-philosophy.md)。相关：[provider-catalog](knowledge/framework/provider-catalog.md)。
