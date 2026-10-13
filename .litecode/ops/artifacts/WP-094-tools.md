# WP-094-tools · Tools 域实现（WP-100）

- 角色：tools；依据 WP-094-backend.md L1–L6 + `.tmp/wp098_interface.md` v1.1（Runtime 冻结接口）。未 commit；未改 Runtime/Session 文件。

## 变更
- `src/tools/subagent/send.rs`：新增 `mode`（enum queue|steer|stop_and_send，默认 queue；`validate_input` 与调用均拒绝未知值）。
  - queue → `enqueue_follow_up`：Queued → `status: queued`（position）；Idle → 本工具起新 turn → `status: started`。
  - steer → 子在跑：`enqueue_user_input` → `status: steered`（pending_id）；空闲 → started。
  - stop_and_send → `cancel_and_follow_up(.., StopOrigin::Parent)`：Queued → `status: stopped_and_queued`；Idle → started。不发 stopped 通知。
  - AgentAlreadyRunning 竞态按同一 mode 重新投递。metadata 带 `mode`、`status`。
- `stop.rs`：`cancel_turn_sync(child, StopOrigin::Parent)`；移除 hub 注入与 ChildStopped 写入（L5：父自停不通知自己）。`registry.rs` 注册回无 hub 版本。
- `delete.rs`：移除 ChildDeleted 写入（父自删不通知自己）。
- `wait.rs`：CompletionRef 填真值——事件路径取 `TurnFinished.reason/cancelled_by`；空闲/已结算路径取 `latest_turn_end_blocking`（仅当 turn_id 匹配）。
- `status.rs`：结果 reason=cancelled 时输出 `cancelled_by: user|parent|null`；新增 `format_steered`、`format_stopped_and_queued`；`format_queued` 改为 position。
- 描述：subagent_send/stop/delete/wait.md；KB：subagent-lifecycle、session、featured-tools、tool-authorization（只改文字，不改 status）。

## 测试
- 新增单测：send_mode_defaults_to_queue、send_queue_while_running_returns_queued、send_steer_injects_current_turn、send_stop_and_send_no_stopped_notice、send_mode_rejects_unknown、stop_by_parent_no_notice_to_self、delete_by_parent_no_notice_to_self。
- 新增集成：wait_returns_cancelled_when_user_stopped、wait_returns_latest_only；send_then_stop 增断言 `cancelled_by: parent`；busy/concurrent 用例改新文案（busy 断言 has_follow_ups，pending 为空）。
- 结果：tools::subagent 38/38；tool::registry 7/8（ISSUE-055 已知）；subagent_send 10/10、child_turn_contract 10/10、session_isolation 15/15；`cargo test --no-run` ok；rustfmt --check ok；kb_check errors 0。
