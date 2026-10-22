# WP-206 C 组（IDE）— ③ LSP 冷启动 + ⑧ LSP 路径

- 分支 feat/tool-contract-ptc，基线 236edfe，本地 commit d36d11a（未 push，未合 main，未刷 SDK fixture）
- 只改：src/lsp/server.rs, src/lsp/hub.rs, src/lsp/format.rs, src/tools/lsp.rs
- **cargo 未跑**（等后端中层放行）

## ③ 设计
- server.rs：`nav_empty` 提为模块级 fn；documentSymbol / workspace/symbol 以 null 或空数组判空（不用 extract_locations，避免 DocumentSymbol 树被误判为空）。
- hub.rs：documentSymbol、workspaceSymbol（tool_action_async 与 workspace_symbol_outcome 两处）改走 `request_nav_with_retry`，与 hover/definition 同一套 index_ready_for_empty_miss / INDEX_GRACE(20s) 判定 → Err "index not ready" → tools 层 `unavailable`。
- diagnostics：新增 `LspServer::wait_file_diagnostics_gated` + 纯函数 `diagnostics_gate(server_ready, fresh, index_ready)`：未运行→unavailable；有覆盖当前版本的 publish→返回；未 publish 且宽限内→unavailable；宽限过期→空成功（dig-survey Q5）。
- 公共错误 `index_not_ready_error(method)`，文案不变，map_lsp_query_error 已映射 unavailable。
- 未动：IPC `litecode/getDiagnostics`（前端契约）、post-write `wait_file_diagnostics`（编辑反馈）。

## ⑧ 设计
- format.rs：`display_path(path, roots)` / `display_path_for_uri` 复用 `workspace::filter::cheap_rel_under`；工作区内相对（`/` 分隔，根本身为 "."），外部绝对；纯字符串前缀，不 canonicalize、不解析 symlink。
- `extract_location_items` / `extract_symbol_items` / `format_action_result` / call hierarchy 加 `roots: &[&Path]`；`extract_locations` 仍输出绝对路径（server.rs/mod.rs 内部判空用，签名不变）。
- tools/lsp.rs 传 [canon 根, 配置根]；hub 文本输出用 hub.workspace_root()。

## 测例
新增：
- lsp::server::tests::nav_empty_symbols_by_array_len
- lsp::server::tests::diagnostics_gate_table
- lsp::server::tests::index_not_ready_error_maps_to_unavailable_wording
- lsp::format::tests::{location_inside_workspace_is_relative, location_outside_workspace_stays_absolute, display_path_does_not_resolve_symlinks, display_path_tries_roots_in_order, workspace_symbol_paths_relative_inside_absolute_outside, format_locations_text_uses_relative_path}
- tools::lsp::tests::{index_not_ready_rejects_unavailable_for_symbols_and_diagnostics, finish_locations_relative_inside_absolute_outside, finish_symbols_workspace_paths_relative}
修改：tools::lsp::tests::finish_locations_empty_resolves_with_empty_array（仅签名，传 &[]）

## 申请占槽时跑（CARGO_BUILD_JOBS=1，不跑全量）
```
CARGO_BUILD_JOBS=1 cargo test --lib -- lsp::server::tests lsp::format::tests tools::lsp::tests
CARGO_BUILD_JOBS=1 cargo test --lib -- lsp::
```
第二条为回归（全部 lsp 模块单测）。- **待复核（Windows）**：`lsp::format::tests::display_path_does_not_resolve_symlinks` 使用 "/ws/..." Unix 形路径，Windows 上未验证。
- 冷启动（③）只用纯函数测例（nav_empty / diagnostics_gate / 错误码映射）验证，不做真实 LS 端到端——此口径后端中层已认可。

## 运行结果（2026-10-22，CARGO_BUILD_JOBS=1，MemAvailable ≈5.0GiB）
- test-1.log：30 passed / 0 failed，耗时 1m29.7s（含编译）
- test-2.log：130 passed / 0 failed，耗时 0.9s（增量，无重编）
- 最终 sha：d36d11a（无需修复，未 amend）
