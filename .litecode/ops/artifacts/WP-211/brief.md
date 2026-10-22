# WP-211 run_code 体验残留三刀 — brief

分支：`feat/tool-contract-ptc`（不合 main）。来源：Andy 本机实测（WP-210 之后）。**仅三项，产品已钉死；无开放问题。** 不得触碰 `desktop/build/icon.ico`。补测包仍后置。

## 已定范围（仅此三刀）

**1. write conflict/stale → 软失败**

- `soft_failure_reason` 扩展纳入 `conflict` | `stale`（与 WP-210 G 的 ok 语义一致：活没干成 → `ok:false`，不抛）。
- 程序侧 resolve `{ ok:false, reason: "conflict"|"stale", message }`；timeout / cancelled / unavailable / internal / unsupported 等仍 throw。
- 更新 `write.md`、PRELUDE / `ToolFailure.reason` 联合类型（去掉「mismatch rejects ToolError」与 `Promise<WriteData | ToolFailure>` 的自相矛盾）。
- protected：`tests/run_code_model_view.rs::uncaught_tool_error_names_the_call` 现用 create_only conflict 保住「仍会抛」意图——本刀后 conflict 不再抛，须二选一：换仍会 throw 的用例（如 timeout / cancelled / unsupported），或改为断言 `ok:false` + reason；意图写进 acceptance。

**2. MSYS pathconv 假成功（run）**

- 当 `msys_path_hint` 命中（如 `cmd /c`）时：返回 **`ok:false`**、**`exit_code: null`**，保留 `message` + `shell`（stdout/stderr 可留作诊断）；**禁止**再挂 `exit_code: 0`。
- **不加** `timeout_ms` / `background`（run 保持简单）。仅修结果诚实性。

**3. LSP 描述门禁**

- 从 `code_manifest.rs` 的 PRELUDE **删掉**写死的七个 LSP 名（`go_to_definition` / `find_references` / `hover` / `diagnostics` / `document_symbol` / `workspace_symbol` / `go_to_implementation` 那句）。描述**只**经 admitted tool blocks 组装。
- 回归：settings / binding `desired=false`（或等价未准入）时，`render_code_tool_description` 与 projected whitelist **均不含** `LSP_CODE_TOOL_NAMES` 中任一名字。
- **不要**从 `builtin_code_surface_tools` 移除七工具；**不要**削弱 `build_tool_list` 的 Ready+bind 门禁。根因是 PRELUDE 硬编码绕过了「Settings 未开则描述块不进」的管线。

## 验收

1. `cargo test -j 2` 绿（跑前空闲内存 ≥2GiB），产物可附 `cargo-full.txt`。
2. 仅为本三刀补精简 protected；不堆测试量。
3. SDK fixture 与 prompt/工具文档同步。

## 非目标

read 分页字段、console.log 加宽、跨调用状态、rm/mv、timeout_ms/background、知识库以外的新范围。
