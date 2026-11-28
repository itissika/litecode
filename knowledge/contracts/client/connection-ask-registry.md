```node
node : connection-ask-registry
status : enabled
summary : 提问等的是 agent 线程，不是连接；没人回答记中止，不记拒绝。
x : 5094
y : 2448
domain : client-ux
```

**目标：** 一次权限询问由 agent 线程等回执，连接只负责转发。人没点，就不能写「他拒绝了」——那是另一件事。

**必须做到：**

- 等待拆两半：agent 在 bridge 上等回执，握着 `reply_tx`；连接只持登记表 `pending_asks`，绝不在连接循环里陪着等。[WsPermissionBridge.PermissionSink.ask](src/client_protocol/permission_bridge.rs#WsPermissionBridge.PermissionSink.ask)、[PendingAsk](src/client_protocol/connection.rs#PendingAsk)
- 每条问询的 `request_id` 由 bridge 现生成，回执只认 id，不认工具名或会话名。
- 只有那条 turn 还活着才登记；来晚了就当场中止，连抢跑到的 grant 一起丢掉，免得快照重建出一张没人等的卡。[SessionManager.upsert_pending_permission](src/session/manager.rs#SessionManager.upsert_pending_permission)
- 人点了：按 id 取回登记、发回执；只有勾了「总是允许」才顺手记一条运行时许可。[apply_permission_grant](src/client_protocol/connection.rs#apply_permission_grant)
- 对不上号的 grant 先存着（`stray_grants`），等对应的 Ask 到了再配对，不许悄悄丢。[take_stray_grant_for](src/client_protocol/connection.rs#take_stray_grant_for)
- 人没点就结束了：停止或取消一律回中止并清掉库里的待办卡；断连或退出扔掉回执通道，agent 侧当中止，同时清掉待办行。[abort_all_pending_asks](src/client_protocol/connection.rs#abort_all_pending_asks)、[retain_pending_on_disconnect](src/client_protocol/connection.rs#retain_pending_on_disconnect)、[retain_and_clear_pending](src/client_protocol/connection.rs#retain_and_clear_pending)

**绝对不允许：**

- 把「没人回答」记成「拒绝」：`Deny` 是人看了说不，`Aborted` 是没人回答；混了以后审计记录、`always` 许可、重连后的卡片都会说假话。[AskOutcome](src/permission/grants.rs#AskOutcome)
- 连接抓着回执一起等——一条 WS 会死在一次提问上，别的会话也跟着收不到消息。
- 丢掉对不上号的 grant——两次点头会串到对方的工具上。

依据：[retain_pending_on_disconnect_does_not_deny](src/client_protocol/connection.rs#retain_pending_on_disconnect_does_not_deny)、[concurrent_asks_resolve_independently](src/client_protocol/connection.rs#concurrent_asks_resolve_independently)。

UI 表面见 [ask-surface](knowledge/contracts/agent/ask-surface.md)；授权求值见 [tool-authorization](knowledge/contracts/tools/tool-authorization.md)。上层：[client-projection](knowledge/contracts/client/client-projection.md)。
