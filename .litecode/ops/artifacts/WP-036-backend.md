# WP-036 · 后端汇总（后端中层）

- 基线：分支 `ops/team-framework`，HEAD `104fd43` + 未提交工作区。中层未改 `src/`、`tests/`。未 commit。
- 工具链：`rustc 1.99.0`；本机直接 `cargo`（无 PATH 前缀）。共享 target，抽样时无争锁。
- 子包：WP-038 Tools / WP-039 上下文 / WP-040 IDE，均已 done。
- 时间：2026-10-12 19:53 HKT。

## 1. 总览

| 子包 | 域 | 状态 | 关闭/修复的 issue | 测试（域自报） | 交叉审 |
|---|---|---|---|---|---|
| WP-038 | Tools | done | ISSUE-039、ISSUE-040（代码已修；issue 板仍 open，见 §5） | `tools::custom` 23/0；`mcp::` 6/0（含新生命周期测） | IDE↔Tools：I1–I6 全接受；WP-028 X1/X2/X4 由本包闭合 |
| WP-039 | 上下文 | done | ISSUE-035（代码已修；Session 有条件接受；issue 板仍 open，见 §5） | compact 7；context_pipeline 48；death_list 13；stage_a compact_reload 1；failure_calibers 7 | Session：有条件接受（pending-cache 同回合可能重复，非阻断） |
| WP-040 | IDE 基座 | done | ISSUE-037、ISSUE-038 **已 closed** | 探针 s2×2、s1×1；`workspace::service` 5；`file_ops` 8；workspace_api 12；path_consumer 4；process_contract 7 | 本包无新子进程；沿用 WP-028 对 Tools 的 X1/X2/X4（已由 WP-038 闭合） |

## 2. 三项目标

### (1) 统一 custom + MCP 进程生命周期（ISSUE-039 / 040）

**改动（Tools，串行：先 custom 再 mcp）**

- `src/tools/custom.rs`：spawn 后调用 `proc_lifetime::bind_child_to_parent`（只调用、不改实现）；子进程退出后经 `finish_after_child_exit` **先组杀再有界 drain**；Finished 路径 stdin writer 改为有界 join（2s）。
- `src/mcp/client.rs`：unix `process_group(0)` + `bind_child_to_parent`；新增 `kill_process_tree`（unix `kill(-pid)` / Windows `taskkill /T /F`），`kill` / `start_kill` / `Drop` 均走组杀。`pool.rs` 仅确认委托路径。
- **不做**：ISSUE-034（协议无超时/错号）；不合并 custom/MCP 调用面；不改 `proc_lifetime.rs`。

**测试**：`finished_reaps_grandchild_holding_stdout`、`finished_kills_orphaned_grandchild_before_side_effect`；`mcp::client` `kill_reaps_grandchild_process` / `drop_reaps_grandchild_process`。

### (2) 压缩以 Session DB 为唯一真相（ISSUE-035）

**行为（只改 `src/context_pipeline/compact.rs`）**

- `expected_revision` 在 snapshot 时一次捕获，`?` fail-closed（去掉 `unwrap_or(0)`）。
- Compact **落库成功**后若 `working_set_blocking` reload 失败：**不**回滚压缩前内存、**不**发 `Failed`；发 `Succeeded` + `Ok(true)`；应急缓存 = `transcript → WorkingRow::pending` + `tail`。
- persist / LLM / cancel / budget 失败路径仍回滚 + Failed（不变）。`expected_prefix` 保留。

**Session 交叉审（有条件接受）**

- DB authority / Succeeded-not-Failed / revision capture / 零 session diff：同意。
- **pending-cache 有条件同意**：应急行 `log_seq == None` 可作短窗口降级；同回合若 `sync_turn_working` 前进，可能把应急 pending 再 extend，造成同回合重复；`end_turn` 后下一 `begin_turn` 自愈。**非阻断**；建议后续跟进（只标原 pending tail，或 sync 排除应急合成行）。

### (3) workspace 并发覆盖 + 跨盘（ISSUE-037 / 038）

**改动（IDE）**

- ISSUE-037：`file_ops` 新增 `atomic_create` / `atomic_create_bytes`（tmp + hard_link 排他；不可用则 `create_new`）；`create_file` 与 `write_file_bytes(overwrite=false)` 改走该路径，已存在 → `WorkspaceError::AlreadyExists`。
- ISSUE-038：`rename_path` 在 `replace_file` 遇 EXDEV 时回退 `move_via_copy`；失败时仅当目标**原先不存在**才 `cleanup_incomplete_dest`（不误删原目标）。
- 探针 `wp028_probe_s2_create_race` / `wp028_probe_s1_xdev_move` **去掉 `#[ignore]`**，默认绿（s2：300 轮双成功=0；s1：有 `/dev/shm` 时跨盘 overwrite Ok，无双 FS 可 skip）。

## 3. 范围外（已确认未碰）

- ISSUE-032（`take_changes`）、ISSUE-034（MCP 协议层）
- 前端；引擎产品行为；symlink 产品行为（037/038 之外）
- `take_changes` / file tracker；中层未改代码；未 commit

## 4. 中层抽查

### file:line 核对（对照当前工作区，2026-10-12 19:53 HKT）

| 子包 | 主张 | 核对 |
|---|---|---|
| Tools | `custom.rs` spawn 后 `bind_child_to_parent` | **成立**（`:325`） |
| Tools | `finish_after_child_exit` 先 `kill_child_tree` 再有界 drain | **成立**（`:439-448`；`drain_output_bounded` `:463+`） |
| Tools | MCP spawn 组 + bind；`kill_process_tree` | **成立**（`client.rs:35-48`、`:199+`；测 `:240`/`:262`） |
| 上下文 | revision 捕获 `:188-191` | **成立**（`:191` `revision_blocking()?`） |
| 上下文 | Compact mutation `:289-298`；reload 应急 `:313-357` | **成立**（mutation `:289-298`；Err 臂 `:336-356` 不回滚 + Succeeded + Ok(true)） |
| 上下文 | 回归测 `compact_persist_ok_reload_err_keeps_db_truth` | **成立**（`:806`） |
| IDE | `atomic_create` @ `file_ops.rs:31`；`create_file` 走排他 | **成立**（`:31`；`service.rs:281` 调用，fn 起于 `:270`） |
| IDE | EXDEV → `move_via_copy`；失败不误删原目标 | **成立**（`service.rs:354-368`、`:515-522`） |
| IDE | `write_file_bytes(overwrite=false)` → `atomic_create_bytes` | **成立**（产物写「`:416–`」为段落起点；实际 match 在 **`:442`**，同函数内） |
| IDE | 探针无 `#[ignore]` | **成立**（两探针文件无 ignore） |

行号与产物基本一致；唯一标注：IDE 对 `write_file_bytes` 的「`:416–`」偏段落起笔，排他调用在 `:442`。

### 抽样实测（中层，rustc 1.99.0，无 PATH 前缀，共享 target 空闲）

| 过滤 | 结果 |
|---|---|
| `finished_reaps_grandchild` | 1 passed（`finished_reaps_grandchild_holding_stdout`） |
| `kill_reaps_grandchild_process` | 1 passed |
| `compact_persist_ok_reload_err_keeps_db_truth` | 1 passed |
| `atomic_create_fails_if_exists` | 1 passed（`atomic_create_fails_if_exists_without_clobber`） |

四项全绿。未再跑全量 `tools::custom` / 探针集成测（域侧已报绿；本抽样覆盖三目标各至少一条）。

## 5. 供 GM

1. **Issue 板未关全**：037/038 已 closed；**035 / 039 / 040 代码已落地且交叉审过，JSON 仍为 open**。请裁定由 owner 关 issue，或授权中层补 history 后关。
2. **ISSUE-035 残余（非阻断）**：应急 pending 与同回合 `sync_turn_working` 可能重复；跟进即可（Session 交叉审已写清）。
3. **仍排除**：ISSUE-032 / 034、take_changes、前端、引擎产品、symlink 产品行为。
4. **未 commit**：三子包改动均在工作区，等上层收口后再统一提交策略。
