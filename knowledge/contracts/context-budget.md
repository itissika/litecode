```node
node : context-budget
status : pending
summary : 上次请求的 provider prompt_tokens 是预算权威前缀；其后本地估算后缀；超阈触发 compact，硬顶拒绝越界。
x : 5094
y : 720
domain : harness-core
```

**准入：** A1。预算真源不能跨请求累加本地估算：有 baseline 时以 provider 计数覆盖已发送前缀，只对新增 Item 做本地估算；无 baseline 才整段估算。坏了会出现误触发 compact 或漏拦硬顶，污染准备视图与提交节奏。

依据：[@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn occupancy_token_count"]、[@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn should_compact"]、[@ file="src/context_pipeline/budget.rs" symbol="mod tests › fn provider_bill_covers_its_prefix_and_estimates_only_the_suffix"]。
父节点：[@ key="context-view"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b；锚点于 7286012 随 seq 化改名，语义未变）
- provider 前缀权威 + 后缀本地估：`occupancy_token_count`（原 `token_count_with_baseline`，覆盖判定由 item 数改为 row seq）—— [@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn occupancy_token_count"]；测试见 [@ file="src/context_pipeline/budget.rs" symbol="mod tests › fn provider_bill_covers_its_prefix_and_estimates_only_the_suffix"]、[@ file="src/context_pipeline/budget.rs"]。
- 超阈 compact / 硬顶拒绝：`should_compact`；`enforce_token_count`（原 `enforce_hard_limit_with_baseline`，改收已算好的计数）—— [@ file="src/context_pipeline/budget.rs" symbol="impl BudgetPolicy › fn should_compact"]；[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_if_needed"]。
