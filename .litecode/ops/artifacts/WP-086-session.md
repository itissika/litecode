# WP-086-session · WP-091 交付（Session）

- 角色：Session；包 WP-091（父 WP-087，approver=backend_mid）。时间 2026-10-13 HKT。未 commit。
- 依据：WP-086-backend.md §5/§6（字段名、错误码未改）、`.tmp/wp090_interface.md` FROZEN v1、WP-086-runtime.md §2（R8 hook）。
- 中层裁定已执行：Q1 turn_started/turn_finished 照推并带 status，另加 status_changed；Q2 turn_updated/turn_step 不带 status；Q3 只通知被请求删除那一层的父；Q4 所有客户端起 turn 入口都拒；Q5 无事可停不写 ChildStopped；Q6 笔误按 090/091/092；Q7 墓碑存在父会话 meta 字段（加列），不新建表。

## 1. 改动（file:line）
### K1 子会话只读
- `src/client_protocol/protocol.rs:77` 新增 `ErrorCode::SubagentSessionReadonly`、`DescendantRunning`（wire 为 snake_case）。
- `src/client_protocol/connection.rs:62` `structured_error_response`：返回 -32000，`data` 为 StructuredError，可附带额外字段。
- `connection.rs:87` `reject_child_turn_start`，message 用常量 `SUBAGENT_READONLY_MESSAGE`，文字照抄 §5。
- **拦截的入口清单（所有客户端起 turn 的路径）：**
  1. `agent/run`（`connection.rs:300`），包括 `plan_execution: true`
  2. `agent/retry`（`connection.rs:394`；原来是字符串 "cannot retry a subagent session"，现在改用同一个错误码）
  3. `session/pending-enqueue`（`connection.rs:561`）：在入队之前就拒，空闲或在跑都一样，覆盖了原来的空闲竞态起 turn 分支
  - 已核对：`connection.rs` 里只有 run / enqueue 两处调用 `SessionController::start_turn`，retry 走 `retry_failed_turn`；其它 RPC 不会起 turn。
### R1 cancel 如实回报
- `connection.rs:446` 起：`let outcome = cancel_turn(&sid).await`，回包为 `{"cancelled": outcome.cancelled()}`，对子会话放行。只有 `cancelled()` 为真且会话有父时，才调用 `notify_parent_no_wake(parent, ChildStopped)`。
- 其它 4 处 `cancel_turn` 调用按接口 §6 改为 `let _ =`。
### K4 / R6 删除
- 新文件 `src/session/delete.rs`，提供共用删除入口 `delete_session_tree(sessions, cleanup, id)`：先预检后代是否在跑（`descendant_running`），再取得 lease（lease 失败时再判断一次后代），然后 `remove_session`，最后对每个被删 id 调用 `cleanup.on_session_removed`。
  - 用 trait `RemovedSessionCleanup` 而不是直接引用 hub，原因是 `architecture_layers` 测试禁止 `src/session` 出现 `tools::subagent`。SubagentHub 的实现（purge_parent + forget_child）放在 `src/client_protocol/controller/session_lifecycle.rs` 文件末尾。
  - 接口说明给 Tools：`.tmp/wp091_delete_api.md`。
- `src/session/manager.rs:1983` `remove_session -> Result<Vec<String>>`：后代 sorted 在前，root 最后；每条 SessionRemoved 都带父 id（父 id 在删除前读出）。新增 `:2048 parent_session_id_of`、`:2059 running_descendant_ids`。
- `controller/session_lifecycle.rs:360` delete_session 改走共用入口；删的是子会话时写 `ChildDeleted`，不唤醒父；同时移除全部被删 id 的 projection。
- `connection.rs` session/delete 的错误映射：`agent_already_running`（retryable）和 `descendant_running`（`data.running_session_ids`）返回结构化错误；修掉了原来用 `msg.contains("session is running")` 判断、永远匹配不上的问题。
- 不再吞磁盘错误：`src/session/data/sqlite/session.rs`（cleanup_deleted_files 的两处 `let _`）改为 `warn!`；`src/session/data/writer.rs:506` orphan blob 删除失败也 warn（NotFound 除外）。
### R7 deleted 事件与墓碑
- `src/session/live.rs` 的 `SessionRemoved` 增加 `parent_session_id: Option<String>`；`src/client_protocol/project.rs:737` 的 deleted 事件总是输出 `parent_session_id`（顶层会话为 null）。
- 墓碑存储：`schema.rs:171/:344` 新增列 `deleted_children_json TEXT DEFAULT '{}'`（建表时和迁移时都加，属于加法）。`writer.rs:953` 在同一个 Delete 事务里调 `parent_link_on`（`session.rs:1524`）和 `record_deleted_child_on`（`:1545`），写入 `{call_id: child_id}`。只有父还存在时才写；父不 bump revision。
- 读取：`command.rs:389` `SessionRead::DeletedChildBindings`、`read.rs:362`、`manager.rs:2097`；`controller/mod.rs:1225` `subagent_bindings_and_states` 把在用的绑定和墓碑合并。
- wire：`protocol.rs:346` `SubagentBindingState{child_session_id, deleted}`。`BufferLoadResult.subagent_binding_states` 和 `session/snapshot` 回包都新增 `subagent_binding_states`；`subagent_bindings` 保留已删的条目，兼容旧前端。
### R8 原始状态
- 新文件 `src/client_protocol/status_track.rs`：
  - `project_lifecycle` 给 turn_started / turn_finished 附上 `status`（`session_status` 的原始值）。
  - `ParentStatusTracker` 重新计算各级祖先的聚合状态，只在真正变化时调用 `manager.emit_status_changed`（`manager.rs:2088`），对应新的 `LifecycleEvent::SessionStatusChanged`（`live.rs:55`），wire 为 `event:"status_changed", status`。
  - `install_parent_status_hook` 通过 Runtime 的 `set_turn_status_hook` 注册，用 Weak 引用避免循环；在 `src/serve/state.rs:135` 构造 SessionManager 之后调用。
- `ws.rs:319` 的 lifecycle 转发改用 `project_lifecycle`。
- `controller/session_lifecycle.rs:414` list_sessions 不再跳过有父的空会话。
- 已接入 Runtime 的 R8 hook（WP-086-runtime.md §2），代码里没有留 TODO。Runtime 的 StartingTurn 段我没有改。
### 其它
- `manager.rs:2078` 新增 `#[cfg(test)] test_child_session`。原因：`architecture_layers` 禁止 client_protocol 源码里出现 `open_child_session` 这个字样，协议层测试只能借这个函数建子会话。这是绕开文本检查，请中层确认可以接受。
- 新增 LifecycleEvent 字段和变体以后，有三处别人的代码要跟着做机械适配，都只是补 `..` 或补一个 match 分支：
  - `src/tools/subagent/wait.rs`（Tools）的模式补 `..`；
  - `src/tools/subagent/hub.rs`（Runtime）的模式补 `..`；
  - `tests/subagent_session_isolation.rs` 的 match 补 `SessionStatusChanged`。
- 只对本域文件跑了 rustfmt，`--check` 已干净。

## 2. 测试（新增 17 个，全部通过）
- `client_protocol::connection::ask_loop_tests::`，共 11 个：
  - `wp091_agent_run_on_child_rejected_readonly`（含 plan_execution）
  - `wp091_retry_on_child_rejected_readonly`
  - `wp091_pending_enqueue_on_child_rejected_readonly`
  - `wp091_enqueue_on_running_child_rejected_readonly`
  - `wp091_agent_cancel_reports_false_when_idle`（同时验证 Q5）
  - `wp091_agent_cancel_child_writes_parent_notice_no_wake`
  - `wp091_delete_parent_with_running_child_returns_descendant_running`
  - `wp091_delete_running_child_rejected`
  - `wp091_delete_child_tombstone_and_notice`
  - `wp091_list_sessions_includes_empty_child`
  - `wp091_buffer_load_returns_subagent_binding_states`（补充：subscribe 后 buffer/load 返回 `subagent_binding_states.call_wp091 = {child_session_id, deleted:true}`，`subagent_bindings` 保留该条）
- `session::delete::tests::`，共 4 个：
  - `remove_session_returns_descendants_sorted_root_last`（同时验证 deleted 事件带 parent）
  - `delete_calls_cleanup_for_every_removed_id`
  - `tombstone_survives_reload`
  - `cleanup_missing_paths_do_not_panic`（注意：没有断言 warn 日志本身，因为未接 tracing 捕获）
- `client_protocol::status_track::tests::`，共 2 个：
  - `turn_events_carry_raw_status_and_parent_status_changed`
  - `installed_hook_is_registered`（只验证聚合状态没变时不推）
- 和 WP-086 §5 测试名的差异：名字统一用了 `wp091_` 前缀并放在已有的测试模块里；`turn_events_carry_raw_status` 和 `parent_status_changed_on_child_turn` 合并成了一个测试。

## 3. `cargo test`（直接跑，用共享 target 目录）
- lib 1833 通过，8 个失败，都不在 Session 域：
  - config::path、lsp ×3、terminal::shell：Windows 路径相关测试在 Linux 上失败（既有问题）；
  - `tool::registry::core_bindings_produce_full_builtin_list`：Tools 的 WP-092 正在开发；
  - workspace::git ×2：偶发失败，单独重跑 12/12 通过。
- 集成测试：只有 `architecture_layers::runtime_and_protocol_do_not_open_child_sessions` 失败，原因在 `src/runtime/{subagent_auto_turn,pending_auto_turn}.rs` 的测试代码里出现了 open_child_session（Runtime 的改动），不是本包造成的。其余全部通过。

## 4. 偏差 / 需中层知悉
1. `descendant_running` 的 `data` 是 StructuredError 三个字段（code、message、retryable）再加 `running_session_ids`。§6 只写了 `data.running_session_ids`，这里没有改任何名字。
2. `session/snapshot` 的 `subagent_binding_states` 是在 JSON 回包里注入的，没有改 snapshot 结构体，因为它的构造点太多。
3. 删除入口的签名比 wp090 接口多了一个 trait 参数，但调用写法 `delete_session_tree(&sessions, &hub, &id)` 和 Tools 现有代码一致，Tools 那边编译通过。
4. ChildStopped 的投递去重依赖 hub 的 dedup；子会话被重复取消时只会留一条。

## 5. 中层裁定（backend_mid，对 §4 与 §1"其它"共 6 项）
1. descendant_running 的 data 形状（StructuredError + running_session_ids）：**同意**。
2. `#[cfg(test)] test_child_session`：**有条件同意**，条件是只在测试编译。已核实：`manager.rs:2077` 带 `#[cfg(test)]`，注释写明"仅供单测创建子会话，生产代码不得调用"；全部调用点都在测试模块里：`session/delete.rs` tests、`client_protocol/status_track.rs` tests、`client_protocol/connection.rs` ask_loop_tests，以及 Runtime 的 `runtime/subagent_auto_turn.rs:369`、`runtime/pending_auto_turn.rs:637` 测试。
3. 删除入口的 trait 参数（RemovedSessionCleanup）：**同意**。
4. snapshot 里注入 `subagent_binding_states`：**同意**，属于加法，与 P6 一致。buffer/load 也返回该字段，已补测试。
5. 跨域机械适配：**同意**。
6. cleanup 测试没有断言 warn 日志：**可以**。

### 跨域改动文件
| 文件 | 归属 | 改动 |
|---|---|---|
| `src/tools/subagent/wait.rs` | Tools | 匹配 `LifecycleEvent::SessionRemoved { session_id }` 的模式补 `..`，以适配新增的 `parent_session_id` 字段 |
| `src/tools/subagent/hub.rs` | Runtime | 同上，router 里的 `SessionRemoved` 模式补 `..` |
| `tests/subagent_session_isolation.rs` | 测试 | match 里 `SessionRemoved { .. } => continue` 一臂并入 `SessionStatusChanged { .. }` |

### 已知失败（非本包）
- ISSUE-046：Windows 路径测试（config::path、lsp ×3、terminal::shell）在 Linux 上失败。
- ISSUE-055：`tool::registry::core_bindings_produce_full_builtin_list`（Tools）。
- `architecture_layers` 已由 Runtime 修复，复跑 5/5 通过。

### 交付前复跑（2026-10-13 HKT）
- `cargo test --lib wp091` 11/11；`session::delete` 4/4；`status_track` 2/2；`pending_auto_turn` 10/10。
- `--test architecture_layers` 5/5；`subagent_session_isolation` 4/4；`session_delete` 15/15。
- 对本域文件跑 rustfmt `--check`：干净。
