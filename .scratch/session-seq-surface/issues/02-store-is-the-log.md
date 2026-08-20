# 02: 持久化权威是事件日志（拆分）

原单票过大：schema、append 落盘、compact=replace、关掉 SQL 重排绑在一起，中间态会让 loop 窗口既不是指针也不是 surface。

**不要双写** `surface_op: replace` 与 `checkpoint_seq` 窗口。compact 与 turn-load 必须同一张票收口。

拆成：

| 票 | 文件 | 做什么 | 仍允许残破 |
|---|---|---|---|
| 02a | [02a-schema-event-envelope.md](./02a-schema-event-envelope.md) | 砸库加信封列 | compact 仍是 checkpoint |
| 02b | [02b-persist-load-events.md](./02b-persist-load-events.md) | append-origin 按 seq 读写 | turn 窗口仍走旧 SQL |
| 02c | [02c-compact-replace-and-fold-load.md](./02c-compact-replace-and-fold-load.md) | replace 落盘 + fold 加载，指针不再当窗口 | loop 内存工作集仍可能是 `Vec<Item>`（03） |

03 阻塞于 **02c**，不是 02a。
