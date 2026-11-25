```node
node : turn-reservation
status : pending
summary : turn 预占状态机：Idle→StartingTurn→RunningTurn；启动失败必须释放；收尾按 turn_id 核对。
x : 5094
y : 2952
domain : harness-core
```

**准入：** A2。同一 session 同时只允许一个忙碌活动；预占与 config `TurnGuard` 成对增减。Spawn 失败不释放会永久堵死会话；收尾不核 turn_id 会误关别人的轮次。取消/回退可进 Exclusive，与 RunningTurn 互斥。

依据：[SessionActivity](src/session/manager.rs#SessionActivity)、[SessionManager.reserve_turn](src/session/manager.rs#SessionManager.reserve_turn)、[SessionManager.release_turn_reservation](src/session/manager.rs#SessionManager.release_turn_reservation)、[SessionManager.start_turn](src/session/manager.rs#SessionManager.start_turn)、[SessionManager.finish_turn](src/session/manager.rs#SessionManager.finish_turn)、[TurnGuard.begin_turn](src/config/turn_guard.rs#TurnGuard.begin_turn)。
父节点：[turn-lifecycle](knowledge/contracts/turn-lifecycle.md)。
