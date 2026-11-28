```node
node : stream-projection
status : enabled
summary : 一次调用的输出只落一份：流里开行、就地更新、返回时封口。
x : 5364
y : 1458
domain : harness-core
```

**目标：** 一次模型调用的输出只落一份记录——流式内容到达时先开一行、就地更新，调用返回时用它自己的副本封口。

**必须做到：**

- 一次调用持有自己的一张映射：provider 的 item id → 日志 seq，只在这条调用里有效。
- 一个 item 一辈子只有一行：第一次知道它时开（added、首个 delta，或终态补开），之后所有增量只更新这一行。
- 封口用调用返回的那份 Item，不用流里最后看到的内容。
- 写入失败就让整次调用失败——不能「模型调用成功、日志没落全」。

**绝对不做：**

- 流和返回值各写一遍：那是同一件内容两个身份，读者会看到错序的推理，reload 也修不回来。
- 拿流里的内容冒充完整终态；没有权威列表时可以用它封口，但必须标成 incomplete。
- 吞掉写入错误，让调用假装成功。

依据：[StreamProjection](src/runtime/stream_projection.rs#StreamProjection)、[StreamProjection.settle](src/runtime/stream_projection.rs#StreamProjection.settle)、[death_list_gate.rs](src/types/death_list_gate.rs)。

上层：[turn-lifecycle](knowledge/contracts/agent/turn-lifecycle.md)。相关：[llm-stream-contract](knowledge/contracts/llm/llm-stream-contract.md)。
