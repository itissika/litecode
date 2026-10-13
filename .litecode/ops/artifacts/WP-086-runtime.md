# WP-086-runtime · WP-090 交付（Runtime：R1 / K5 / R11 / K6 + R8 hook）

- owner=runtime · approver=backend_mid · 2026-10-13 HKT · 分支 ops/team-framework · **未 commit**
- 接口：`.tmp/wp090_interface.md` FROZEN v1 —— **无签名偏离**；新增一项（中层追加派单）：§5 turn status hook。

## 1. 改动
| 项 | 位置 | 说明 |
|---|---|---|
| R1 CancelOutcome | `src/session/manager.rs:86`（enum+`cancelled()`），`pub use` 于 `crate::session` | Running / Starting / Idle |
| R1 StartingTurn.cancel_requested | `manager.rs:75` | reserve_turn / reserve_turn_and_claim_pending 置 false |
| R1 cancel_turn_sync / cancel_turn | `manager.rs:1198` / `:1191` | 单次 records 锁内判定+动作；Exclusive→Idle；不碰 pending/mailbox/inbox；不发通知；Starting 重复取消仍回 Starting（幂等） |
| R1 start_turn | `manager.rs` start_turn | 若 cancel_requested：装 RunningTurn(stopping=true) 并立即 `cancel.cancel()`，走正常取消收尾（TurnFinished→父照常唤醒） |
| Q4 helper | `manager.rs:1221` `pub(crate) fn idle_child_session_ids() -> Vec<(child, parent)>` | 只读 |
| K5 ParentNotice/NoticeInbox | `src/tools/subagent/jobs.rs`；`mod.rs` 导出 `ParentNotice, NoticeInbox` | 独立于 CompletionInbox，去重 |
| K5 hub API | `src/tools/subagent/hub.rs:85-104` `notify_parent_no_wake / take_notices / restore_notices / has_notices` | 不调 exit_handler、不影响 has_pending；`purge_parent` 同清 notices；`forget_child` 不删以该 child 为对象的通知；router 在 SessionRemoved(该会话为父) 时清其 notices |
| K5 step seam | `src/runtime/exec.rs:293` 取通知；以 `SettledChild{turn_id:"", reason:"stopped"/"deleted"}` 并入同一 `reminder/subagent_settled`（有 completions 时追加到 detail）；写失败/未写按 completions 同口径 `restore_parent_notices`（:416/:501/:522） | 未改 reminder 模块 |
| R11 | `hub.rs` `Router::handle`/`rescan`（:183） | 记 delivered(child→turn)；Lagged→重扫 idle 子最新 `turn/end`，不同即补投+exit_handler；重复 lag 不重投 |
| K6 | 新 `src/runtime/auto_turn_sink.rs` `auto_turn_permission_sink(sessions, sid)` | 子→deny；主→last sink（Q1）。subagent/pending/custom/bash auto-turn 四处改用 |
| remove_session 签名 | `manager.rs` | `Result<Vec<String>>`（后代 sorted，root 最后）。实现已由 Session WP-091 在树内接管 |

## 2. R8 turn status hook（给 Session）
```rust
// src/session/manager.rs:108
pub type TurnStatusHook = Arc<dyn Fn(&str, SessionStatus) + Send + Sync>;
// manager.rs:951
pub fn set_turn_status_hook(&self, hook: TurnStatusHook); // 替换旧 hook；默认 None=no-op
```
调用点（`fire_turn_status`，manager.rs:956）：
- `release_turn_reservation` 成功释放 → `Idle`（manager.rs:945）
- `start_turn` 装好 RunningTurn 后 → `Running`；若 Starting 窗口已请求取消 → `Stopping`（:1136，TurnStarted 之后）
- `finish_turn` → 结束后的状态：通常 `Idle`；有排队 revert 拿到独占时为 `Running`（:1074，TurnFinished 之后）

保证：总在 **records 锁外**、对应 lifecycle 事件 emit 之后、在发生迁移的线程上同步调用；hook 内可安全回调 SessionManager（不会死锁）。子会话同样触发（父聚合 `RunningWithSubagent` 由 Session 自行重算）。
**不触发**的迁移：reserve_turn（Idle→StartingTurn）、begin_turn（直起）、cancel_turn（→Stopping）、try_begin_revert、remove_session —— 若 Session 需要，请报中层追加。
测试：`manager::child_session_tests::turn_status_hook_fires_on_start_and_release`（release→Idle、start→Running、finish→Idle）。

## 3. 测试
- done_when 9 条 + hook 1 条：**10 passed / 0 failed**（`cargo test --lib -- <names>`）。
- `cargo test --lib -- runtime:: tools::subagent:: manager::`：126 passed / 1 failed —— 失败为 Tools 新测 `tools::subagent::stop::tests::stop_writes_child_stopped_notice_without_wake`（见 §5）。
- 集成：agent_loop 13/13、compact_failure_calibers 7/7、issue_032_changed_files_restore 6/6、manual_compact_e2e 4/4、subagent_session_isolation 15/15 —— 全绿。
- subagent_child_turn_contract 8/10、subagent_send 6/8、subagent_tool_pipeline 8/9：失败均为旧文案断言（`stop_requested`/`already ended`/`already running`），由 Tools WP-092 的 stop/send 文案与 K2 入队改动引起，非 Runtime。
- fmt：Runtime 触及文件干净（`cargo fmt --check` 仅 connection.rs 有 Session 侧差异）。clippy：本机工具链未装 clippy 组件，未跑（未擅自安装）。

## 4. 非本域最小编译补丁
- `src/session/mod.rs:36`：`pub use` 增加 `CancelOutcome`（接口要求导出）。
- `tests/subagent_session_isolation.rs:815`：`assert!(sessions.cancel_turn_sync(&first))` → `.cancelled()`。
- `src/tools/subagent/stop.rs`：曾加 `== CancelOutcome::Running` 机械补丁，现已被 Tools WP-092 正式改写覆盖，**Runtime 不再有改动**。
- `src/client_protocol/connection.rs`：**无改动**（`cancel_turn(..).await;` 返回值非 must_use，原样编译）。
- `src/client_protocol/controller/session_lifecycle.rs`：**无改动**（`?;` 丢弃 Vec 即可）。
- `manager.rs` shutdown_cleanup：`let _ = self.cancel_turn(id).await;`（行为不变）。

## 5. 留给 Session / Tools
- Tools：`stop_writes_child_stopped_notice_without_wake` 末段假设"Starting 窗口第二次 stop 为 Idle"，与 v1 §1"Starting 窗口内重复 cancel 仍返回 Starting"冲突——需先 `release_turn_reservation`/start+finish 后再断言 Idle；另更新上述 3 个集成测试的旧文案。
- Session：按 §2 安装 hook；connection.rs 调用点语义（`.cancelled()` 回包、ChildStopped/ChildDeleted 通知）按 v1 §6 落地。
- R11 说明：rescan 对 delivered 无记录的 idle 子也会补投其最新 turn（按 v1 字面），进程重启后首次 lag 可能让父多收到一次旧结果（inbox 去重、只一次）。
