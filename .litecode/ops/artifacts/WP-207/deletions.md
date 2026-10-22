# WP-207 deletions（G3/G2 + permission_pipeline；G4/death_list 待续）

图例：过时=断言已被 WP-197/199/200/201/206 改掉的契约；冗余=有等价覆盖；过期 gate=迁移期守卫；文本重复=SDK fixture 快照（run_code_sdk.txt）已逐字覆盖。

## 整文件删除
| 文件 | 测 | 理由 | 替代覆盖 |
|---|---|---|---|
| src/tool/deletion_gate.rs | phase0_direct_legacy_deletion_gate | 过期 gate（自述 Phase 0 记录红；目标 edit/feedback.rs 已删） | 无需（迁移完成） |
| src/tool/contract_matrix.rs | phase5_contract_matrix_asserts_live_output | 过时（:421 断言 run_code 无 data，WP-201 已加 data）+冗余：tool×scenario 行与 tools::baseline fixtures（`src/tools/baseline/fixtures/*__*.json`，锁 data+error）重复 | tools::baseline::phase0_tool_fixtures_lock_data_and_error |
| src/tool/phase1_gate.rs | 6 条，见下 | | |
| src/tools/phase2_gate.rs | 17 条，见下 | | |
| src/tools/phase3_gate.rs | 9 条，见下 | | |

### phase1_gate（6）
| 测 | 处置 | 理由 / 替代 |
|---|---|---|
| edit_lsp_error_reaches_outcome_and_run_code_data | 删 | 过时（25a15e1 删 edit/feedback.rs，diagnostics 通路变更） |
| run_code_calls_match_the_text_ledger_and_logs_keep_level | 删 | 过时（WP-200/201 去 ledger、改 data） |
| uncaught_tool_error_keeps_the_outer_code | 删 | 过时+冗余：ptc::tests::uncaught_tool_error_keeps_code |
| read_image_is_unsupported_without_parts | 删 | 冗余：read.rs test_read_image_is_unsupported |
| write_outside_workspace_reports_path_outside_workspace | 并入 | src/tool/executor.rs `code_caller_contract_tests` |
| unknown_parameter_is_invalid_input | 并入 | 同上（code caller 下 invalid_input；executor 已有 direct 下 warn 的 unknown_parameters_warn_and_still_run） |

### phase2_gate（17）
| 测 | 处置 | 理由 / 替代 |
|---|---|---|
| phase2_read_limit_reports_more_in_message | 删 | 冗余：read.rs test_read_default_limit_pages / test_read_with_start_line_and_limit |
| phase2_grep_no_cursor_dialect | 删 | 冗余：grep.rs schema 测（`!props.contains_key("cursor")`，grep.rs:1031）+ SDK 快照 |
| phase2_grep_limit_bounds_hits | 删 | 冗余：grep.rs limit_bounds_hits_and_notes_truncation |
| phase2_grep_invalid_regex_details | 删 | 冗余：grep.rs assert_invalid_regex 系列（a_bare_brace_is_invalid_regex 等） |
| grep_message_no_match / corpus_empty / glob_empty / path_excluded / binary_file、grep_literal_searches_metacharacters（6） | 并入 | src/tools/grep.rs `message_contract_tests`（去 phase2_ 前缀） |
| read_page_size_ignores_token_budget、read_missing_file_is_not_found、glob_over_limit_notes_truncation_and_read_consumes_path、outside_workspace_paths_noted、chain_glob_read、chain_grep_read、chain_read_edit（7） | 并入 | src/tools/read.rs `chain_contract_tests` |

### phase3_gate（9）
| 测 | 处置 | 理由 / 替代 |
|---|---|---|
| phase3_apply_edits_partial_reaches_run_code_ledger | 删 | 过时（WP-200 去 ledger） |
| phase3_edit_rejects_expected_hash_knob | 删 | 冗余：edit/planner.rs parse_rejects_replace_all_and_expected_hash |
| phase3_apply_edits_serial_skip_on_failure | 删 | 冗余：apply_edits.rs serial_skip_continues_results_same_len |
| write_expected_hash_*（4） | 并入 | src/tools/write.rs `expected_hash_tests`（write.rs 原无 expected_hash 测） |
| edit_failure_resolves_with_message、edit_failure_messages_are_stable | 并入 | src/tools/edit/tests.rs `failure_message_tests` |

## 描述/PRELUDE 文本断言（文本重复 → SDK 快照为唯一守卫）
| 位置 | 处置 |
|---|---|
| src/tools/edit/tests.rs `description_has_no_old_contract_knobs`（整测） | 删；edit 描述在 run_code_sdk.txt |
| src/tools/apply_edits.rs `description_mentions_serial_skip`（整测） | 删；apply_edits 描述在快照 |
| src/tool/code_manifest.rs 8 条 PRELUDE 子串断言（do not copy type annotations / declare const libs / stats / csv / groupBy / declare class ToolError / `undefined` is tolerated / dropped paths） | 删断言，测试保留（其余断言验证 FakeTool 的 schema→TS 渲染，非快照可覆盖） |
| src/tools/code.rs 5 处 `text.contains("N succeeded…")` | **保留**：断言运行输出，不是描述文本 |
| registry.rs / subagent/launch.rs / knowledge.rs | 保留（Q3：不在 SDK 快照） |

## 改写（保留）
| 测 | 改动 | 理由 |
|---|---|---|
| tools::litecode_workspace::tests::refresh_reports_invalid_mcp_definitions_outside_the_accepted_diff | 冲突 id `bash` → `run` | 过时：Direct bash 已退役，`run` 为现役 core tool（config/global_db/tools.rs） |
| tests/permission_pipeline.rs floor_blocks_sensitive_commands_under_unrestricted_all_preset、safe_bash_matrix…（改名 safe_run_matrix_readonly_allow_others_deny_floor_hard_deny） | `run_command` → `run` | 过时工具名使 floor 规则未命中（Allow/Ask）；安全守卫必须保留 |

## 计数（本批）
- 删除测试：lib 13 条（deletion_gate 1、contract_matrix 1、phase1 4、phase2 4、phase3 3）+ 描述整测 2 = 15；删 PRELUDE 断言 8 处。
- 并入（迁移保留）：2 + 13 + 6 = 21 条。
- 改写：3 条。
- 待续（G4）：code_surface 合并、death_list 清理、settings_api/global_db_seed/code_tool_runtime/ptc_contract_harness/optional_lsp 过时项；Runtime 在判的 6 条不动。

## G4（集成测）
### code_surface_phase1..5 → tests/code_surface.rs（模块 projection / run_code_only / edit_write / approval_locks / approval_concurrency）
| 测 | 处置 | 理由 |
|---|---|---|
| phase1 run_code_description_is_a_pure_function_of_the_code_surface | 删 | 文本重复：SDK 快照 |
| phase3 partial_program_keeps_one_ledger_line_per_started_write | 删 | 过时：ledger 已去除 |
| phase5 manual_procedure_chains_glob_read_grep_edit_and_write | 删 | 过时：依赖已退役的 page.next / file.hash / edit expected_hash+edits[] 方言；链路由 tools::read chain_contract_tests 覆盖 |
| phase5 cancel_before_approval_does_not_write_later | 删 | 冗余：approval_locks::cancel_before_approval_does_not_write_later（phase4 同名） |
| 2b cancellation_seals_in_flight_nested_calls | 改 | Runtime 判过时：去 ledger 文本，断言 sealed + error kind=Cancelled |
| 2b outer_timeout_does_not_overwrite_a_completed_nested_result | 改 | Runtime 判过时：去 ledger 文本，断言 sealed + Timeout + "1 succeeded" |
| 2b a_call_that_outlasts_the_grace_period_is_unsettled | 改 | Runtime 判过时：断言 "still running"（3402ms 符合设计） |
| 2b approval_wait_does_not_consume_the_program_budget | 改 | Runtime 判过时：read.file → read.path/lines |
| 2b direct_code_tools_are_refused_and_succeed_through_run_code | 改 | 过时脚本：read.file→path/lines、grep.matches→hits |
| 3 two_nested_writes_leave_one_complete_file_and_sequential_writes_both_apply | 改 | Runtime 判过时：删 2 条 ledger 文本断言，保留文件内容行为断言 |
| 3 conflicting_spellings_have_one_create_only_winner_and_a_read_waits_for_the_write | 改 | Runtime 判过时：lineText 用 lines[0]（字符串数组） |
| 3 edit_and_write_succeed_through_run_code_with_path | 改 | 过时：edit 改扁平 old_string/new_string，edited.ok |
| 3 historical_file_path_is_rejected_and_does_not_write | 改 | 过时：去 "error:invalid_input" ledger 文本，edit 扁平入参 |
| 3 cancelled_run_code_seals_an_in_flight_write_and_edit | 改 | 过时：edit 扁平入参；head 文本断言改为 error 为空或 Cancelled |
| 5 approval_pending_does_not_lock_other_calls | 改 | 过时：lines[0].text→lines[0]、matches→hits |
| run_code_only::deep_run_code_return_survives_the_host_process | 改 | 合并后自调用过滤名加模块前缀 |

### 其他集成测
| 测 | 处置 | 理由 |
|---|---|---|
| tests/ptc_contract_harness.rs（3） | 删整文件 | Q4：bundle 接口=SDK 快照；read/glob/grep 运行与 ptc/grep 单测重复 |
| tests/code_tool_runtime.rs（4） | 改 | 过时：data 现非空、grep hits、read path/lines/limit、失败文案 "timeout after"/"output limit" |
| tests/settings_api.rs apply_preset / put_agent_expands / mcp_server_rejects_builtin_id | 改 | 过时工具名 bash → run |
| tests/global_db_seed.rs config_global_db_migration_and_seed | 改 | run_command → run |
| tests/optional_lsp.rs idle_reclaim_during_diagnostics_does_not_panic | 改 | WP-206 #3：冷启动可 reject index-not-ready；测试目的（idle 后不 panic）保留 |

### death_list（GM：只留仍可能回潮的）
| 守卫 | 处置 | 理由 |
|---|---|---|
| tests/workspace_hotswitch_death_list.rs（2） | 删整文件 | 热切换功能已删，无引入路径 |
| session_data_sql_death_list::leftover_symbols_absent_from_repo | 删 | 已删符号（SessionGate 等）无引入路径 |
| workspace_path_death_list::death_list_forbidden_legacy_path_symbols | 删 | 已删符号（normalize_lsp_abs 等）无引入路径 |
| session_data_sql::sessions_db_sql_and_rusqlite_stay_in_sqlite_module | 留 | SQL 外溢随时可能回潮 |
| workspace_path 其余 3（canonicalize 白名单、web 静默 strip） | 留 | canonicalize 新调用随时可能出现 |
| code_index_tick（2）、startup_order（9）、terminal_bypass（3） | 留 | 后台索引/启动顺序/node-pty 依赖均有现役引入路径 |
| src/types/death_list_gate.rs（13） | 留 | provider catalog/方言/seq 语汇守卫，现役代码可回潮 |

## 总计
- 删除测试：lib 15 + 集成 11（code_surface 4、ptc_contract_harness 3、hotswitch 2、遗留符号 2）= 26 条；删 PRELUDE 断言 8 处。
- 并入保留 21 条；改写保留 lib 3 + 集成 21。
- 计数：lib 2226 → 2212；全量 2739 → 2714；删后 0 红。
