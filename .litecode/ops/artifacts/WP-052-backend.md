# WP-052 · 后端汇总（后端中层 / WP-053）

- 基线：分支 `ops/team-framework`，HEAD `5421369` + 未提交工作区。中层未改产品逻辑以外的编排职责；**未 commit**。
- 工具链：`rustc 1.99.0`；直接 `cargo`（无 PATH 前缀）。
- 父：WP-052（产品缺陷级钉稳）。本包 WP-053；子包 WP-054..056 均 **done**。
- 时间：2026-10-12 20:50 HKT。
- 禁区遵守：未碰引擎产品、symlink 产品行为、前端。

## 1. 总览

| 子包 | 域 | Issue | 状态 | 中层抽查 | 结果 |
|---|---|---|---|---|---|
| WP-056 | 上下文 | 032 前置 API | done | `mark_dirty` 仅 `snaps.remove`、留 `order`；`reminder::file_tracker` **5 passed**（Tools 修编译后复跑） | API 就绪 |
| WP-054 | Runtime | ISSUE-032 | done | `restore_changed_files`→`mark_dirty`；exec spine Err + FilesChanged 软回灌；`issue_032_changed_files_restore` **6 passed** | **ISSUE-032 closed** |
| WP-055 | Tools | ISSUE-034 | done | id 循环读 + 有界超时 + dirty→组杀重建；`mcp::` **9/9**；`mcp_integration` **10/10** | **ISSUE-034 closed** |

032 与 034 代码面不交，并行完成；032 依赖 Context `mark_dirty`（WP-056）后由 Runtime 收口。

## 2. ISSUE-032（外部改文件 take_changes 失败不丢）

**根因**：seam 开头 `take_changed_files` 推进快照后，spine/FilesChanged 失败路径未回灌 changed（仅 mailbox 三路）。

**落地**
1. Context：`FileTracker::mark_dirty(paths)` — 删 snap、留 order（非 forget）。
2. Session：`restore_changed_files` 薄转调 → `mark_dirty`（中层授权 Runtime 加一行）。
3. Runtime `exec.rs`：spine `Err` 回灌；FilesChanged 未写入时软回灌（对齐 bash/custom）；SubagentSettled 硬失败早退对称回灌。

**证据**：`tests/issue_032_changed_files_restore.rs` 中层复跑 6 passed；ISSUE-032 status=closed。

## 3. ISSUE-034（MCP stdio 超时 + id 匹配）

**根因**：`send_request` 无界 `read_line`、不校验 response id；取消/超时后 BufReader 残留致错号；pool Mutex 贯穿无界调用。

**落地**
1. `client.rs`：按 id 循环读，跳过 notification；每请求 `timeout`；`DirtyOnDrop` / dirty 拒继续用。
2. `pool.rs`：超时/脏 → `kill_child` 组杀 + 剔除重建（复用 WP-038 生命周期，不改 `proc_lifetime`）。

**证据**：中层复跑 `mcp::` 9 passed、`mcp_integration` 10 passed；ISSUE-034 status=closed。

## 4. 交叉审

| 对 | 结论 |
|---|---|
| Runtime ↔ Context | `mark_dirty` 语义满足 seam 回灌；Runtime 不直改 reminder |
| Tools ↔ IDE | 协议超时触发组杀，不改 WP-038 bind/kill 假设（Tools 产物交叉审节；IDE 未强制回） |

## 5. 门禁自检

- [x] 032/034 均 closed
- [x] 子包 WP-054/055/056 done
- [x] 禁区未碰；未 commit
- [x] 产物：`WP-052-{runtime,tools,context,backend}.md`

## 6. 请 GM 验收

WP-053 完成。请 `close --to done`（或 reject 并说明）。统一 commit 仍由 GM 收口。
