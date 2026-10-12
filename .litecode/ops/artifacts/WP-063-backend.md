# WP-064 · 后端汇总（后端中层 / WP-063）

- 基线：分支 `ops/team-framework`，HEAD `a72ad6a` + 未提交工作区。中层只拆活/交叉审/汇总；**未写 `src/`**；**未 commit**。
- 父：WP-063（合包）。本包 WP-064；子包 WP-067 IDE、WP-068 Runtime、WP-069 Session 均 **done**。
- 时间：2026-10-12 22:04 HKT。
- 禁区遵守：ISSUE-004 仍 open；未碰引擎产品行为；未擅自 commit；symlink 产品口径按 Andy「保链接」落地（非 Escape）。

## 1. 总览

| 子包 | 域 | 状态 | 中层抽查 | 结果 |
|---|---|---|---|---|
| WP-067 | IDE | done | 删 refuse；lexical readlink → sandbox 门 → dest 重建；顶层/嵌套 preserve + 出界 Escape；`workspace::service` 7 passed；ISSUE-010 closed | 保链接就绪 |
| WP-068 | Runtime | done | `TurnErrorKind` / `TurnCompleted.error_kind`；不 enrich `TurnEndReason`；不落盘；§3.4-A 机械兼容；`project.rs` 零改；T1–T4 定点绿 | 036 emit 就绪 |
| WP-069 | Session | done | 041 白名单 + 文件级 cfg(test) 跳过，假红清；036 `turn_error` 按 `error_kind==CompactionFailed`，废 final_text 文案匹配；相关测绿 | 036/041 闭环 |

串并行：IDE 与 Runtime emit、Session-041 并行；Session-036 consume **等 WP-068 deliver** 后落地（已按序完成）。

## 2. ISSUE

| Issue | 状态 |
|---|---|
| ISSUE-010 | **closed**（copy_path 保链接；sandbox 仍门） |
| ISSUE-036 | **closed**（Runtime emit + Session consume 类型化） |
| ISSUE-041 | **closed**（death_list 假红收紧；引擎产品未改） |
| ISSUE-042 | **open**（Tools：`litecode_workspace.rs:821` 生产路径 `SessionDataReader::open`；本波外，另挂） |
| ISSUE-004 | **open**（optional_lsp；禁区，本波未碰） |

## 3. 交叉审

| 对 | 结论 |
|---|---|
| Runtime ↔ Session | `error_kind: Option<TurnErrorKind>` 内存侧对齐；落盘 `turn/end` 无新字段；consume 不依赖 `final_text` |
| IDE ↔ 其余 | `service.rs` 独立；与 Runtime/Session 无文件竞态 |
| death_list ↔ 引擎 | 仅测门禁收紧；`engines/session_search/**` 产品零改 |

## 4. 门禁自检

- [x] WP-067 / WP-068 / WP-069 done
- [x] ISSUE-010 / 036 / 041 closed
- [x] ISSUE-042 已另挂（Tools）；ISSUE-004 仍 open
- [x] 禁区未碰；未 commit
- [x] 产物：`WP-063-{ide,runtime,session,backend}.md`

## 5. 请 GM 验收

WP-064 完成。请 `close --to done`（或 reject 并说明）。统一 commit 仍由 GM 收口。
