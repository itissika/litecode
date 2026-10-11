```node
node : turn-llm-binding
status : pending
summary : turn 的模型绑定：会话种子/修复唯一入口；能力不匹配硬失败，投影只裁请求副本。
x : 5598
y : 1566
domain : harness-core
```

**准入：** A3+A5。`TurnLlmBinding` 是一次 turn 解析出的 provider+凭证+model；会话缺模型时由 `ensure_session_model` 种子修复，不得在别处另开绑定。输入能力校验失败是硬错误；不支持的模态只投影省略请求副本，不回写日志。坏了会导致错误模型调用或把裁剪写进真源。

依据：[@ file="src/runtime/llm_resolve.rs" symbol="struct TurnLlmBinding"]、[@ file="src/runtime/llm_resolve.rs" symbol="fn resolve_session_llm"]、[@ file="src/runtime/llm_resolve.rs" symbol="fn validate_llm_input_capabilities"]。
上层：[@ key="turn-lifecycle"]。相关：[@ key="provider-catalog"]、[@ key="config-commit"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- TurnLlmBinding 唯一解析入口 —— [@ file="src/runtime/llm_resolve.rs" symbol="struct TurnLlmBinding"]、[@ file="src/runtime/llm_resolve.rs" symbol="fn resolve_session_llm"]。
- 能力不匹配硬失败 —— [@ file="src/runtime/llm_resolve.rs" symbol="fn validate_llm_input_capabilities"]；调用前 fail-closed：[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn call_model"]。
- 压缩绑定 session-blind —— [@ file="src/runtime/llm_resolve.rs" symbol="fn binding_for_agent"]（隐藏 `compaction` agent，不继承会话模型/tier/mode）。
