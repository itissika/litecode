```node
node : session-write-path
status : enabled
summary : 会话写入守门：一个入口、一次事务、重试不重做、旧版本不覆盖。
x : 5094
y : 2844
domain : harness-core
```

**目标：** 会话库的写只能从一个门进——一次写要么全落地、要么全不落地，重试不重做，旧版本盖不掉新版本。

**必须做到：**

- 一个工作区只有一个 writer 能写会话库；读可以并发，写不能另开一扇门。
- 一条写的所有落地物在同一事务里：业务变更、版本推进、操作回执，缺一个不算成功。
- 同一个 operation_id 再来，先查回执；做过就原样返回，不再执行一遍。
- 版本对不上就显式失败：不静默合并，也不猜。

**绝对不做：**

- 第二个写者：旁路开库、另起写循环、直接改 SQL。
- 只写一半：变更在、回执不在，或回执在、变更不在。
- 跳过回执检查直接重跑——会重复发消息、重复跑工具。
- 拿旧 revision 硬写，盖掉别人刚提交的状态。

依据：单入口 [WriterHandle.submit_blocking](src/session/data/writer.rs#WriterHandle.submit_blocking)；事务内分发与回执 [execute](src/session/data/writer.rs#execute)、[record_commit](src/session/data/sqlite/ops.rs#record_commit)；幂等先于版本 [execute](src/session/data/writer.rs#execute)；禁止旁路开库 [session_data_sql_death_list.rs](tests/session_data_sql_death_list.rs)。

父节点：[session-model](knowledge/contracts/session/session-model.md)。
