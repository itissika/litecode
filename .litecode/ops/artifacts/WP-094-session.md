# WP-094-session · WP-099 交付（Session）

- 角色：Session；包 WP-099（父 WP-095，approver=backend_mid）。时间 2026-10-13 HKT。未 commit。
- 依据：WP-094-backend.md §2 L5、§5 WP-099、§6 P1–P3；`.tmp/wp098_interface.md` FROZEN v1。
- 中层裁定：
  - Q1：cancelled_by 存在 turn/end 事件 data 里，不加列；
  - Q2：wire 上显式输出 null；
  - Q3：连接级 cancel 记作 user；
  - Q4：snapshot 不动；
  - Q5：turn_finished 保留 `reason`，旁边加 `cancelled_by`；SessionInfo 用 `last_turn_cancelled_by`；
  - Q6、Q7：可以。
- GM 裁定：stop_and_send 内部的停记作 Parent。

## 1. 改动（file:line）
### 用户停/删写 by=user，不唤醒父
- `src/client_protocol/connection.rs:449` agent/cancel 调 `cancel_turn(&sid, StopOrigin::User)`；真的停掉一个子会话时，在 :459 写 `ParentNotice::ChildStopped{child_session_id, by: User}`（`notify_parent_no_wake`）。没有在跑的 turn 时不写。
- `src/client_protocol/controller/session_lifecycle.rs:372` 用户删子会话写 `ChildDeleted{.., by: User}`。
- 不唤醒父由 WP-098 R-W 保证（Router 对 cancelled 只 push）。本包测试断言父的 `has_pending` 为 false。
### SessionInfo.last_turn_cancelled_by（P1）
- `src/client_protocol/protocol.rs:222` 新增 `last_turn_cancelled_by: Option<String>`，紧挨着 `last_turn_reason`，不加 skip，null 会显式输出。
- `session_lifecycle.rs:442`：和 `last_turn_reason` 取自同一条 turn/end 记录，用同一规则：运行中或主会话为 null；reason 不是 cancelled 时为 null。
- `src/client_protocol/status_track.rs:79` `latest_turn_end` 封装 WP-098 的 `SessionData::latest_turn_end_blocking`（读的是持久化数据，重连不丢）。
### turn_finished（P2 / Q5）
- `status_track.rs:38`：`session/lifecycle turn_finished` 保留 `reason`，旁边加 `cancelled_by`（"user" / "parent" / null，显式输出），值取自 WP-098 的 `LifecycleEvent::TurnFinished.cancelled_by`。每个位置只用一个名字，不重复出键。
- Runtime 已经把 cancelled_by 放到 TurnFinished 上，所以没有用"从事件数据回读"的兜底方案。
### 子会话只读
- WP-091 的 `reject_child_turn_start` 没动，另加了回归测试。
### snapshot
- 不动（Q4）。

## 2. by 取值逐处核对
| 调用点 | by | 结论 |
|---|---|---|
| connection.rs:449 agent/cancel | User | 对 |
| connection.rs:~1748 SessionRequest::Quit（连接退出） | User | 对（Q3） |
| connection.rs:~1761 SessionRequest::Cancel（连接级取消） | User | 对（Q3） |
| connection.rs:~1894 / ~1911（WS 断开路径） | User | 对（Q3） |
| connection.rs:459、session_lifecycle.rs:372 通知 | User | 对 |
| tools/subagent/stop.rs:51 | Parent | 对（Tools） |
| tools/subagent/send.rs:185 stop_and_send | Parent | 对（GM 裁定） |
| runtime/pending_auto_turn.rs:827/844 | Parent | Runtime 域，看上去是 follow-up 和 stop_and_send 路径，对 |
| **session/manager.rs:2007 `shutdown_cleanup`** | User | **待裁**：这是进程关闭时取消全部 turn，既不是用户点停止也不是父。这样会让子会话的 turn/end 记成 cancelled_by=user，前端会显示"被用户停止"。可以接受（关服务也算用户操作），或者改为不记来源。暂不改，请中层裁。 |

另外：:1748 和 :1761 两处在 `tokio::select!` 宏里，rustfmt 不处理，所以保持单行；我原本加的"Q3"注释被 Runtime 的机械替换覆盖了，语义不变。

## 3. 测试（新增 7 个，全部通过）
- `client_protocol::connection::ask_loop_tests::`：
  - `wp099_session_info_last_turn_cancelled_by_user_parent_null`（user / parent / 非 cancelled → null / 主会话恒 null / 显式 null 序列化）
  - `wp099_user_cancel_child_notice_by_user_and_no_wake`
  - `wp099_child_still_readonly_for_user`（run、plan_execution、retry、enqueue）
- `client_protocol::status_track::tests::`：
  - `turn_end_cancelled_by_user_parent_null`
  - `last_turn_cancelled_by_survives_reload`
  - `turn_finished_event_carries_cancelled_by`（同时验证只有一个键名、显式 null）
- WP-091 的 `wp091_agent_cancel_child_writes_parent_notice_no_wake`、`wp091_delete_child_tombstone_and_notice` 期望值已加上 by=User，并且覆盖了"用户删子 by=user"。
- 复跑：`cargo test --lib wp09` 14/14，`status_track` 5/5，`session::delete` 4/4。
- `cargo test --no-fail-fast`：lib 1857 通过、7 个失败，都不在本域：
  - ISSUE-046 Windows 路径 ×5；
  - ISSUE-055 registry ×1；
  - workspace::git ×1 偶发失败。
  - 集成测试全部通过。
- 本域文件 rustfmt `--check` 干净。

## 4. 偏差 / 备注
- 和 WP-094-backend §6 P2 原文的差异：原文写 turn_finished 带 `last_turn_reason`、`last_turn_cancelled_by`；按中层 Q5 裁定实际是保留 `reason`、新增 `cancelled_by`。前端需要按这个形状对齐。
- 构建期间，Runtime 的机械修改把我测试里的 `TurnFinished{..}` 构造填成了 `cancelled_by: None`，导致断言失败；已恢复为 `Some(Parent)`。
- 用户的 cancel 落在 Starting 窗口、turn 被放弃时（Q7），不会有 turn/end 记录，所以 `cancelled_by` 为 null，但仍会写 `ChildStopped{by: User}`。

## 5. 中层裁定（交付前补记）
- `shutdown_cleanup`（session/manager.rs:2007）传 `StopOrigin::User`：**接受**。关闭 litecode 属于用户操作；`StopOrigin` 只有 User/Parent 两个值，Runtime 接口保持不变。§2 表中该行的"待裁"就此关闭。
- P2 已修正为 turn_finished 带 `reason` 和 `cancelled_by`（不再写 `last_turn_reason` / `last_turn_cancelled_by`）；snapshot 项已移除（Q4），本包不改 snapshot。
