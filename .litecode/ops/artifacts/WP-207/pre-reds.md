# WP-207 删前基线红（cargo test -j 2 --no-fail-fast，HEAD a8612cf + deadline 补丁）
总计：2699 passed / 36 failed / 4 ignored；耗时 1m43s（增量编译）。日志 cargo-full-pre.txt。
- lib 9：即已知 9 条基线（ptc stack 2 归 Runtime）。
- tests/ 27（初判，均待逐条确认）：
  - 旧工具名 bash/run_command（过时→改用 run）：permission_pipeline ×2（floor_blocks_sensitive_commands…、safe_bash_matrix…）、settings_api ×3（apply_preset…、mcp_server_rejects_builtin_id、put_agent_expands…）、global_db_seed ×1。permission floor 两条是安全守卫，应**改**不删。
  - 旧 run_code 契约（ledger/输出文本/data 形状/cursor 方言 include_ignored/read 入参）：code_surface_phase2b ×5、phase3 ×6、phase5 ×2、code_tool_runtime ×4、ptc_contract_harness ×3。
  - WP-206 C 行为变化：optional_lsp::idle_reclaim_during_diagnostics_does_not_panic（diagnostics 冷启动现 reject index-not-ready，测试仍期望成功）。
- 待核（可能掩盖产品缺陷）：phase2b cancellation_seals_in_flight_nested_calls / outer_timeout_does_not_overwrite_a_completed_nested_result / a_call_that_outlasts_the_grace_period_is_unsettled / approval_wait_does_not_consume_the_program_budget（取消/超时/审批预算语义）；phase3 conflicting_spellings…（read 未等 write：file not found）、two_nested_writes…。
