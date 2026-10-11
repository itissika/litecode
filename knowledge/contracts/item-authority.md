```node
node : item-authority
status : pending
domain : harness-core
summary : 会话原子唯一权威：Responses Item / StreamEvents；禁止平行 Message/ContentBlock 方言。
```

**准入：** A7。内核 transcript 与流事件必须是 `async_openai::types::responses` 经 `authority` 再导出的类型；再定义一套 Message/ContentBlock/自制 StreamEvent 会把日志真源拆成两套，视图、投影、回放全部漂移。死亡清单与 `types/transcript` 硬规则守门。

依据：[@ file="src/types/transcript.rs" symbol="fn user_message"]、[@ file="src/types/death_list_gate.rs" symbol="fn death_list_dialect_tokens_absent_from_src"]。
上层：[@ key="agent-harness-philosophy"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- Responses Item 为唯一内核原子 —— [@ file="src/types/transcript.rs" symbol="fn user_message"]；[@ file="src/types/death_list_gate.rs" symbol="fn death_list_dialect_tokens_absent_from_src"]。
- 禁止平行 Message/ContentBlock 方言 —— 死亡清单扫描 src 方言令牌；无平行结构体定义路径。
