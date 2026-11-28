```node
node : context-budget
status : enabled
summary : 该压的时候压、不该压的时候别打扰：已发送的那段用真实用量定价，只有新内容才本地估；80% 准备压缩，100% 不再发送，停下来等压完就继续跑，体验丝滑。
x : 5094
y : 720
domain : harness-core
```

用户只关心两件事：别在快满的时候突然发不出去，也别在没满的时候被反复压缩打断，压缩看的是产品的上下文0.8，上下文是供应商给的，litecode不造假。所以量的时候要用真账——上一轮实际消耗覆盖已发送的那段，只有之后新写的才本地估；没有账单才整段估。判的时候只有一处：80% 是准备线，100% 是硬线（这一步就不会发出去）。历史被就地改短之后旧账单作废，必须按新内容重算；否则等于拿旧价格给新前缀定价，会在阈值附近来回抖，或者漏掉最后一道线。

依据：[ContextMeter.measure](src/context_pipeline/context_meter.rs#ContextMeter.measure)、[ContextMeter.should_compact](src/context_pipeline/context_meter.rs#ContextMeter.should_compact)、[ContextMeter.hard_limit](src/context_pipeline/context_meter.rs#ContextMeter.hard_limit)、[ContextMeter.is_at_wait_line](src/context_pipeline/context_meter.rs#ContextMeter.is_at_wait_line)、[ProviderPromptBaseline.invalidate](src/context_pipeline/budget.rs#ProviderPromptBaseline.invalidate)、[tests.provider_bill_covers_its_prefix_and_estimates_only_the_suffix](src/context_pipeline/budget.rs#tests.provider_bill_covers_its_prefix_and_estimates_only_the_suffix)。
父节点：[context-view](knowledge/contracts/context/context-view.md)。
