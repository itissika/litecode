```node
node : turn-llm-binding
status : pending
summary : 库里有模型就能直接回车发话；这套选择一次定下，压缩不换模型。
x : 5598
y : 1566
domain : harness-core
```

**准入：** A3+A5。用户不该为了发一句话先研究用哪个模型：只要库里有可用模型，会话就能开跑；缺模型时自动落到第一个能用的，并把这套选择定下来。一次对话只用一套 provider + 密钥 + 模型。压缩同样不另找模型——它把这套原样拿来再发一次；换模型会让缓存重付，费用和口吻也跟着变。能力问题（输入模态不支持）是硬错误，不做静默降级；不支持的模态只裁请求副本，不回写日志。

依据：[@ file="src/runtime/llm_resolve.rs" symbol="struct TurnLlmBinding"]、[@ file="src/runtime/llm_resolve.rs" symbol="fn resolve_session_llm"]、[@ file="src/runtime/llm_resolve.rs" symbol="fn validate_llm_input_capabilities"]、[@ file="src/runtime/llm_resolve.rs" symbol="impl TurnLlmBinding › fn compaction_context"]。
上层：[@ key="turn-lifecycle"]。相关：[@ key="provider-catalog"]、[@ key="config-commit"]。
基线：666540c

**核实结论：按实现订正。**（基线 666540c）
- 唯一解析入口：`TurnLlmBinding` + `resolve_session_llm`，缺模型由 `ensure_session_model` 补 —— [@ file="src/runtime/llm_resolve.rs" symbol="struct TurnLlmBinding"]、[@ file="src/runtime/llm_resolve.rs" symbol="fn resolve_session_llm"]。
- 能力不匹配硬失败 —— [@ file="src/runtime/llm_resolve.rs" symbol="fn validate_llm_input_capabilities"]；调用前 fail-closed：[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn call_model"]。
- 压缩不做单独绑定：`compaction_context` 从当前绑定复制 provider / model / thinking，注释写明下游不存在第二套解析 —— [@ file="src/runtime/llm_resolve.rs" symbol="impl TurnLlmBinding › fn compaction_context"]；原隐藏 compaction agent 的 `binding_for_agent` 已删。
