```node
node : subagent-lifecycle
status : enabled
summary : 子会话契约：持久 parent/call 关系，独立 turn；Hub 只路由完成，父取消不隐式取消 child。
x : 990
y : 692
w : 398
h : 396
```

**主张：** subagent 是完整 session；调度工具是同一 turn 基座的另一种入口。

- 同一原语：launch / send 与人类消息走同一条预占与启动路径，stop 走取消；Hub 只是完成路由器，不持有运行真相或生命周期。
- 完整会话：child 有自己的持久会话行（父子关系与深度记在其中）与独立执行配置；其事件按自身 session 分发，不混入父 turn。
- 深度是产品规则：上限一层、不可嵌套；不设 per-parent 并发上限。
- 取消不传播：父 turn 取消不连带取消 child；停止必须显式指向对应会话。
- 完成靠事实与唤醒：结果来自 child 的持久记录；运行中的父在请求缝隙收到完成提醒，空闲父满足条件时被唤醒消费，不靠隐形注入。

上层：[@ key="featured-tools"]。
依据：[@ file="src/tools/subagent/turn.rs"]、[@ file="src/tools/subagent/hub.rs"]。
