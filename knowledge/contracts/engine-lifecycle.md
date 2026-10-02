```node
node : engine-lifecycle
status : pending
summary : 引擎契约：desired、运行 state、索引状态与工具可见性各有含义；code/session 执行门解耦。
x : 756
y : 710
w : 398
h : 720
```

**主张：** 是否启用、是否运行、数据是否就绪不能压成一个布尔值。

- 意图与状态分离：配置表达启用意图，owner 持有运行状态（Idle / Warming / Warm / Failed / Stopped）；工具绑定不拥有生命周期，显式刷新只能请求启动或维护，不能反向开配置。
- 索引状态独立：语料是否需要构建或更新是索引自己的事；可用性是面向消费者的派生视图，不是配置。
- 门闸与可见性解耦：工具可见性只由目录候选与 agent 绑定决定；执行门在消费时按运行与索引状态给出等待或失败，不拿旧索引假装成功。
- 两类语料各自独立：code 与 session 的执行门解耦——code 门同时要运行与索引可用；session 语义门只要求共享 worker Warm，滞后只损召回，命中仍回真源水合。
- 多消费者共享：人类界面与 agent 消费同一 owner 的状态与数据；界面只能读投影，不能回写意图或伪造生命周期。

上层：[@ id="config" label="config"]、[@ id="engine-design" label="engine 设计"]。
依据：[@ file="src/engines/mod.rs" label="生命周期与执行门"]、[@ file="src/engines/status_view.rs" label="可用性投影"]。
