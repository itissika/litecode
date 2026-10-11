```node
node : custom-tool-hub
status : pending
domain : tools-permission
summary : Custom 后台作业：begin_job 登记；完成经 mailbox 投递；取消可杀进程。
```

**准入：** A2/A5。选参后台时 `CustomToolHub` 用 job_id 登记在飞作业，完成后 `push_notice`；session 侧 `take_mailbox` / `restore_mailbox` 与 seam 提醒对接。同步路径仍走普通 Tool 结果。hub 是完成事件的唯一投递口，丢 notice 或重复投会让 agent 收不到/收到两次结算。

依据：[@ file="src/tools/custom_hub.rs" symbol="impl CustomToolHub › fn begin_job"]、[@ file="src/tools/custom_hub.rs" symbol="impl CustomToolHub › fn push_notice"]、[@ file="src/tools/custom_hub.rs" symbol="impl CustomToolHub › fn take_mailbox"]、[@ file="src/tools/custom_hub.rs" symbol="impl CustomToolHub › fn cancel_job"]、[@ file="src/tools/custom_hub.rs" symbol="impl CustomToolHub › fn restore_mailbox"]。
父节点：[@ key="tool-adapters"]。
相关：[@ key="tool-call-lifecycle"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- begin_job / push_notice / take_mailbox / restore_mailbox / cancel_job / mailbox_pending —— [@ file="src/tools/custom_hub.rs" symbol="impl CustomToolHub › fn begin_job"] 等 API 齐全。
- session 侧 drain → CustomToolSettled 提醒 —— [@ file="src/runtime/exec.rs"]（take_mailbox → CustomToolSettledEntry）；自动回合 [@ file="src/runtime/custom_auto_turn.rs"]。
- 单元：hub mailbox 去重/restore；reminder kind 对齐。TP-B2-01 联调疑点关闭（字段路径已接线）。
