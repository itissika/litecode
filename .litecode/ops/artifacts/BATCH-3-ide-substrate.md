# Batch 3 · ide-substrate 实现对照核实

- 负责人：IDE 基座（ide / ide-substrate）
- 日期：2026-10-12（HKT）
- 工作区 HEAD：`075308b`（核实基线；工作树有本域未提交改动）
- 分支：`ops/team-framework`
- 作用域：`src/workspace` `src/engines` `src/lsp` `src/terminal` `src/ide_base.rs` `src/proc_lifetime.rs`
- **未 commit / 未 push**

## 1. 节点对照表

| key | status | 结论 | 证据（文件 + 符号/测试） |
|---|---|---|---|
| workspace-shared-substrate | enabled | **一致，无实质漂移** | `Sandbox::resolve`、`WorkspaceWatcher::start`、`WorkspaceLock::acquire`；终端死亡清单通过。ISSUE-010 为 copy 组合 bug，不否定唯一安全门；ISSUE-007 为热切换门禁误报。 |
| engine-lifecycle | enabled | **一致，无实质漂移** | `EngineState` + `workspace_engine_desired`（`src/engines/mod.rs`）；`status_view` 投影；子节点 lsp-hub / process-lifetime 已挂。ISSUE-009 关（gate）。 |
| retrieval-contract | enabled | **一致，无实质漂移** | `session_search/echo.rs` + `ranking.rs`；词法腿无索引加速符号。ISSUE-002 关（gate 断言已修）。 |
| engine-design | enabled | **一致，无实质漂移** | 理念层与 lifecycle/retrieval 契约一致；configured≠Warm。 |
| workspace-sandbox | pending | **一致，无实质漂移** | `impl Sandbox › fn resolve`；`rejects_parent_dir_escape` 通过；与 `resolve_human_relative` 共用。ISSUE-010 在 resolve **之后**丢 symlink 语义。 |
| workspace-watcher | pending | **一致，无实质漂移** | sole notify owner + 排除源端硬切；`reload_workspace_excludes_from_disk` ≠ 根热切换。ISSUE-007 open。 |
| terminal-pty | pending | **一致，无实质漂移** | `PtySession::spawn_interactive` / Drop；`install_hub`；`terminal_bypass_death_list` 3/3 通过。 |
| lsp-hub-lifecycle | pending | **一致，无实质漂移** | `LspHub::activate` 只登记；`stop` drain+shutdown；`LspEngine::warmup/stop`。ISSUE-004 suspect 旁听。 |
| process-lifetime | pending | **一致，无实质漂移** | `bind_child_to_parent` / `sweep_orphan_lsp_processes`；`CodeSearchEngine::stop`→`kill_client`；warmup stop 测通过。ISSUE-009 关。 |

**统计：** 一致 **9** / 不能扩写 **0**。无 core 节点结论 B → **不触发 B2**。

节点正文均已按 admission §7.2 追加结论 A。

## 2. 正式 issue 列表

机读目录：`.litecode/ops/issues/`（按 schema 落盘）。

| id | grade | layer | core | status | 变更 | 摘要 |
|---|---|---|---|---|---|---|
| ISSUE-002 | drift | gate | 是（retrieval-contract） | **closed** | 种子→确认失败→修断言 | 词法腿门禁 2 参→4 参；不变量仍成立 |
| ISSUE-004 | suspect | product | 否 | open | 种子保持 | optional_lsp findReferences 上下文；主责或在 tools |
| ISSUE-007 | drift | gate | 否 | open | 种子→复现并扩证据 | 热切换死亡清单误伤 excludes / open_with_default_app |
| ISSUE-009 | drift | gate | 是（engine-lifecycle） | **closed** | suspect/env→drift/gate→修测试 | Unix 僵尸使 `is_process_alive` 误报；SIGKILL 已送达 |
| ISSUE-010 | bug | product | 是（workspace-shared-substrate） | open | 种子→复现确认 | `copy_path` resolve 跟随顶层 symlink 后复制目标；不越界非 B3 |

### ISSUE-010 证据链（优先）

1. `WorkspaceService::copy_path` 对 `from` 调用 `sandbox.resolve`（`src/workspace/service.rs`）。
2. `Sandbox::resolve` → `resolve_human_relative` → 存在路径走 `canon_abs`（`canonicalize`，跟随符号链接）。
3. `copy_recursive` 对已解析路径做 `symlink_metadata`：顶层链接已变成目标文件 → 当作普通文件 `std::fs::copy`；子目录内仍是链接的条目会被拒。
4. 复现：`workspace::service::tests::refuses_to_copy_symlinks`（unix）在 `075308b` **FAILED**（期望 `InvalidMove`，实际 `Ok`）。
5. 目标仍在工作区内 → **不触发 B3**（全局唯一安全边界未被绕过出界）。

本轮**未**修产品行为（非极小门禁）；建议后续：在 `copy_path` 于 resolve 前对相对路径做 `symlink_metadata`，或 resolve 提供「不跟随」变体。

### 本轮直接修（门禁，有 issue 记录）

| 改动 | issue | 说明 |
|---|---|---|
| `lexical.rs` 测试断言改为 4 参 | ISSUE-002 | 极小可逆 |
| `stop_kills_*` 先 `try_wait` 再探活 | ISSUE-009 | 极小可逆；避免僵尸误报 |

未改 `workspace_hotswitch_death_list`（ISSUE-007 建议收窄 FORBIDDEN，待 GM/下轮，禁止静默为绿）。

## 3. 待删 / 待改非知识 md 清单

本域撞车来源扫描：`docs/` 仅 ADR + `knowledge-references.md`，**无**与 sandbox/watcher/PTY/LSP 主张重复的杂 md。

| 路径 | 建议 | 理由 |
|---|---|---|
| （无） | — | 无待删候选；死亡清单本身是测试不是文档 |
| `tests/workspace_hotswitch_death_list.rs` | **待改**（非删） | ISSUE-007：收窄 `reload_workspace` / 排除 `post_open`（实为 `open_with_default_app`）与 `api/workspace/open` |

删前经 GM（本批只标清单）。

## 4. 互引补全说明

Batch-2 alignment 开放互引（client-projection↔session-model 等）**不在本域强制项**。本域侧：

- 存量父节点已有 `向下展开` / `契约展开` → pending 子节点（Batch 2）。
- 本轮未新增 `相关：`（无跨域必补缺口）；`engine-design` ↔ lifecycle/retrieval 已由契约展开覆盖。

## 5. 阻断声明

| 阈值 | 命中？ | 说明 |
|---|---|---|
| B1 arch confirmed | **无** | arch=0 |
| B2 core 结论 B | **无** | 9 节点均为结论 A |
| B3 A6 唯一安全边界绕过 | **无** | ISSUE-010 不越界 |
| B4 同 core 节点 product drift+bug ≥3 | **无** | 核心 product open：仅 ISSUE-010（1） |
| B5 全批 core product drift+bug ≥12 | **无**（本域贡献 1） |

**阻断：无。** 无需立刻上报 GM（B1–B5）。收尾私信 GM 交付本 artifact 即可。

## 6. kb_check / 测试摘要

- `node .litecode/ops/tools/kb_check.mjs`：全库 errors=**0**，warnings≈80（他域/存量提示；本域 `--node` 各 key errors=0）。
- 复现失败（修前）：`refuses_to_copy_symlinks`、`lexical_lane_source_never_calls_*`、`death_list_forbidden_hot_switch_symbols`、`stop_kills_os_process_when_client_not_installed`。
- 通过：`terminal_bypass_death_list`（3）、`workspace_path_death_list`（4）、`sandbox::tests`（3）、`stop_during_warmup_kills_worker`。

## 7. 交付路径

- Artifact：`/workspace/litecode/.litecode/ops/artifacts/BATCH-3-ide-substrate.md`
- Issues：`/workspace/litecode/.litecode/ops/issues/ISSUE-{002,004,007,009,010}.json`
