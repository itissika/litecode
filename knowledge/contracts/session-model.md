```node
node : session-model
status : pending
summary : 会话数据：业务日志为真源；seq 是身份，Item 是载荷，Surface 是派生序。
x : 34
y : 530
w : 452
h : 628
```

**会话记录产品事实，不把产品语义塞进 provider 消息格式。**

### 数据
- `SessionLogRow` 的 `kind / body` 表达业务，`seq` 在一个 session 内标识行与顺序，`state` 表达存储生命周期；`cites` 记录来源行。
- `Item` 是 `item/*` 行的结构化载荷及 AgentView 的输出；reminder、turn、request、compacted 各有自己的业务正文。
- `SessionMeta` 保存会话身份、绑定及持久指针；运行中 turn、连接与引擎状态不属于它。

### 流转
业务事实 → 存储分配 seq、写日志 → 折叠 Surface → 模型工作集／人类日志视图／检索语料。

### 不变量
- 行身份是 `(session_id, seq)`，不是 provider item id、数组下标或文本；分配高水位不回退，truncate 后不复用删掉的 seq。
- 行的 `state` 由写入命令决定，不能从载荷的 `status` 推断；无 status 的流式载荷也可以在途。
- Surface 是日志的可重建派生序，不是第二份持久历史；控制面行不进入模型对话序列。
- compact 追加摘要并遮蔽早期 Surface 区间，不删除被摘要的历史日志。
- 未知、非 ignorable 的事件拒绝折叠，不能静默跳过影响语义的事实。

依据：[@ file="src/session/model.rs" symbol="struct SessionLogRow" label="业务 envelope"]、[@ file="src/session/event.rs" symbol="enum EventType" label="事件分类"]、[@ file="src/session/surface.rs" symbol="fn plan_surface" label="折叠规则"]。
验证：[@ file="tests/wire_seq_identity.rs" symbol="fn revert_then_append_does_not_reuse_deleted_seq" label="回退不复用身份"]。

上层：[@ id="session" label="session"]。
