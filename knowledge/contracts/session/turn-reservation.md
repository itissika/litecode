```node
node : turn-reservation
status : pending
summary : turn 预占状态机：Idle→StartingTurn→RunningTurn；启动失败必须释放；收尾按 turn_id 核对。
x : 5094
y : 2952
domain : harness-core
```

**准入：** A2。同一 session 同时只允许一个忙碌活动；预占与 config `TurnGuard` 成对增减。Spawn 失败不释放会永久堵死会话；收尾不核 turn_id 会误关别人的轮次。取消/回退可进 Exclusive，与 RunningTurn 互斥。

依据：[@ file="src/session/manager.rs" symbol="enum SessionActivity"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn reserve_turn"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn release_turn_reservation"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn start_turn"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn finish_turn"]、[@ file="src/config/turn_guard.rs" symbol="impl TurnGuard › fn begin_turn"]。
父节点：[@ key="turn-lifecycle"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 状态机 Idle→StartingTurn→RunningTurn（及 Exclusive）—— [@ file="src/session/manager.rs" symbol="enum SessionActivity"]。
- 预占 / 释放 / 启动 —— [@ file="src/session/manager.rs" symbol="impl SessionManager › fn reserve_turn"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn release_turn_reservation"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn start_turn"]。
- 收尾核 turn_id —— [@ file="src/session/manager.rs" symbol="impl SessionManager › fn finish_turn"]；与 [@ file="src/config/turn_guard.rs" symbol="impl TurnGuard › fn begin_turn"] 成对增减。
- 回归测试名（同 manager 测试模块）：`start_turn_fanout_spawn_failure_rolls_back_running_and_turn_guard`、`finish_turn_clears_running_immediately_and_allows_rebegin`。与 agent-loop 分层，不复述主环主张。
