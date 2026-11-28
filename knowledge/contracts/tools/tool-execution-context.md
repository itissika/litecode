```node
node : tool-execution-context
status : enabled
summary : 模型只给参数；工作区、权限模式、取消、会话这些事实由执行器装一次，工具不许自己再拼一份。
x : 2124
y : 378
w : 480
h : 600
domain : tools-permission
```

**解决什么：** 模型只会给参数；真跑起来还得知道在哪个工作区、能不能出沙箱、什么时候该停、属于哪个会话。这批事实模型碰不到，由执行器装成一份交给工具，工具不必自己找。[ToolExecutionContext](src/tool/trait_.rs#ToolExecutionContext)

**必须做到：** 每次调用只装一次，而且装的是真事实——工作区根取本轮上下文，路径模式取权限引擎对这个工具的判定，取消是 turn 的子令牌（工具超时只停自己，turn 取消会往下传）。[executor.rs:697-709](src/tool/executor.rs#L697-L709)

**绝对不允许：** 工具自己再拼一份上下文；测试兜底里那份 `path_mode: All` 尤其不能当生产入口。[Tool.call](src/tool/trait_.rs#Tool.call)

上游：[tool-control-boundary](knowledge/contracts/tools/tool-control-boundary.md)。相关：[tool-scheduling](knowledge/contracts/tools/tool-scheduling.md)、[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)。
