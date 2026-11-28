```node
node : turn-lifecycle
status : enabled
summary : 一轮对话归内核所有：输入只从请求缝隙进来，先落库再执行，取消只是收尾不是抹掉。
x : 522
y : 558
w : 398
h : 414
domain : harness-core
```

**目标：** 一轮对话是内核持有的执行单元——输入只从请求缝隙进来，先落库再干活，取消只是收尾，不是把发生过的事抹掉。

**必须做到：**

- 归内核所有，不归 tab 或连接：界面关了、连接断了，这一轮照常收尾。
- 一次一个，先占位再启动；启动失败立刻还位（见 [turn-reservation](knowledge/contracts/session/turn-reservation.md)）。
- 输入只走一个缝：运行中的用户消息和系统提醒都经请求缝隙进入下一次请求；排队消息在落库前只是输入，落库失败要还回去。
- 先落库再执行：模型输出先提交，再执行其中完整的工具调用；工具结果提交后，下一步从真源重新派生视图。
- 取消是收尾：已产出的内容、已完成的副作用都不擦；没执行或被打断的调用补一条中断结果，不留悬空调用。
- 收尾按 turn 身份核对所有权，回退可以接管；即使终态落库失败，也要发布完成通知。

**绝对不做：**

- 把 turn 绑在 tab 或连接上——关个界面就把一轮杀掉。
- 让输入绕过缝隙直接改模型上下文。
- 取消时删掉已产出的内容，或回滚已经发生的副作用。
- 留下没有结果的工具调用。
- 把「已经发布完成」当成「终态已经落库」。

依据：[run](src/agent/core.rs#run)、[persist_or_stop](src/agent/core.rs#persist_or_stop)、[interrupted_outputs](src/agent/core.rs#interrupted_outputs)、[sync_request_seam](src/runtime/exec.rs#sync_request_seam)、[SessionManager.finish_turn](src/session/manager.rs#SessionManager.finish_turn)、[emit_turn_completed](src/runtime/mod.rs#emit_turn_completed)。

上层：[session](knowledge/framework/session.md)。
