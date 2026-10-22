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
