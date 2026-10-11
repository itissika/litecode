```node
node : llm-provider-surface
status : pending
summary : LLM 产品面：唯一入口 complete_with_stream_events；方言关在 codec/；无平行非流式入口。
x : 5364
y : 1566
domain : model-config
```

**准入：** A1+A4。产品只看见 `LlmProvider` + `ModelRequest` + authority Items；`provider_from_model` / `codec::build` 按目录选 codec。静默丢弃 `on_event` 或另开非流式产品入口会让投影与回放分叉。坏了波及 runtime 投影、turn 绑定与客户端流。

依据：[@ file="src/llm/provider.rs" symbol="trait LlmProvider"]、[@ file="src/llm/codec/responses.rs" symbol="impl LlmProvider for ResponsesCodec › fn complete_with_stream_events"]、[@ file="src/llm/mod.rs" symbol="fn provider_from_model"]。
上层：[@ key="provider-catalog"]。
相关：[@ key="turn-llm-binding"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- 唯一产品入口 complete_with_stream_events —— [@ file="src/llm/provider.rs" symbol="trait LlmProvider"]、[@ file="src/llm/codec/responses.rs" symbol="impl LlmProvider for ResponsesCodec › fn complete_with_stream_events"]。
- 按目录选 codec —— [@ file="src/llm/mod.rs" symbol="fn provider_from_model"]。
- Chat Completions 同面 —— [@ file="src/llm/codec/chat.rs" symbol="impl LlmProvider for ChatCompletionsCodec › fn complete_with_stream_events"]（无平行非流式产品入口）。
