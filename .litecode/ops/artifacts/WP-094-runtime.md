# WP-094-runtime · WP-098 交付（Runtime）

- owner=runtime · 父 WP-095 · approver=backend_mid · 2026-10-13 HKT · 分支 ops/team-framework · **未 commit / 未 push**
- 接口：`.tmp/wp098_interface.md` **v1.1**（v1 中层已批；v1.1 = TurnFinished.cancelled_by 同值 + CompletionRef.cancelled_by + FollowUpOutcome::Started→Idle）。计划：`.tmp/wp098_plan.md`。Q1–Q9 均按默认（Q1–Q4 GM 裁定同默认）。

## 1. 落地（Runtime 域）
| 项 | 位置 |
|---|---|
| `enum StopOrigin { User, Parent }`（serde snake_case，`as_str`/`parse`） | `src/types/mod.rs:24`（`pub use` 于 `crate::session`） |
| `cancel_turn_sync(sid, by)` / `cancel_turn(sid, by)`；单锁体 `cancel_locked`，首写者胜 | `src/session/manager.rs:110`、`~1236-1262` |
| `LiveTurnState.cancelled_by` / `StartingTurn.cancelled_by` | `src/session/manager.rs:62`、`:79` |
| `turn_cancelled_by(sid, turn_id)`（runtime 写 turn/end 前读） | `src/session/manager.rs:1265` |
| follow-up 队列 `SessionRecord.follow_ups`（内存，与 pending 分开） | `src/session/manager.rs:271` |
| `FollowUpOutcome { Idle, Queued{position} }` | `src/session/manager.rs:101` |
| `enqueue_follow_up` / `cancel_and_follow_up` / `has_follow_ups` / `follow_ups_len` / `reserve_turn_and_pop_follow_up` / `restore_follow_up_front` | `src/session/manager.rs:1282 / 1311 / 1338 / 1346 / 1356 / 1399` |
| turn/end data 写 `"cancelled_by"`（仅 reason=cancelled 且有 by） | `src/runtime/mod.rs:817-826` |
| `InternalEvent::TurnCompleted.cancelled_by` | `src/runtime/observer.rs:150`（填充 `src/runtime/mod.rs:850`） |
| `SessionData::latest_turn_end_blocking -> (turn, reason, Option<StopOrigin>)`（旧函数包装保留） | `src/session/data/mod.rs:327` |
| **`LifecycleEvent::TurnFinished.cancelled_by: Option<StopOrigin>`** | 定义 `src/session/live.rs:67`；填充 `src/session/manager.rs:1104-1117`（finish_turn 从 durable turn/end 读，与落盘值同源同值） |
| **`CompletionRef.cancelled: bool` + `cancelled_by: Option<StopOrigin>`** | 定义 `src/tools/subagent/jobs.rs:15-17`；Router 填充 `src/tools/subagent/hub.rs:169-170`（TurnFinished）与 `:208-209`（Lagged rescan） |
| CompletionInbox 每子一条、新覆盖未读、restore 不盖更新 | `src/tools/subagent/jobs.rs` push / restore_front |
| `ParentNotice::{ChildStopped,ChildDeleted}{child_session_id, by}`；去重键 (kind, child)；`reason_text()` → "stopped by user" 等 | `src/tools/subagent/jobs.rs`；`src/runtime/exec.rs:1067` 改用 `reason_text()` |
| cancelled 结果只进邮箱、不调 exit_handler | `src/tools/subagent/hub.rs:135` deliver |
| follow-up 调度：pending 先 flush（Q7），空后逐条 FIFO 一 turn 一条 | `src/runtime/pending_auto_turn.rs` try_begin_pending_flush |
| 测试 builder `build_child_runtime_with_provider`（集成测用 open_child_session，src/runtime 只用 test_child_session） | `tests/common/runtime.rs` |

## 2. 机械补参清单（授权：只加 by / 新字段，编译用，零语义改动）
by 一律 `StopOrigin::User`：
- `src/client_protocol/connection.rs:449` agent/cancel → `cancel_turn(&sid, User)`；`:459` `ChildStopped{.., by: User}`
- `src/client_protocol/connection.rs:1748`、`:1761`、`:1894`、`:1911` 连接级 cancel → `cancel_turn(.., User)`
- `src/client_protocol/connection.rs:2676`、`:2750` 测试断言 `ParentNotice{.., by: User}`
- `src/client_protocol/controller/session_lifecycle.rs:372` 用户删 `ChildDeleted{.., by: User}`
- `src/tools/subagent/stop.rs:~50` `cancel_turn_sync(child, User)` —— **Tools 已在 WP-100 中改为 `Parent`**（当前树为 Parent）
- `src/session/manager.rs:2007` remove 路径 cancel → `User`；manager 单测内 cancel 调用同补
- `tests/subagent_session_isolation.rs:819` `cancel_turn_sync(&first, User)`
新增字段的模式/构造补齐（无行为变化）：
- `src/client_protocol/controller/mod.rs:355/363`（透传 cancelled_by）、`:882`（`cancelled_by: _`）、`:1857`（测试 `None`）
- `src/client_protocol/project.rs:489`、`:897` 模式加 `..`
- `src/client_protocol/status_track.rs:43` 模式加 `..`（之后 Session 已改为读取 cancelled_by）、测试构造 `cancelled_by: None`
- `src/tools/subagent/wait.rs` 三处 `CompletionRef` 先补 `cancelled:false, cancelled_by:None` —— **Tools 已在 WP-100 中替换为真实值**
- `src/runtime/subagent_auto_turn.rs:250`、`src/runtime/observer.rs:400` 测试构造
- `tests/client_protocol_project.rs:390/564/590/613`、`tests/wire_seq_identity.rs:447` `cancelled_by: None`
注：Session（WP-099）、Tools（WP-100）在本包实施期间并行修改同一工作树（status_track.rs、stop.rs、wait.rs、send.rs、connection.rs:2886 等），这些非本包改动。

## 3. 新 ISSUE
- **ISSUE-056**（grade=bug/product，Andy 五档 2 产品缺陷，status=open，裁决 undecided/pending）：重启后已回 `queued:true` 的 follow-up 静默丢失；建议 (a) durable follow_up 事件重建 /(b) deferred item/user /(c) 至少发 FollowUpLost 通知。本包不修。
- ISSUE-054：per-child 覆盖使「未读时重启补投」不再重复入邮箱，但已读后仍会补投并唤醒 → 部分缓解，不关闭（`hub_lagged_rescan_dedups_per_child` 记录现状）。

## 4. 测试证据（plain `cargo test`，scoped，2026-10-13 HKT）
- `cargo test --lib -- pending_auto_turn tools::subagent session::manager`：93 passed
- `cargo test --lib -- runtime:: client_protocol:: session:: tools::subagent`：362 passed, 0 failed
- `--test wp098_follow_up_queue`：6 passed（4 本包 + 2 common 自带）
- `--test architecture_layers` 5、`pending_messages` 5、`subagent_child_turn_contract` 10、`subagent_send` 10、`subagent_session_isolation` 15、`subagent_tool_pipeline` 9、`client_protocol_project` 26、`wire_seq_identity` 11、`issue_032_changed_files_restore` 6 —— 全绿
- 既有失败：本次所跑范围内无。`cargo fmt --check` 干净。clippy 未跑。
§12 测试名对照：
- 红线 `runtime_main_session_pending_still_injects_at_seam`（tests/wp098_follow_up_queue.rs）
- `runtime_steer_still_injects_at_step_seam`、`runtime_queue_not_drained_at_step_seam`、`runtime_stop_and_send_cancels_then_runs_new_turn_no_notice`（同文件；后者同时覆盖 `runtime_cancelled_by_persisted_in_turn_end` 与 TurnFinished 同值断言）
- `runtime_queue_while_running_runs_new_turn_after_end`、`runtime_queue_multiple_runs_one_turn_each_fifo`、`pending_flushes_before_follow_ups`、`runtime_stop_and_send_starting_window`（src/runtime/pending_auto_turn.rs）
- `hub_latest_only_one_result_per_child`、`hub_newer_result_overwrites_unread`、`hub_restore_front_does_not_clobber_newer`、`notice_dedup_is_per_kind_and_child_latest_by_wins`、`settled_reminder_text_stopped_by_user`（jobs.rs）
- `hub_stopped_turn_result_mailbox_only_no_wake`、`hub_notice_carries_by_user_parent`、`hub_lagged_rescan_dedups_per_child`（hub.rs）

## 5. 偏差 / 风险
- v1.1：`FollowUpOutcome::Started` → `Idle`（manager 无 runtime 句柄；子空闲时调用方照旧直接起 turn）。
- 「重载后读回」未单独起新进程测；cancelled_by 读取走 durable 事件日志（`latest_turn_end_blocking`），非内存。
- Exclusive（manual compact/revert）期间入队的 follow-up 要等下一次 TurnFinished 才调度（与 pending 现状一致）。
- 端到端「被停 turn 后自动起 follow-up 真跑一轮」由 pending_auto_turn 预约层 + 集成 stop_and_send 测分段覆盖，未做整链 spawn 测。
