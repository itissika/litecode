```node
node : session-model
status : enabled
domain : harness-core
summary : 会话数据：业务日志为真源；seq 是身份，Item 是载荷，Surface 是派生序。
x : 34
y : 556
w : 416
h : 430
```

**目标：** 会话就是一本事实日志——每件事按顺序写下一行，身份和顺序都长在日志上，不在第二套消息类型里。

**必须做到：**

- 每行以 `(session_id, seq)` 定位；`seq` 单调分配、不回退，删行之后那个 seq 永不复用。
- `item/*` 行装的是 Responses Item，也就是模型看到的东西；提醒、压缩、控制面各有各的业务正文。
- 对话视图（Surface）从日志算出来：按 seq 应用追加和替换，压缩用屏蔽区间表达。是计算，不是第二份存储。
- 控制面的行不进模型对话序列。
- 数据只收业务事实：加一列、加一种行之前先问三句——这个需求成立吗？已有数据够用吗？这是事实还是推导？问不出「必须落库」就不加。
- 真要存派生值（计数、摘要），它是缓存：能从真源重算，永远不当真源读。
- 写只有一条路（见 [session-write-path](knowledge/contracts/session/session-write-path.md)）：一个 writer、同事务落地、幂等先于版本。

**绝对不做：**

- 平行消息类型（Message / ContentBlock 方言）——身份和顺序必须来自日志。
- 把 Surface 存成第二份数据。
- 复用一个已经删掉的 seq。
- 覆写已经封口的正文；回退是删行，不是改写。
- 把「提交之后收信失败」当成「没提交」：COMMIT 可能已经落库，只能凭操作身份查回。
- 预留字段、备用 JSON、杂物列——「以后可能用得上」不是理由。
- 把推导结果当事实写进日志，再当真相读回来。

依据：[session.rs](src/session/data/sqlite/session.rs)、[writer.rs](src/session/data/writer.rs)、[schema.rs](src/session/data/sqlite/schema.rs)。契约展开：[session-log-envelope](knowledge/contracts/session/session-log-envelope.md)。上层：[session](knowledge/framework/session.md)。相关：[turn-lifecycle](knowledge/contracts/agent/turn-lifecycle.md)、[client-projection](knowledge/contracts/client/client-projection.md)。
