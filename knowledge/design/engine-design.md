```node
node : engine-design
status : enabled
summary : engine 设计理念：工作区引擎基座——LSP + 检索（code / session 两语料，稀疏 + 稠密两腿）；独立于 Tool、配置驱动、人类与 agent 共用。
x : 250
y : 188
w : 344
h : 321
```

- 长生命周期服务，不归任何 tool 所有；工具只是消费者，人类与 agent 共用同一实例。
- 检索两腿：稀疏（词法 / BM25）＋稠密（语义 ANN）；code / session 两语料各有自己的融合，旋钮不互借。
- 配置驱动（`.litecode/engines.json`，人类在设置页开关）；configured ≠ Warm。
- 横向可拓：新引擎遵循同一模型。

框架展开：[@ key="config"]。
关键契约：[@ key="engine-lifecycle"]、[@ key="retrieval-contract"]。
上层：[@ key="product-brief"]。
