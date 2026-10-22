# WP-209 acceptance — run_code 模型视图整形 + token 预算

代码提交：bb4cfdc（基线 90654c1，分支 feat/tool-contract-ptc，未合 main）。全量 `cargo test -j 2` @ bb4cfdc：**837 passed / 0 failed**（825 + 新增 12），日志 cargo-full.txt。

| brief 验收 | 结果 |
|---|---|
| 1 表格/回退/转义测试 | uniform_object_array_renders_as_a_table、non_uniform_nested_or_short_arrays_fall_back_to_compact_json、table_cells_quote_ambiguous_strings、table_nested_in_an_object_renders_as_a_keyed_block、other_values_are_compact_json_and_strings_verbatim |
| 2 token 预算（含 CJK）+ 字节兜底 | cjk_result_is_cut_by_tokens_not_bytes、table_is_truncated_on_whole_rows、byte_fallback_still_caps_the_view、large_result_is_token_truncated_and_states_the_omission |
| 3 成功 logs 一行 / 失败 ≤2k | success_logs_collapse_to_one_line、failure_logs_keep_the_tail_within_2k_tokens；集成 code_tool_captures_logs_and_enforces_output_ledger |
| 4 超限提示仅超限、位于 result 开头 | notice_appears_only_when_over_budget + 上述截断测试断言首行 |
| 5 PRELUDE + SDK fixture sha/快照 | 前置段（摘要优先 / 只有 return 可见 / 8k·2k / logs omitted / 表格形态）；run_code_sdk.txt + .sha256 已刷，phase0_run_code_sdk_snapshot 绿 |
| 6 measurements.md | 见同目录 |
| 7 全量绿 | 837/0 |
| 8 push 不合 main | 是 |

GM 批复落实：Q1–Q6、Q8、Q9 按推荐；Q7 英文 `logs omitted (N lines)`。前端/human_data 不动（human_data_stays_structured_for_the_pretty_frontend）。旧结果不裁剪（未触及历史/上下文管线）。64KB 字节兜底保留。

## 实现说明
- 列序：serde_json 未开 preserve_order，对象键按排序迭代，表头列序 = 排序后的键（与 Q1「列序取首元素键序」一致，首元素键序即排序序）。
- 截断：行级二分取前缀（表格不截半行）；单行超限时按字符二分。快速放行：bytes ≤ 预算直接放行（每 token ≥1 字节）。
- 失败 logs：尾部二分，开头 `… [N earlier lines omitted]`；最后一行超长时保尾，`[output limit reached]` 标记保留。
- 删除旧 LOGS_FLOOR / logs_budget / render_logs 等字节抢预算逻辑（已无调用）。

## 契约变化导致的既有测试改动（均在 protected 内）
- render.rs `large_result_keeps_the_tail_and_states_the_omission` → 改名 `large_result_is_token_truncated_and_states_the_omission`（Q6 前缀截断，不再保尾）；protected.txt 同步改名。
- render.rs `default_view_stays_inside_the_budget_without_legacy_sections`：`bytes omitted` 断言放宽为 omitted/truncated。
- tests/run_code_model_view.rs `large_result_is_budgeted_in_the_request`：END_SENTINEL/bytes omitted → 首行超限提示 + tok 标记。
- tests/code_tool_runtime.rs `code_tool_captures_logs_and_enforces_output_ledger`：成功断言 `logs omitted (2 lines)`；新增失败分支断言 logs 可见。
- protected.txt：767 → 779（+12 新增）。

## 偶发
本包两次全量（第一次因 run_code_model_view 契约断言未改而停）均未见偶发失败。

## 未做 / 遗留
- 未提交工作区里他人改动：board.json / log.jsonl / tools/board.md、tool_traces.jsonl、desktop/build/icon.ico（删除状态，未碰）。
