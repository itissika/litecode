```node
node : platform-knobs
status : pending
summary : 用户选的是「想多久、上下文多大」，不是厂商参数；压缩请求复用同一个供应商、同一段前缀，为的是缓存全命中。
x : 5094
y : 1728
domain : harness-core
```

**准入：** A7。产品只暴露两个意图：思考强度、上下文模式；厂商写法只在模型目录里对应，不进产品路径。压缩请求的首要目标是省钱：它必须复用同一个供应商、同一段前缀，让缓存全部命中，不为摘要再付一遍 miss。模型、指令、工具、思考强度都跟着主对话走，就是为了不把这个前缀弄断——第二套厂商字符串会绕过目录契约，第二套思考设置会让前缀对不上，缓存直接失效。

依据：[@ file="src/platform_knobs.rs" symbol="enum ThinkingSpec"]、[@ file="src/platform_knobs.rs" symbol="enum ContextMode"]、[@ file="src/llm/request.rs" symbol="struct CompactionRequestContext"]、[@ file="src/runtime/llm_resolve.rs" symbol="impl TurnLlmBinding › fn compaction_context"]、[@ file="src/types/death_list_gate.rs"]。
上层：[@ key="agent-harness-philosophy"]。相关：[@ key="provider-catalog"]。
基线：666540c

**核实结论：按实现订正。**（基线 666540c）
- 两个平台意图不变 —— [@ file="src/platform_knobs.rs" symbol="enum ThinkingSpec"]、[@ file="src/platform_knobs.rs" symbol="enum ContextMode"]。
- 压缩复用主请求：provider、model、指令、工具、thinking 全部复制 —— [@ file="src/llm/request.rs" symbol="struct CompactionRequestContext"]、[@ file="src/runtime/llm_resolve.rs" symbol="impl TurnLlmBinding › fn compaction_context"]；原「压缩强制 Off」不成立，`ModelRequest::compact` 这个入口已删。
- 厂商字面量只在目录 tiers —— [@ file="src/types/death_list_gate.rs"]。
