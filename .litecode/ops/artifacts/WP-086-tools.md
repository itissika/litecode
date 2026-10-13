# WP-092 产物（Tools）· 交付
接口：.tmp/wp090_interface.md FROZEN v1（WP-090 已交付）、.tmp/wp091_delete_api.md。未 commit。

## 落地
- T1 send.rs：子在跑 → enqueue_user_input，返回 `queued: true` + pending_id；reserve 竞态 AgentAlreadyRunning → 降级入队；空闲起 turn 返回 `queued: false`；无 mode 参数。
- T2 stop.rs：单次 cancel_turn_sync → CancelOutcome；Running|Starting → `status: stopped`（不带 turn_id）+ notify_parent_no_wake(ChildStopped)；Idle → `status: nothing running`；不碰 pending。registry 给 stop 注入 runtime.subagent_hub。
- T3 delete.rs（新）：subagent_delete{session_id}；只删直接子（meta.parent_session_id==调用方）；调 session::delete::delete_session_tree；AlreadyRunning/DescendantRunning 拒绝；成功只对直接子写一条 ChildDeleted（不唤醒）。schema.rs SUBAGENT_SERIES_TOOL_IDS 加 "subagent_delete"（授权的唯一一行）。
- list.rs：每个子输出 transcript（virtual_path）与 transcript_lines（reader().transcript_file_blocking 实时读取，与 turn_result/status 同源，非索引）。
- 描述：launch（权限创建时固定、运行中需审批一律拒绝）、send、stop、list、delete（新）。
- ISSUE-053：三份集成测试旧文案 → 已改为 queued/stopped/nothing running。

- 授权改动：src/config/schema.rs SUBAGENT_SERIES_TOOL_IDS +"subagent_delete"；src/config/global_db/tools.rs core_none_tools() +"subagent_delete"（各一行，其余未动）。

## 测试
- cargo test --lib tools::subagent：26/26
- cargo test --lib tool::registry：7/8；唯一失败 core_bindings_produce_full_builtin_list 为预存（ISSUE-055，见下）
- cargo test --lib runtime::pending_auto_turn：10/10
- 集成：subagent_child_turn_contract 10/10、subagent_send 8/8、subagent_tool_pipeline 9/9
- cargo test --lib --no-run：Finished；rustfmt --check（改动文件）干净

## done_when 映射
- send::starts_turn_when_idle → tests/subagent_send::send_to_idle_child_runs_another_turn
- send::queued_message_delivered_next_opportunity → runtime::pending_auto_turn::{idle_flush_reserves_claims_and_persists_one_user_row, busy_session_leaves_the_queue_alone}

## ISSUE
- ISSUE-053：三份集成测试旧文案，已在本包更新。
- ISSUE-055（grade 4，tools-permission）：core_bindings_produce_full_builtin_list 在 HEAD 即失败——HEAD 的 core_configurable_tools 与测试均含 session_search，但 a72ad6a（WP-057/061 检索面门控 retrieval_tools_visible）在非 Warm/desired=false 时把 session_search 移出 LLM 列表，测试断言（2e75ac2 起）未随之更新。非 WP-092 引入，本包不修。
