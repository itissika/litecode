```node
node : session
status : enabled
summary : session：业务日志（kind / body / seq / state）为真源；单写者、回退与独立子会话。
x : 34
y : 350
w : 308
h : 373
```

- seq 单调分配、身份稳定；在途行可更新并以同 seq 封口，settled 正文不可覆写，回退删行但不复用身份。
- 业务日志表达会话事实；Item 是 item/* 载荷与模型投影，不是全部业务数据。
- 三原语：append / seal / truncate；Surface 从日志折叠。
- 单写者：一工作区一 writer actor + 只读池；消费者走类型化命令，不自行开库。
- subagent session 与 primary session 底层完全等价。

契约展开：[@ id="session-model" label="会话数据模型"]、[@ id="session-write" label="会话写门"]、[@ id="turn-lifecycle" label="turn 生命周期"]。
上层：[@ id="agent-design" label="agent 设计"]。
