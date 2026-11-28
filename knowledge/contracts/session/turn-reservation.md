```node
node : turn-reservation
status : enabled
summary : 一个会话同一时刻只干一件事：开始前占位，结束或失败都还位，还位只认自己的轮次。
x : 5094
y : 2952
domain : harness-core
```

**目标：** 一个会话同一时刻只干一件事——开始前先占位，结束或失败都要还位，还位只认自己的那一轮。

**必须做到：**

- 占位与进程级计数配对：预占时 `TurnGuard` +1，释放时 −1，一进一出不落单。
- 状态三段：Idle → StartingTurn（已占位、还没跑）→ RunningTurn（真在跑）；只有占位成功的那一轮能进 RunningTurn。
- 启动失败也要还位——spawn 失败、提前返回，都要把占位和计数退回去。
- 收尾按 turn_id 核对：只关自己那一轮，对不上就什么都不改。
- 取消、回退走 Exclusive，与 RunningTurn 互斥：先等这一轮结束再接管。
- 已经在忙时再来一轮直接拒绝，不排队、不覆盖。

**绝对不做：**

- 占位后漏释放：会话会被永久堵死，后面任何一轮都进不来。
- 不核对 turn_id 就改状态：那会误关别人的轮次。
- 用第二套「忙」的表示法——忙不忙只认这一份状态。

依据：[SessionActivity](src/session/manager.rs#SessionActivity)、[SessionManager.reserve_turn](src/session/manager.rs#SessionManager.reserve_turn)、[SessionManager.release_turn_reservation](src/session/manager.rs#SessionManager.release_turn_reservation)、[SessionManager.start_turn](src/session/manager.rs#SessionManager.start_turn)、[SessionManager.finish_turn](src/session/manager.rs#SessionManager.finish_turn)、[TurnGuard.begin_turn](src/config/turn_guard.rs#TurnGuard.begin_turn)、[TurnGuard.end_turn](src/config/turn_guard.rs#TurnGuard.end_turn)。

上层：[turn-lifecycle](knowledge/contracts/agent/turn-lifecycle.md)。
