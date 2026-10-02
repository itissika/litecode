```node
node : context-request
status : pending
summary : 请求视图：从权威工作集派生一次性 PreparedView；适配、补齐与媒体裁剪不回写历史。
x : 748
y : 530
w : 480
h : 560
```

**持久历史回答发生了什么，请求视图回答这一次模型能看到什么。**

### 数据
- `PreparedView` 持有本次 `items`、位置对齐的 `item_seqs` 和 token 估算；合成内容没有来源 seq。
- `ModelRequest` 用统一 Item 输入，provider wire 编码由 codec 负责；纯文本 preview 不是结构化输入真源。
- turn 工作集是带日志身份的派生窗口及未提交尾部，不是第二份永久历史。

### 流转
同步日志工作集 → Item 投影 → 补未应答调用 → 模型能力适配 → 媒体预算 → 展开媒体引用 → PreparedView → 取走视图／构建请求 → provider codec。

### 不变量
- 每步重建请求副本，视图补齐、媒体占位与 provider replay 适配不回写业务日志。
- `item_seqs` 与 items 按位置对齐；provider 可跨请求复用 id，不能用它反查永久身份。
- 无 PreparedView 不能调用模型；准备和消费必须遵循同一请求边界。
- 不静默机械截短对话历史；但请求媒体可能按预算裁剪或因能力不支持降级为明确占位，不能把“持久历史无损”理解为“每次请求逐字逐媒体全送”。
- 媒体引用在日志中保持引用，仅在请求副本内解析；文本 preview 不得用于重新组装多模态消息或预算真相。
- 回退使旧未提交尾部失效，后续提交不得将已丢弃的内容重新接回。

依据：[@ file="src/context_pipeline/view.rs" symbol="struct PreparedView" label="临时视图"]、[@ file="src/context_pipeline/mod.rs" symbol="impl ContextPipeline › fn build_view" label="组装边界"]、[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn call_model" label="消费边界"]、[@ file="src/llm/replay_compat.rs" label="请求副本适配"]。
验证：[@ file="tests/working_set_identity.rs" symbol="fn revert_then_commit_discard_returns_fold_window_and_does_not_append" label="回退尾部作废"]、[@ file="src/context_pipeline/media_budget.rs" label="媒体预算与测试"]。

上层：[@ id="context" label="context"]。
