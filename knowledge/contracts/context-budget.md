```node
node : context-budget
status : pending
summary : 该压的时候压、不该压的时候别打扰：已发送的那段用真实用量定价，只有新内容才本地估；80% 准备压缩，100% 不再发送。
x : 5094
y : 720
domain : harness-core
```

**准入：** A1。用户只关心两件事：别在快满的时候突然发不出去，也别在没满的时候被反复压缩打断。所以量的时候要用真账——上一轮实际消耗覆盖已发送的那段，只有之后新写的才本地估；没有账单才整段估。判的时候只有一处：80% 是准备线，100% 是硬线（这一步就不会发出去）。历史被就地改短之后旧账单作废，必须按新内容重算；否则等于拿旧价格给新前缀定价，会在阈值附近来回抖，或者漏掉最后一道线。

依据：[@ file="src/context_pipeline/context_meter.rs" symbol="impl ContextMeter › fn measure"]、[@ file="src/context_pipeline/context_meter.rs" symbol="impl ContextMeter › fn should_compact"]、[@ file="src/context_pipeline/context_meter.rs" symbol="impl ContextMeter › fn must_compact_before_send"]、[@ file="src/context_pipeline/budget.rs" symbol="impl ProviderPromptBaseline › fn invalidate"]、[@ file="src/context_pipeline/budget.rs" symbol="mod tests › fn provider_bill_covers_its_prefix_and_estimates_only_the_suffix"]。
父节点：[@ key="context-view"]。
基线：666540c

**核实结论：按实现订正。**（基线 666540c）
- 量：账单覆盖已发送前缀、只估后缀 —— [@ file="src/context_pipeline/context_meter.rs" symbol="impl ContextMeter › fn measure"]；同名测试仍在 [@ file="src/context_pipeline/budget.rs" symbol="mod tests › fn provider_bill_covers_its_prefix_and_estimates_only_the_suffix"]。
- 判：两条线同源 —— [@ file="src/context_pipeline/context_meter.rs" symbol="impl ContextMeter › fn should_compact"]、[@ file="src/context_pipeline/context_meter.rs" symbol="impl ContextMeter › fn must_compact_before_send"]；`BudgetPolicy` 只剩保留窗口与摘要输出上限，不再参与压力判断。
- 作废：裁剪改写 surface 后旧账单失效 —— [@ file="src/context_pipeline/budget.rs" symbol="impl ProviderPromptBaseline › fn invalidate"]。
