```node
node : connection-ask-registry
status : pending
domain : client-ux
summary : Ask 阻塞的是 agent，不是连接：request_id→reply_tx 登记；断线一律 Deny。
```

**准入：** A2 / A5。权限/提问等待必须挂在连接侧的 pending 表上，连接循环不得 await `reply_tx`；agent 线程在 bridge 上阻塞。Grant 按 request_id 配对；未知 grant 不得静默丢弃；断开/取消/退出对所有未决 Ask 回 Deny。坏了会出现整条 WS 卡死或回执串台。UI 表面见 ask-surface；授权求值见 tool-authorization。

依据：[@ file="src/client_protocol/connection.rs" symbol="struct PendingAsk"]、[@ file="src/client_protocol/connection.rs" symbol="fn apply_permission_grant"]、[@ file="src/client_protocol/connection.rs" symbol="fn reject_all_pending_asks"]、[@ file="src/client_protocol/permission_bridge.rs" symbol="impl PermissionSink for WsPermissionBridge › fn ask"]。
上层：[@ key="client-projection"]。
相关：[@ key="ask-surface"]、[@ key="tool-authorization"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b；修复提交 74f2bb1）
- Ask 阻塞 agent、不阻塞连接：`request_id → reply_tx` 登记，主循环继续处理各 session 的 JsonRpc —— [@ file="src/client_protocol/connection.rs" symbol="struct PendingAsk"]、[@ file="src/client_protocol/connection.rs" symbol="fn apply_permission_grant"]；循环注释明确无嵌套 select 排队异会话 RPC。
- 与 Andy 口径对齐：**一 session 只管自己的 Ask**；跨 session 整条 WS 串行 = 错（已在 74f2bb1 修掉）。测试见 [@ file="src/client_protocol/connection.rs" symbol="mod ask_loop_tests › fn ask_does_not_block_other_session_rpc"]。
- 断线/取消/退出对未决 Ask 回 Deny —— [@ file="src/client_protocol/connection.rs" symbol="fn reject_all_pending_asks"]；未知 grant 进 stray 不静默丢。ISSUE-019 / DRAFT-CUX-1 = closed。
