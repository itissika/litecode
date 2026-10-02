```node
node : tool-aci
status : pending
summary : Tool 统一抽象：同合约、同出口、同门闸；少即是多，工程兜底在管线。
x : 986
y : 1044
w : 344
h : 395
```

**所有 tool 对 loop 无差异：统一合约、统一出口、统一门闸。**

- 合约：唯一 name + JSON schema；单一 execute 边界；并发安全标记 + resource_keys 决定并行 / 串行 [@ file="src/tool/trait_.rs" label="trait_.rs"]。
- 门闸（不可用即不可见）：目录候选 ∧ agent 绑定（最终门闸）→ 进入 LLM tool list [@ file="src/tool/availability.rs" label="availability.rs"]；可见即尽量可用——未 Warm 也可调用并返回进一步信息。
- 出口整形：截断 / 超大落盘 / preview 由管线统一处理；信号三级 Error / Warning / Hint 合成稳定文本，工具不自写方言 [@ file="src/tool/signal.rs" label="signal.rs"]。
- 来源三类：内置 / Custom / MCP；子 agent 调度是内核编排（内置 tool 暴露），不列第四类。
- 演化纪律（少即是多）：能不加 tool 就不加；默认与约束由工程兜底，不交给模型猜。
