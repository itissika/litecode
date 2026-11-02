```node
node : connection-ask-registry
status : pending
summary : Ask 阻塞的是 agent，不是连接：request_id→reply_tx 登记；断线一律 Deny。
x : 5094
y : 2448
domain : client-ux
```

**准入：** A2 / A5。权限/提问等待必须挂在连接侧的 pending 表上，连接循环不得 await `reply_tx`；agent 线程在 bridge 上阻塞。Grant 按 request_id 配对；未知 grant 不得静默丢弃；断开/取消/退出对所有未决 Ask 回 Deny。坏了会出现整条 WS 卡死或回执串台。UI 表面见 ask-surface；授权求值见 tool-authorization。

依据：[@ file="src/client_protocol/connection.rs" symbol="struct PendingAsk"]、[@ file="src/client_protocol/connection.rs" symbol="fn apply_permission_grant"]、[@ file="src/client_protocol/connection.rs" symbol="fn reject_all_pending_asks"]、[@ file="src/client_protocol/permission_bridge.rs" symbol="impl PermissionSink for WsPermissionBridge › fn ask"]、[@ file="src/client_protocol/connection.rs"]。
上层：[@ key="client-projection"]。
相关：[@ key="ask-surface"]、[@ key="tool-authorization"]。
