```node
node : platform-knobs
status : pending
domain : harness-core
summary : 平台旋钮：ThinkingSpec / ContextMode 是产品意图；厂商字面量只在目录 tiers 映射。
```

**准入：** A7。UI/session 持久化平台枚举，不把厂商 thinking 字符串写进产品路径；codec 按目录 `reasoning.tiers` 映射，压缩路径强制 `ThinkingSpec::Off`。第二套厂商字符串会绕过目录契约并污染回放。

依据：[@ file="src/platform_knobs.rs" symbol="enum ThinkingSpec"]、[@ file="src/platform_knobs.rs" symbol="enum ContextMode"]、[@ file="src/types/death_list_gate.rs" symbol="fn compact_llm_path_does_not_bypass_unified_provider"]。
上层：[@ key="agent-harness-philosophy"]。相关：[@ key="provider-catalog"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- ThinkingSpec / ContextMode 为平台意图 —— [@ file="src/platform_knobs.rs" symbol="enum ThinkingSpec"]、[@ file="src/platform_knobs.rs" symbol="enum ContextMode"]。
- 压缩路径强制 ThinkingSpec::Off —— [@ file="src/llm/request.rs" symbol="impl ModelRequest › fn compact"]（`src/context_pipeline/compact.rs` 单测 `compact_request_disables_thinking_and_uses_caller_cap` 断言同点；cfg(test) 未入符号索引）。
- 厂商字面量只在目录 tiers —— [@ file="src/types/death_list_gate.rs" symbol="fn compact_llm_path_does_not_bypass_unified_provider"]；产品路径未见绕过 catalog 的厂商 thinking 字符串（DRAFT-HC-R1 关闭）。
