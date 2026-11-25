```node
node : connection-ask-registry
status : pending
summary : Ask 阻塞的是 agent，不是连接：request_id→reply_tx 登记；断线一律 Deny。
x : 5094
y : 2448
domain : client-ux
```

**准入：** A2 / A5。权限/提问等待必须挂在连接侧的 pending 表上，连接循环不得 await `reply_tx`；agent 线程在 bridge 上阻塞。Grant 按 request_id 配对；未知 grant 不得静默丢弃；断开/取消/退出对所有未决 Ask 回 Deny。坏了会出现整条 WS 卡死或回执串台。UI 表面见 ask-surface；授权求值见 tool-authorization。

依据：[PendingAsk](src/client_protocol/connection.rs#PendingAsk)、[apply_permission_grant](src/client_protocol/connection.rs#apply_permission_grant)、[reject_all_pending_asks](src/client_protocol/connection.rs#reject_all_pending_asks)、[WsPermissionBridge.PermissionSink.ask](src/client_protocol/permission_bridge.rs#WsPermissionBridge.PermissionSink.ask)、[connection.rs](src/client_protocol/connection.rs)。
上层：[client-projection](knowledge/contracts/client-projection.md)。
相关：[ask-surface](knowledge/contracts/ask-surface.md)、[tool-authorization](knowledge/contracts/tools/tool-authorization.md)。
