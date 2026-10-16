# WP-181-b brief（runtime）— PhasePermissionSink / phase 对齐
**board id:** WP-187

**parent:** WP-181  
**owner:** runtime  
**approver:** backend_mid  
**domain:** harness-core  
**artifact:** `.litecode/ops/artifacts/WP-181/181-b/`

## Goal
在 WP-181-a 冻结 snapshot/挂起权威之后：
- `PhasePermissionSink` / observer：进入挂起 ↔ `waiting_for_user` / pending 列表一致
- Allow / Deny / Aborted 离开时 phase 与 turn 边界正确（等待态不 finished；Cancel 后 finished）
- 不另起 ask 通道；不改 FE

## Place
`src/{runtime,agent}`（phase.rs、observer 等）；**勿**并行改 connection/permission_bridge（归 181-a）

## Depends
WP-181-a ready/in_progress 完成协议+落盘后开工（串行）

## Done when
- phase/sink 与 pending 列表一致；相关测绿；无 H1 codec；无权限桥断线大改（已由 a 完成）
