# WP-063 / WP-067 — IDE：copy_path 保链接（ISSUE-010）

- 角色：ide（IDE 基座）；父 WP-064 / 合包 WP-063
- 基线：HEAD `a72ad6a`，分支 `ops/team-framework`，2026-10-12 21:57 HKT
- 产物：本文件；**不 commit**
- 禁区：ISSUE-004（仍 open）、引擎产品、不 Escape

## 1. 口径迁移（拒拷 → preserve）

| | 旧（WP-004 / ISSUE-010 原 promise） | 新（Andy 2026-10-12 锁定） |
|---|---|---|
| 行为 | 拒绝复制 symlink（`refuse_top_level_symlink_copy` + `copy_recursive` 拒） | **保链接**：lexical readlink → sandbox 门 → dest 重建 symlink |
| 顶层 | 拒 / 或曾经 follow 后抄内容 | preserve，不解引用抄内容 |
| 嵌套 | 拒 | 同样 preserve |
| 出界 | （拒拷掩盖） | **Escape**，不落半成品 |
| 测 | `refuses_to_copy_symlinks` | `preserves_symlink_top_level` / `preserves_symlink_nested` / `refuses_symlink_copy_escaping_workspace` |
| promise | 拒绝符号链接 | 保链接；sandbox 仍门；链出 Escape |

## 2. 改动摘要

**文件：** `src/workspace/service.rs`（主）

- 删除 `refuse_top_level_symlink_copy`。
- 新增：`lexical_relative_path`、`preserve_symlink_at`、`ensure_link_target_in_workspace`、`create_symlink`、`refuse_symlink_node_overwrite` 等。
- `copy_path`：顶层若为 symlink，不走 `Sandbox::resolve`（避免 canonicalize 跟随）；否则旧路径 + `copy_recursive(root, …)`。
- `copy_recursive`：symlink 节点改调 `preserve_symlink_at`（嵌套一致）；签名带 `root`，返回 `WorkspaceError`。
- `move_via_copy`：传入 `root`，删除源时用 `symlink_metadata`（不跟随）；自动 inherit preserve。

**未改：** `src/engines/**`、`src/lsp/**`、ISSUE-004 / `tests/optional_lsp.rs`。

## 3. 测试

命令：
```bash
HOME=/workspace/litecode/.tmp/home timeout 600 cargo test --lib workspace::service:: -- --nocapture
```

结果（HEAD `a72ad6a` 工作树含本包 diff）：**7 passed / 0 failed**，含：

- `preserves_symlink_top_level`
- `preserves_symlink_nested`
- `refuses_symlink_copy_escaping_workspace`
- 既有 `mkdir_rename_copy_*` / `refuses_directory_overwrite_*` / `delete_*` / broadcast

说明：集成测 `wp028_probe_*` 本次未能重编——并行域 Runtime 引入 `InternalEvent::TurnCompleted.error_kind`，与本包无关；本域 lib 测已覆盖 copy/symlink 回归。

## 4. ISSUE-010 / promise / 知识节点

- **ISSUE-010** → `closed`
- **promise** → `workspace copy 保链接（preserves_symlink 顶层+嵌套）；sandbox 仍是唯一路径门；链出工作区拒绝（Escape）；不解引用抄内容`
- repro 指向新测名
- polish（不改主张）：
  - `knowledge/contracts/workspace-sandbox.md`（pending）：ISSUE-010 注记改为保链接口径
  - `knowledge/harness/workspace-shared-substrate.md`（enabled）：同上，唯一门主张不变
  - `.litecode/ops/context/issue-grading.md`：描述/测名跟新
- **ISSUE-004** 仍 `open`（未碰）

## 5. move_via_copy 副作用

跨盘 move 共用 `copy_recursive`，symlink 节点一并 preserve；删源不跟随链接。无单独产品承诺，行为与 copy 一致。

## 6. 门禁自检

- [x] 引擎产品未动
- [x] ISSUE-004 未碰、仍 open
- [x] 链出工作区 → Escape（测绿）
- [x] 未 commit / 未 push
- [x] 与 WP-068/069 文件面不交（仅本域 service.rs + 知识/issue 文案）

## 7. 交叉审（Tools 生命周期）

本包无新增子进程；沿用既往结论，无新发现。
