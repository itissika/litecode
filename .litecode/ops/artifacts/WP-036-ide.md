# WP-036 / WP-040 — IDE 基座：ISSUE-037 / ISSUE-038 修复

- HEAD：`104fd43`（分支 `ops/team-framework`），日期：2026-10-12 19:49 HKT
- 执行：ide；审批：backend_mid。**不 commit**（本包只改工作区）。
- 工具链：rustc 1.99.0；测试在 `/workspace/litecode` 直接 `cargo`（无 `PATH=/home/box/.cargo/bin` 前缀）；`HOME=/workspace/litecode/.tmp/home`；命令末 `</dev/null`；单测 `timeout 600`。

## 改动文件

| 文件 | 要点 |
|---|---|
| `src/workspace/file_ops.rs` | 新增 `atomic_create` / `atomic_create_bytes`（tmp + `hard_link` 排他发布，失败映射 AlreadyExists；硬链不可用时回退 `create_new`）+ 单元测 |
| `src/workspace/service.rs` | `create_file` / `write_file_bytes(overwrite=false)` 改走排他创建；`rename_path` 在 `replace_file` EXDEV 时走 `move_via_copy`；`move_via_copy` 失败时不删预先存在的目标 |
| `tests/wp028_probe_s2_create_race.rs` | 去掉 `#[ignore]`，正向断言 300 轮无双成功 |
| `tests/wp028_probe_s1_xdev_move.rs` | 去掉 `#[ignore]`；无双 FS 可 skip；有 `/dev/shm` 时断言跨盘 overwrite 成功 |

**未改**：`refuse_top_level_symlink_copy` / `copy_recursive` 拒 symlink；engines；take_changes；前端。

## ISSUE-037（并发 create 静默覆盖）

**严重度**：产品缺陷 / 常触发。

**前**：`create_file` / `write_file_bytes(overwrite=false)` 先 `exists()` 再 `atomic_write`（rename 可覆盖）。探针 8×300：两 API 均 300/300 多成功。

**后**：
- `file_ops.rs:31–` `atomic_create` / `atomic_create_bytes`
- `service.rs:270–` `create_file` → `atomic_create`，`AlreadyExists` → `WorkspaceError::AlreadyExists`
- `service.rs:416–` `write_file_bytes`：`overwrite=false` → `atomic_create_bytes`；`overwrite=true` 仍 `atomic_write_bytes`

**复现/回归**：`HOME=…/.tmp/home timeout 600 cargo test --test wp028_probe_s2_create_race -- --nocapture </dev/null`  
结果：`create_file` / `write_file_bytes` 均 `rounds_with_multiple_successes=0`，2 passed。

**关闭建议**：status → closed；依据 WP-040 + 探针绿。

## ISSUE-038（跨盘 overwrite → CrossesDevices）

**严重度**：罕见边角。

**前**：`rename_path` 在 dest 已存在 + overwrite 时先 `replace_file`（仍 rename）；EXDEV 直接返回，`move_via_copy` 不可达。源与原目标完好但操作失败。

**后**：
- `service.rs:354–368`：`replace_file` 遇 EXDEV → `move_via_copy`
- `service.rs:515–`：`move_via_copy` 仅在目标**原先不存在**时 `cleanup_incomplete_dest`（失败不误删原目标）

**复现/回归**：`… cargo test --test wp028_probe_s1_xdev_move -- --nocapture </dev/null`  
结果：`rename_path` Ok，dst=`NEW`，src 已删；1 passed。

**关闭建议**：status → closed；依据 WP-040 + 探针绿。

## 测试结果（本包）

| 套件 | 结果 |
|---|---|
| `wp028_probe_s2_create_race` | 2 passed |
| `wp028_probe_s1_xdev_move` | 1 passed |
| `workspace::service::` (lib) | 5 passed（含 `refuses_to_copy_symlinks`、`mkdir_rename_copy_and_reject_self_copy`） |
| `workspace::file_ops::` (lib) | 8 passed（含新建 atomic_create 测） |
| `--test workspace_api` | 12 passed |
| `--test workspace_path_consumer` | 4 passed |
| `--test workspace_process_contract` | 7 passed |

说明：全量 `workspace::` filter 另有 2 个既有失败（`git::history_helpers…`、`symbols::drift_is_false…`，依赖本机 git add），与本包改动无关，未纳入关闭条件。

## 交叉审

本包无新增子进程。沿用 WP-028 对 Tools `custom.rs` 的 X1/X2/X4；本包无新发现。symlink / 引擎 / take_changes 未碰。

## Issue 关闭条件（已满足）

1. ISSUE-037：并发 create 不再静默覆盖；探针默认绿；AlreadyExists 契约保持。
2. ISSUE-038：跨盘 overwrite 走 copy 回退成功；失败不删原目标；探针默认绿。
3. 禁区未碰；不 commit。
