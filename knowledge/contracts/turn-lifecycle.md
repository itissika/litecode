```node
node : turn-lifecycle
status : pending
summary : turn 契约：先预占再启动；seam 接收输入；取消是收尾而不是抹掉已发生的事。
x : 510
y : 530
w : 398
h : 720
```

**turn 是 session 上由内核持有的执行单元，不由 tab 或网络连接持有。**

### 数据
- `SessionActivity` 区分 Idle、StartingTurn、RunningTurn 和 Exclusive 操作；`turn_id` 标识这次执行。
- `TurnProgress` 是进程内进度；`turn/start`、`turn/end` 是持久业务事实。
- pending 用户消息在排队时只是内存输入，消费并提交后才成为日志行。

### 流转
reserve → spawn → start → 写 turn/start、输入 → 每步 seam → compact／视图／模型 → 提交输出 → 工具／提交结果 → 封口 → 写 turn/end → finish／完成通知。

### 不变量
- 同一 session 只能持有一个忙碌活动；预占必须先于启动，启动失败释放 reservation。
- seam 是运行中输入与提醒进入下一次请求的边界；队列只由匹配 turn 且未 stopping 的执行者 claim，落库失败要归还输入。
- 模型输出先提交，再执行其中完整的 tool call；工具结果再提交，下一请求从真源派生。
- 取消不擦除已产出内容或已完成副作用；未执行／中断的调用补结果，避免下次请求面对悬空 call。
- 收尾尝试封住残留在途行、持久 turn/end，再释放 RunningTurn 并发布完成；封口或 turn/end 写失败当前只 warn，仍继续 finish，因此完成通知不是终态已持久化的证明。
- finish 按 turn_id 核对所有权；排队回退可在收尾时接管 Exclusive 租约，旧 turn 不能释放新 turn。

依据：[@ file="src/session/manager.rs" symbol="impl SessionManager › fn reserve_turn" label="预占"]、[@ file="src/session/manager.rs" symbol="impl SessionManager › fn finish_turn" label="所有权交接"]、[@ file="src/runtime/exec.rs" symbol="impl AgentDeps for AgentRuntime › fn sync_request_seam" label="seam 输入"]、[@ file="src/agent/core.rs" symbol="fn run" label="执行与取消顺序"]。
验证：[@ file="tests/session_turn_entry.rs" label="turn 入口测试"]、[@ file="tests/pending_messages.rs" label="排队输入测试"]。

上层：[@ id="session" label="session"]。
