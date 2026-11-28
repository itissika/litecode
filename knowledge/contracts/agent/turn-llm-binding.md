```node
node : turn-llm-binding
status : enabled
summary : 库里有模型就能直接回车发话；这套选择一次定下，压缩不换模型。
x : 5598
y : 1566
domain : harness-core
```

用户不该为了发一句话先研究用哪个模型：只要库里有可用模型，会话就能开跑；缺模型时自动落到第一个能用的，并把这套选择定下来。一次对话只用一套 provider + 密钥 + 模型。压缩同样不另找模型——它把这套原样拿来再发一次；换模型会让缓存重付，费用和口吻也跟着变。能力问题（输入模态不支持）是硬错误，不做静默降级，表现是发不了消息，但是模型自己去看不会硬失败，会剥离掉媒体文件，让模型自己收敛或换个方式；不支持的模态只裁请求副本，不回写日志。

依据：[TurnLlmBinding](src/runtime/llm_resolve.rs#TurnLlmBinding)、[resolve_session_llm](src/runtime/llm_resolve.rs#resolve_session_llm)、[validate_llm_input_capabilities](src/runtime/llm_resolve.rs#validate_llm_input_capabilities)、[TurnLlmBinding.compaction_context](src/runtime/llm_resolve.rs#TurnLlmBinding.compaction_context)、[AgentRuntime.AgentDeps.call_model](src/runtime/exec.rs#AgentRuntime.AgentDeps.call_model)。
上层：[turn-lifecycle](knowledge/contracts/agent/turn-lifecycle.md)。相关：[provider-catalog](knowledge/framework/provider-catalog.md)、[config-commit](knowledge/contracts/config/config-commit.md)。
