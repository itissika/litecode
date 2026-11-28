```node
node : llm-provider-surface
status : pending
summary : LLM 产品面：唯一入口 complete_with_stream_events；方言关在 codec/；无平行非流式入口。
x : 5364
y : 1566
domain : model-config
```

**准入：** A1+A4。产品只看见 `LlmProvider` + `ModelRequest` + authority Items；`provider_from_model` / `codec::build` 按目录选 codec。静默丢弃 `on_event` 或另开非流式产品入口会让投影与回放分叉。坏了波及 runtime 投影、turn 绑定与客户端流。

依据：[LlmProvider](src/llm/provider.rs#LlmProvider)、[ResponsesCodec.LlmProvider.complete_with_stream_events](src/llm/codec/responses.rs#ResponsesCodec.LlmProvider.complete_with_stream_events)、[provider_from_model](src/llm/mod.rs#provider_from_model)、[ChatCompletionsCodec.LlmProvider.complete_with_stream_events](src/llm/codec/chat.rs#ChatCompletionsCodec.LlmProvider.complete_with_stream_events)。
上层：[provider-catalog](knowledge/framework/provider-catalog.md)。
相关：[turn-llm-binding](knowledge/contracts/agent/turn-llm-binding.md)。
