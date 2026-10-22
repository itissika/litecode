# WP-205 acceptance
- 根因：测试过时（见 dig-survey.md）；产品代码未改，未加反引号。
- 改动（仅测试）：
  - `src/engines/session_search/mod.rs`：新增 `#[cfg(test)] named_spill_location`（取 ` are in ` 后首个 token，去尾句号）；`agent_view_spills_the_remainder_into_a_named_file` 改用它。
  - `src/tools/session_search.rs`：`a_result_over_budget_names_the_file_that_holds_the_rest` 共用该 helper；`search_hit_line_matches_virtual_read` 改为 `start_line`+`limit`，断言 `data.lines` 覆盖 hit 范围且含 token。
- 验收：
  - [x] 点名溢出文件可读（两测读到 Remaining/needle）
  - [x] hit 行与虚拟 read 一致（行数=范围，含 token）
  - [x] `cargo test -j 2 --lib session_search`：201 passed / 0 failed / 1 ignored（available 9GiB）→ cargo-session_search.txt
  - [x] dig 写清根因；推 feat/tool-contract-ptc，不合 main
