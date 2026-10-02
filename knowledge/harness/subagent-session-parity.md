```node
node : subagent-session-parity
status : pending
summary : 子 agent 即完整会话：同 turn 原语、持久 child session、depth=1 不嵌套、事件按自身 session 投影。
x : 748
y : 1044
```

**child 是完整 session，不是父 turn 的附属品。**

- 同原语：人类 turn 与 subagent tool 都走 reserve → spawn → start；stop 走 cancel；Hub 只是完成路由器，不拥有 turn 生命周期 [@ file="docs/adr/0003-subagent-session-ownership.md" label="ADR 0003"]。
- 持久 child：`parent_session_id` / `parent_call_id` 写在会话记录里；child 事件按自身 session_id 分发 [@ file="src/tools/subagent/mod.rs" label="mod.rs"]。
- 深度锁是产品：上限 1（SUBAGENT_MAX_DEPTH），子代理不可再嵌套；depth > 0 不暴露 subagent 系列 tool。
- 无 per-parent 并发帽——并发不是产品规则（ADR 明确否决）。
- 投影：子代理挂在父会话 launch item 下，child 事件不转发进父 turn 通道 [@ id="process-visibility" label="过程可见"]。


