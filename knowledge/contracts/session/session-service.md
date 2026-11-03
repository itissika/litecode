```node
node : session-service
status : enabled
summary : 会话业务面：UI、subagent 与后台自动回合统一走 SessionService；SessionManager 仍是原子原语与唯一写者。
domain : harness-core
```

**一句话：** 会话业务只有一个入口；原生消费者通过 `SessionService` 组装业务动作，底层 `SessionManager` 只保留预占、锁、写入和生命周期原语。

## 守什么

1. **一个业务面。** UI、subagent、后台自动回合的启动、停止、回退、手动压缩和队列流转都走 `SessionService`；生产代码不直接持有 `SessionManager` 做这些回合编排。工具自己读写 plan/todo 等数据仍走 manager 的数据 API，不属于这条业务入口。
2. **manager 只做原语。** `SessionManager` 继续拥有预占状态、排他租约、seq/写入和收尾；service 只组合这些原语，不产生第二套写入路径、第二份会话状态或第二个 writer。
3. **业务面与支持面分开。** 业务面是生命周期、对话与回合、权限交互、观测与历史；Projection 读取、runtime 构造、压缩 worker、启动/GC/shutdown、权限路由和自动回合的装配属于内部支持面，不是未来外部契约。
4. **自动回合是内部实现。** bash 完成、custom tool 完成、子代理完成和待发消息触发的是内部 auto-turn；外部契约只应看到发消息、停止、订阅和权限应答，不暴露“启动自动回合”这个动作。
5. **回退的租约覆盖完整动作。** 会话回退持有排他租约到投影刷新和帧发出之后；文件回退持有文件回退 plan 到工作区恢复和结果帧发出之后。手动压缩是一次 service 业务动作，先完成独占 episode 和全部输入，再交给 worker。
6. **外部契约只映射业务面。** 未来窄外部契约应映射到 service 的业务操作；不能把 support 方法、runtime 句柄或自动回合计时器直接变成外部 API。

## 为什么关键

- 如果 UI、subagent 和后台自动回合各自组装回合启动，预占、失败释放、pending 清理和租约覆盖会分叉，最先坏的是同一 session 的并发语义。
- 如果支持面混进外部契约，外部会被迫理解 runtime、compaction 和 projection 的内部机制，等于把内部编排固化成公开协议。
- auto-turn 一旦对外暴露，外部就会拥有“何时自动继续”的策略权，和“发消息/停止”的用户语义重叠；它应留在内部触发器后面。

## 证据

- 业务与支持边界：[@ file="src/session/service.rs" lines="1-17"]、[@ file="src/session/service.rs" lines="56-72"]。
- 对话、停止、回退与手动压缩：[@ file="src/session/service/conversation.rs" lines="60-100"]、[@ file="src/session/service/conversation.rs" lines="177-303"]。
- 统一回合启动：[@ file="src/session/service/conversation.rs" lines="305-377"]。
- 自动回合的内部装配：[@ file="src/session/service/support.rs" lines="272-320"]、[@ file="src/session/service/support.rs" lines="332-397"]、[@ file="src/runtime/bash_auto_turn.rs" lines="29-73"]、[@ file="src/runtime/pending_auto_turn.rs" lines="48-160"]。
- UI 投影只读 service：[@ file="src/client_protocol/controller/mod.rs" lines="68-72"]。
- 回退租约覆盖：[@ file="src/client_protocol/controller/session_lifecycle.rs" lines="433-448"]、[@ file="src/client_protocol/controller/session_lifecycle.rs" lines="485-600"]。
- 进程装配：[@ file="src/serve/state.rs" lines="133-165"]。
- 分层守卫：[@ file="tests/architecture_layers.rs" lines="114-189"]。

`SessionManager` 的唯一写者边界仍由 [@ key="session-write-path"] 约束；turn 预占状态机仍由 [@ key="turn-reservation"] 约束。

上层：[@ key="session"]。
相关：[@ key="turn-lifecycle"]、[@ key="subagent-lifecycle"]、[@ key="client-projection"]、[@ key="compact-pipeline"]。
