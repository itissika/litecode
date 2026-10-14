# WP-148（Tools）交付：code_search 可见性只看 bind；运行状态进调用结果（契约 v1 A+C）

## 改动（未 commit；engines/前端/session_search 行为未动）
- src/tool/availability.rs
  - `is_available`：code_search 恒可用（不再看 readiness=desired）；lsp 照旧。
  - `available_tools`：code_search bind 卡片恒列出；lsp 照旧按 readiness。
  - `is_retrieval_surface_tool/retrieval_tools_visible` → `is_session_search_gated/session_search_visible`：只作用于 session_search，函数体（desired + is_searchable）逐字保留。code_search LLM 表 = bind。
- src/tools/mod.rs
  - 新增 `format_sync_pct_line_labeled(sync, Option<SyncLineLabel>)`，为唯一首行构造点；`format_sync_pct_line` 委托给它（label=None 行为不变）。
  - `SyncLineLabel::{EngineOff, Warming, Failed(&str)}`；`sanitize_sync_label`：去 `()`、控制字符/换行转空格、合并空白、截断 60 字符，空则 `engine failed`。
- src/tools/code_search.rs
  - `not_ready_result()` 在 maybe_consume_index 与检索之前判定：engine off → failed → warming → None(就绪走原路径)。全部 `ToolCallResult::ok`，正文无命中、零检索。
  - 首行：`sync_pct: unknown (engine off)` / `sync_pct: failed (<净化原因>)` / `sync_pct: N% (warming) chunks d/t` 或 `sync_pct: scanning (warming)`；正文：`代码引擎未开启，无结果。可在设置中开启代码检索引擎。` / `代码引擎失败：<原因单行>` / `引擎预热中，暂无结果，请稍后重试。`
  - 删除 defensive_not_ready_message / enable_retrieval_message / indexing_wait_message 及 warning/error 级返回。
  - cfg(test) thread_local `SEARCH_CALLS` 计数（紧贴 engines.search 前）证明非就绪零检索。

## 与方案的偏差（小）
- 失败原因优先级：先用引擎 detail（last_error/gate detail）净化，空时才用 progress 的 reason 码（方案写的是反过来）。原因：progress 在 Failed 下总带 `engine_unavailable`，会盖掉真实原因，也使净化反例测不到。
- 场景测试通过 `should_include_in_llm_list`（build_tool_list L219 用的同一门禁）断言可见，而非起完整 RuntimeHandle。
- `warm_refreshing_does_not_block_on_wait`、`warm_refresh_busy_does_not_block_on_wait` 仅把夹具从 init_workspace_index 换成 enable_code_search_engine（desired=true，否则新规则会先返回 engine off）；断言未改。

## 测试结果（cargo 1.99，-j2，主 target，</dev/null，编译前 MemAvailable≥6GiB）
- `cargo test --lib tools::` → 440 passed, 0 failed（原 436 + 新增 4：failed_reason_with_parens_and_newlines_is_sanitized、four_returns_first_line_matches_v2_regex、not_ready_issues_no_search、warming_turn_sees_code_search_and_gets_warming_line）。
- `cargo test --lib tool::availability` → 11 passed, 0 failed（新增 3：code_search_visible_when_bound_engine_failed、code_search_hidden_when_unbound、code_search_bind_card_listed_when_desired_false）。
- `cargo test --lib tool::` → 83 passed, 1 failed：`tool::registry::tests::core_bindings_produce_full_builtin_list`，即既有 ISSUE-055（期望 session_search，非本包引入）。
- `cargo fmt --check` → 干净（0）。
- 未跑 tests/optional_p0.rs（集成测试，不涉 code_search 可见性；需额外编译，留给验收统一跑）。

## 8 个改写的测试（断言变化）
1. availability `retrieval_hidden_when_desired_false` → `code_search_visible_when_bound_engine_off`：code_search 断言从「desired=false 时隐藏」改为「bind=true 即可见」（session_search 仍断言隐藏）。
2. availability `retrieval_hidden_while_warming_even_if_desired` → `code_search_visible_when_bound_while_warming`：code_search 断言从「Warming 时隐藏」改为「Warming 时可见」（session_search 仍隐藏）。
3. `warming_returns_immediately_without_sleeping`：从「Warning 级 + 含 Warming/not ready 英文」改为「Ok 级 + 首行过 v2 且带 (warming) + 预热中文正文」。
4. `desired_false_defensive_points_at_settings` → `engine_off_returns_off_line_and_empty`：从「Warning 级 + 含 Settings → Engines」改为「Ok 级 + 正文恰为 `sync_pct: unknown (engine off)` + 未开中文文案」。
5. `indexing_wait_message_reads_job_json`：从「indexing_wait_message 含 refreshing 和 chunks 4/8」改为「call_inner 首行过 v2 且以 `% (warming) chunks 4/8` 结尾、无假命中」。
6. `indexing_wait_message_uses_chunk_progress_not_pending_hint`：从「indexing_wait_message 含 refreshing、无 files pending」改为「call_inner 首行过 v2 且带 (warming)、不含 files pending / 53 估计」。
7. `failed_engine_returns_last_error`：从「Error 级 + 含 last_error」改为「Ok 级 + 首行 `sync_pct: failed (…)` 过 v2 + 正文含 last_error」。
8. `warm_failed_index_job_returns_error_not_hits`：从「Error 级 + 含 embed exploded」改为「Ok 级 + failed 首行过 v2 + 正文含原因 + 零检索调用」。

删除的测试：无。

## 补充（验收前，未重新 deliver）
### 集成测试改写
9. tests/optional_code_search.rs `catalog_hides_tool_while_warming_shows_after_warm` → `catalog_lists_tool_while_warming_and_after_warm`：Warming 期间断言从「should_include_in_llm_list 为 false（隐藏）」改为「为 true（bind 即可见）」；Warm 后可见断言不变。
10. tests/settings_api.rs `settings_retrieval_engine_init_and_stop_use_engine_api`：engine stop 后断言从「available-tools 不含 code_search」改为「仍含 code_search，且对该工作区调用 code_search 首行为 `sync_pct: unknown (engine off)`」。

### 场景（契约验收第 2 条）
`tools::code_search::tests::warming_turn_sees_code_search_and_gets_warming_line`：同一 WorkspaceEngines 实例先 Warming → code_search 在 LLM 表门禁中可见、调用首行过 v2 且带 (warming)、正文为预热文案、检索计数不变；再置 Warm → 下一次调用检索计数 +1（走正常路径）。全程显式断言 bind（agents.default.tools.code_search.enabled）与 engines.json desired 前后不变。对应 WP-145-backend.md「验收 2」。

### 补充结果（cargo 1.99，-j2，主 target，</dev/null，编译前 MemAvailable≥6GiB）
- `cargo test --lib tools::` → 440 passed, 0 failed
- `cargo test --lib tool::availability` → 11 passed, 0 failed
- `cargo test --test optional_p0` → 8 passed, 0 failed（无需改写）
- `cargo test --test optional_code_search` → 19 passed, 0 failed（改写 1 条后）
- `cargo test --test settings_api` → 48 passed, 0 failed（改写 1 条）
- `cargo fmt --check` → 0
