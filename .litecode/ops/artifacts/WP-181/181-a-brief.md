# WP-181-a brief（session）— 协议 / 落盘 / snapshot / 断线语义
**board id:** WP-186

**parent:** WP-181  
**owner:** session  
**approver:** backend_mid  
**domain:** harness-core  
**artifact:** `.litecode/ops/artifacts/WP-181/181-a/`

## Goal
落地 WP-179 §FE终审：
- 对外 `waiting_for_user`；对内可保留 `AwaitingPermission` 扩载荷
- 权限类（ask_user|permission|approval）tool_call 落盘故意不封口
- startup：权限类未封口 → 恢复等待；非权限类仍 seal interrupt
- snapshot/attach：`pending_permissions` 与 `permission_request` 1:1，`kind` 必填；absent=[]
- 断线 ≠ Deny（保留挂起）；用户 Skip=Deny；Cancel=Aborted + turn_finished（WS 连着必达）
- 等待态 turn 未 finished；`awaiting_permission` 布尔派生可选

## Place
`src/{session,client_protocol,serve}`（含 connection `reject_all_pending_asks`、permission_bridge、protocol snapshot、writer seal 分流）

## Done when
- 上列行为有测或协议测；产物说明变更面
- 不改 H1 codec；不改 web/desktop；权限桥断线语义按上列（非 WP-183）

## Serial
先于 WP-181-b；完成后通知 backend_mid / FE（WP-182）可联调 snapshot。
