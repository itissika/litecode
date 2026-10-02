```node
node : tool-pipeline
status : enabled
summary : 工具执行：模型参数与执行事实分离；按资源冲突调度；结果按 call_id 配对、按调用顺序落地。
x : 990
y : 548
w : 452
h : 414
```

**主张：** 工具实现可以不同，生产执行入口、授权与输出契约不能绕过。

- 统一合约：内置 / Custom / MCP 经同一执行入口；模型参数与执行事实分离，会话、路径模式、取消等由内核注入，不由模型 JSON 决定。
- 调度按声明：并发安全且资源不冲突才并行，其余按批串行、串行批构成屏障；跨会话互斥只覆盖文件写（write / edit），冲突显式失败不排队。
- 授权单管线：Allow / Ask / Deny 三动作；安全 floor 不可放宽，有序策略首条命中；进程内许可只能把 Ask 软化为 Allow，Deny 不可软化。
- 子 agent 收紧：child 的 Ask 一律视为 Deny，不向人类问询，也不继承父侧许可。
- 结果契约：按 call_id 配对、按调用顺序输出，每个调用都有结果——取消补中断结果，模型永不见悬空调用；已完成的结果不被晚到的取消覆盖。
- 出口统一整形：截断、超大结果落盘、信号合成在管线完成，工具不自造方言。

上层：[@ id="tools" label="tools"]。相关：[@ id="ux-interaction" label="交互模型"]。
依据：[@ file="src/tool/executor.rs" label="生产入口与调度"]、[@ file="src/permission/engine.rs" label="授权求值"]。
