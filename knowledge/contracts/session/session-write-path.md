```node
node : session-write-path
status : pending
summary : 会话写入守门：一个入口、一次事务、重试不重做、旧版本不覆盖。
x : 5094
y : 2844
domain : harness-core
```

**一句话：** 会话库的写，只能从一个门进；一次写要么全落地，要么全不落地；重试不重复执行；旧版本不能覆盖新版本。

## 守什么

1. **一个入口。** 一个工作区只有一个 writer actor 能写会话库；读可以并发，写不能另开一扇门。
2. **一次落全。** 业务变更、版本推进、操作回执在同一事务里；不能只写一半。
3. **重试不重做。** 同一个 operation_id 再来，先查回执；已经做过就原样返回，不再执行。
4. **旧版本不覆盖。** expected_revision 对不上就显式失败；不静默合并，也不猜。

## 为什么关键

- 第二写者会让 seq、顺序和回执互相打架。
- 只写一半会出现“变更在、回执不在”或“回执在、变更不在”。
- 重试不查回执，会重复发消息、重复跑工具。
- 旧版本硬写，会把别人刚提交的状态盖掉。

## 怎么起作用

- `submit_blocking` 把写请求排进唯一 writer。
- `execute` 在 `BEGIN IMMEDIATE` 里分发变更，并推进 revision。
- `record_commit` 把回执和变更写在同一事务。
- `execute` 先 `load_operation` 查重，再比 `expected_revision`。

## 证据

- 单入口：[@ file="src/session/data/writer.rs" symbol="impl WriterHandle › fn submit_blocking"]
- 事务内分发与回执：[@ file="src/session/data/writer.rs" symbol="fn execute"]、[@ file="src/session/data/sqlite/ops.rs" symbol="fn record_commit"]
- 幂等先于版本：[@ file="src/session/data/writer.rs" symbol="fn execute"]
- 禁止旁路开库：[@ file="tests/session_data_sql_death_list.rs"]

父节点：[@ key="session-model"]。

审计：本节点服务 A3 / A5 / A7；基线 eb0452a。
