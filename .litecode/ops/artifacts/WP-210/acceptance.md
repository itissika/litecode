# WP-210 acceptance

代码 tip：3d5ebbe（53f5b59 Runtime / 7345454 Tools / 3d5ebbe Context）。分支 feat/tool-contract-ptc，未合 main。

## 全量
`cargo test -j 2 --no-fail-fast` @ 3d5ebbe：**846 passed / 0 failed / 0 ignored**（含 lib 559、集成、doctest）。flaky 重跑：无。明细见 cargo-full.txt。

## A–K 落地
| 项 | 结果 | 主要位置 |
|---|---|---|
| A | 失败头 `run_code failed: <label> · program: failed · tool calls: N ok / M failed`；附「completed tool calls before the failure」逐调用一行（brief ≤160 字符，超出 `… N more`） | src/tools/code/render.rs, src/tools/code.rs |
| B | 运行时异常 `line L, column C · snippet` + `stack:`（≤5 帧，映射 program.js 行，去 libs.js 帧） | src/ptc/mod.rs, engine.rs, render.rs |
| C | 结构化降级：数组保留 K + `"… N more"` → 深度 `{…N keys}`/`[…N items]` → 字符串 `…(N chars)`；表格整行截 `… N more rows`；提示在开头；结果仍为合法 JSON | render.rs |
| D | 成功 logs 保留开头 ≤200 tok，超出 `… N lines omitted`；失败保留尾部 | render.rs |
| E | boundary BTreeMap→有序 Vec，serde_json preserve_order 显式化；表格列按首行 key 序 | src/ptc/boundary.rs, json.rs, Cargo.toml |
| F | run：`output`=stdout、`stderr` 分字段、`shell` 身份、MSYS 路径提示；UTF-8 优先，Windows 回退 ANSI 代码页（encoding_rs，字节夹具单测） | src/terminal/decode.rs, src/tools/command_job.rs, run.rs |
| G | 领域失败（not_found/invalid_args/permission/io/no_match/empty）resolve `{ok:false, reason, message}`；系统码仍 reject ToolError；成功对象带前置 `ok:true`；edit 增 `reason` | src/tool/outcome.rs, src/tools/edit/* |
| H | read 仅未读完给 Showing/Continue；超长行 `Line N truncated to 1500 chars`；成功不显示 dropped undefined | src/tools/read.rs, render.rs |
| I | Code 内只读 `fs.ls/stat/exists`（工作区内，ls 非递归 ≤500）；LSP 七工具补入 builtin_code_surface_tools | src/ptc/fs.rs, engine.rs, src/tool/registry.rs |
| J | websearch `top_k`（默认 5，上限 10）→ `results[{title,url,snippet}]`，去 `text` | src/tools/websearch.rs |
| K | 24 处失效 Rust 引用修复（13 文件） | knowledge/** |

## 新增 protected（WP-208/protected.txt）
- src/tools/code/render.rs::failed_run_keeps_completed_calls_and_separates_program_from_tool_counts (A)
- src/tools/code/render.rs::over_budget_object_degrades_structurally_and_stays_valid_json (C)
- src/ptc/tests.rs::runtime_exception_reports_line_column_and_stack (B)
- src/ptc/tests.rs::object_keys_keep_agent_order_across_the_bridge (E)
- src/ptc/fs.rs::fs_probes_are_read_only_and_workspace_confined (I)
- src/terminal/decode.rs::run_output_is_utf8_or_transcoded_from_the_ansi_code_page (F)
- src/tool/outcome.rs::domain_failures_resolve_ok_false_and_system_failures_reject (G)
- src/tools/run.rs::run_separates_stdout_from_stderr_and_names_the_shell (F)
- src/tools/websearch.rs::websearch_top_k_yields_structured_results (J)
- 改名：render.rs::success_logs_collapse_to_one_line → success_logs_keep_the_head_within_200_tokens (D)

## 改动的 protected 断言（逐条）
| 测试 | 旧断言 | 新断言 | 原因 |
|---|---|---|---|
| render.rs::dropped_undefined_line_sits_in_the_head | 成功输出头部含 dropped undefined | 仅失败时出现，成功不得出现 | H |
| render.rs::success_header_counts_failures_from_call_states | 含 "1 succeeded, 2 failed" | 含 "program: ok · tool calls: 1 ok / 2 failed" | A 头部措辞（Q2） |
| render.rs::failure_header_counts_and_marks_unsettled | "N succeeded, M failed" | "program: failed · tool calls: N ok / M failed" | A |
| render.rs::uniform_object_array_renders_as_a_table | 列 `{hit,line,path}` | `{path,line,hit}` | E 保序 |
| render.rs::table_nested_in_an_object_renders_as_a_keyed_block | 字母序 key | 原序 total,note,`hits[3]{p,n}` | E |
| render.rs::table_is_truncated_on_whole_rows | `{line,path}`，无尾标 | `{path,line}` + `… N more rows` | C/E |
| render.rs::success_logs_collapse_to_one_line（改名） | "logs omitted (N lines)"，不含日志 | 保留开头 ≤200 tok，超出 `… N lines omitted` | D（Q5） |
| code.rs::unfinished_call_stays_unsettled… / permission_denial_is_counted… / render_run_output_attaches… | "X succeeded, Y failed" | "tool calls: X ok / Y failed" | A |
| websearch.rs::soft_fail_shape_resolves_ok_false | 构造 `text: None` | `results: Vec::new()` 且断言 results 为空 | J 去 text（Q14） |
| sdk_snapshot.rs::phase0_run_code_sdk_snapshot | fixture 旧；全文不得含 `view`、`file_path` | fixture 刷新；`view` 改为整词匹配；`file_path` 仅禁止于 Read/Edit/ApplyEdits/Write/Grep/Glob 类型块 | I 补入 LSP 工具，其 schema 按 WP-202 用 `file_path`、示例含 `preview` |
| baseline::phase0_tool_fixtures_lock_data_and_error | 旧 fixture | 刷新 edit__empty / edit__partial_failure（新增 reason）、read__truncation（H 文案） | G/H |
| tests/code_surface.rs::direct_code_tools_are_refused_and_succeed_through_run_code | "3 succeeded, 0 failed" | "tool calls: 3 ok / 0 failed" | A |
| tests/code_surface.rs::outer_timeout_does_not_overwrite_a_completed_nested_result | "1 succeeded" | "tool calls: 1 ok" | A |
| tests/code_surface.rs::nested_write_approval_denial_does_not_apply_the_edit | 脚本靠 catch 置 error；"0 succeeded, 1 failed" | 脚本另判 `ok === false`；"tool calls: 0 ok / 1 failed" | G 权限拒绝 resolve；A |
| tests/code_surface.rs::nested_write_sensitive_path_is_denied_by_the_same_floor | 同上 | 同上 | G/A |
| tests/code_surface.rs::approval_pending_does_not_preclaim_the_same_path | read 缺文件须 throw ToolError；"0 succeeded, 1 failed" | read 须 resolve `ok:false`；"tool calls: 0 ok / 1 failed" | G/A |
| tests/code_surface.rs::conflicting_spellings_have_one_create_only_winner_and_a_read_waits_for_the_write | 脚本以 catch 判 read 失败 | 脚本经 `must()` 把 `ok:false` 视为失败；Rust 断言不变 | G |
| tests/code_surface.rs::cancel_before_approval_does_not_write_later | 允许 "0 succeeded" | 允许 "tool calls: 0 ok" | A |
| tests/run_code_model_view.rs::caught_tool_error_is_counted_and_not_listed | catch ToolError not_found；"0 succeeded, 1 failed" | resolve `[false,"not_found",…]`；"tool calls: 0 ok / 1 failed" | G/A |
| tests/run_code_model_view.rs::uncaught_tool_error_names_the_call | read 缺文件 → "uncaught ToolError not_found … read path=missing.txt" | write create_only 冲突（系统码）→ "uncaught ToolError conflict … write path=taken.txt" | G：not_found 不再抛，改用仍 reject 的系统码保持测试意图 |
| tests/code_tool_runtime.rs::code_tool_captures_logs_and_enforces_output_ledger | "logs omitted (2 lines)"，不含 "hello 42" | 含 "[log] hello 42"，不含 "logs omitted" | D |

另：src/ptc/fs.rs 改用 `config::path::canon_abs`，以满足 workspace_path_death_list（未改该测试）。

## 遗留
1. QuickJS 怪癖：函数内对 null 取属性报错，栈只给函数所在行、column 1；显式 throw 精确。B 测试用显式 throw。
2. F 的 Windows 路径（GetACP/CP936、MINGW 身份）仅字节夹具单测，需 Andy 本机验证。
3. K 只核 Rust 引用；剩余 24 处 TS `fn X` 为误报，未处理。
4. `fs` 按计划实现在 engine（QuickJS 全局），非独立工具，不计入工具调用数。
5. examples.md 为代表性格式片段（与单测断言一致），非真实会话录制。
