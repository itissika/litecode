# WP-183-a brief（runtime）— Obs-1 + Obs-2
**board id:** WP-188

**parent:** WP-183  
**owner:** runtime  
**approver:** backend_mid  
**domain:** harness-core  
**artifact:** `.litecode/ops/artifacts/WP-183/183-a/`

## Goal
- Obs-1：panic hook + diagnostics 目录约定 + 工具审计（observe-only）
- Obs-2：TurnError/GiveUp 关联 wire 路径；失败保留说明；Luna+grep runbook 写入 artifacts

## Non-goals
- 不修 H1 codec
- 不改权限桥 / AskOutcome（WP-181）

## Heavy
`LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`；与 WP-181 互斥排槽；报 backend_mid

## Done when
见父 plan §6；runbook 与验收记录落 `.litecode/ops/artifacts/WP-183/`
