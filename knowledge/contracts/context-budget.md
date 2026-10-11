```node
node : context-budget
status : pending
summary : 上次请求的 provider prompt_tokens 是预算权威前缀；其后本地估算后缀；超阈触发 compact，硬顶拒绝越界。
x : 5094
y : 720
domain : harness-core
```

**准入：** A1。预算真源不能跨请求累加本地估算：有 baseline 时以 provider 计数覆盖已发送前缀，只对新增 Item 做本地估算；无 baseline 才整段估算。坏了会出现误触发 compact 或漏拦硬顶，污染准备视图与提交节奏。

依据：[@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn token_count_with_baseline"]、[@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn should_compact"]、[@ file="src/context_pipeline/budget.rs" symbol="mod tests › fn token_count_prefers_provider_when_set"]。
父节点：[@ key="context-view"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- provider 前缀权威 + 后缀本地估：`token_count_with_baseline` —— [@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn token_count_with_baseline"]；测试见 [@ file="src/context_pipeline/budget.rs" symbol="mod tests › fn token_count_prefers_provider_when_set"]。
- 超阈 compact / 硬顶拒绝：`should_compact`；`enforce_hard_limit_with_baseline` —— [@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn should_compact"]；[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_if_needed"]。
