```node
node : item-authority
status : pending
summary : 会话原子唯一权威：Responses Item / StreamEvents；禁止平行 Message/ContentBlock 方言。
x : 5094
y : 1062
domain : harness-core
```

**准入：** A7。内核 transcript 与流事件必须是 `async_openai::types::responses` 经 `authority` 再导出的类型；再定义一套 Message/ContentBlock/自制 StreamEvent 会把日志真源拆成两套，视图、投影、回放全部漂移。死亡清单与 `types/transcript` 硬规则守门。

依据：[user_message](src/types/transcript.rs#user_message)、[death_list_gate.rs](src/types/death_list_gate.rs)。
上层：[agent-harness-philosophy](knowledge/harness/agent-harness-philosophy.md)。
