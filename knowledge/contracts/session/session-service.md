```node
node : session-service
status : enabled
summary : 会话业务只有一个入口：业务动作走 SessionService，SessionManager 只当原子原语。
x : 5688
y : 720
domain : harness-core
```

**目标：** 会话业务只有一个入口——业务动作走 `SessionService`，`SessionManager` 只当原子原语，谁也别在外面自己拼一套回合。

**必须做到：**

- 启动、停止、回退、手动压缩、队列流转统一走 `SessionService`；UI、subagent、后台自动回合都算在内。
- `SessionManager` 只管原语：预占、排他租约、seq 与写入、收尾；service 只组合它们，不再造第二条写入路径或第二份会话状态。
- 业务面和支撑面分清楚：生命周期、对话回合、权限、观测属于业务面；投影读取、runtime 构造、压缩 worker、启动/GC、权限路由、自动回合装配属于支撑面。
- 自动回合藏在内部：外部只看到发消息、停止、订阅、应答权限，看不到「启动自动回合」这个动作。
- 回退的租约要盖住整个动作：会话回退到投影刷新和帧发出之后，文件回退到工作区恢复和结果帧之后；手动压缩先拿到独占 episode 和全部输入，再交给 worker。
- 对外契约只映射业务面，不映射支撑面。

**绝对不做：**

- 在 UI、subagent、自动回合里各自组装回合启动——预占、失败释放、pending 清理会立刻分叉。
- 让 service 自己写库，或者把 `SessionManager` 变成第二个业务入口。
- 把支撑面（runtime 句柄、压缩 worker、自动回合计时器）暴露成外部 API。
- 让外部拥有「什么时候自动继续」的决定权。

依据：[SessionService](src/session/service.rs#SessionService)、[SessionService.start_turn](src/session/service/conversation.rs#SessionService.start_turn)、[SessionService.revert_to_user_anchor](src/session/service/conversation.rs#SessionService.revert_to_user_anchor)、[SessionService.begin_file_revert](src/session/service/conversation.rs#SessionService.begin_file_revert)、[SessionService.start_manual_compaction](src/session/service/conversation.rs#SessionService.start_manual_compaction)、[SessionManager](src/session/manager.rs#SessionManager)、[session_layer_does_not_absorb_the_hub](tests/architecture_layers.rs#session_layer_does_not_absorb_the_hub)。

上层：[session](knowledge/framework/session.md)。
相关：[session-write-path](knowledge/contracts/session/session-write-path.md)、[turn-reservation](knowledge/contracts/session/turn-reservation.md)、[turn-lifecycle](knowledge/contracts/agent/turn-lifecycle.md)。
