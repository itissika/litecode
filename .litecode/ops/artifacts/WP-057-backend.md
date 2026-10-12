# WP-057 · 后端汇总（后端中层 / WP-058）

- 基线：分支 `ops/team-framework`，HEAD `843a167` + 未提交工作区。中层只拆活/交叉审/汇总；**未 commit**。
- 工具链：`rustc` via `/home/box/.cargo/bin`；直接 `cargo`。
- 父：WP-057（检索引擎产品口径落地）。本包 WP-058；子包 WP-060 IDE、WP-061 Tools 均 **done**。
- 时间：2026-10-12 21:22 HKT。
- 禁区遵守：未改 desired 默认 false；未碰 symlink 产品行为；未改 `client_protocol` / 前端。

## 1. 总览

| 子包 | 域 | 状态 | 中层抽查 | 结果 |
|---|---|---|---|---|
| WP-060 | IDE | done | DirtyTooLarge 已废；`is_searchable` / Warm+Stale→Ready；`sync_pct_from_view`；聚焦 8 测绿 | 引擎腿就绪 |
| WP-061 | Tools | done | 可见性（desired/Warming）；暖等删除；lexical-only；首行 sync_pct；相关测绿（含 availability retrieval_*） | 工具面闭环 |

并行：engines vs tools 分家；交叉点 `is_searchable` + `sync_pct_from_view`。

## 2. 口径落地对照

| # | 口径 | 证据 |
|---|---|---|
| 1–2 | Warming 桶；Warming⇒不可见不可搜 | IDE Rebuild→Warming；Tools `retrieval_hidden_while_warming_*` |
| 3 | 非暖总能搜 | IDE gate Ready on Stale/Refreshing；Tools `warm_stale_is_searchable_*` |
| 4 | 重建仅首次/不兼容；废 DirtyTooLarge | `decide_index_work_dirty_always_updates` |
| 5 | 静默刷；宁缺毋假 | Refreshing 不挡搜；语义门槛保留 |
| 6 | 结果头 sync% | 契约 `WP-057-sync-pct-contract.md`；Tools `format_sync_pct_line`；已知会前端中层 WP-059 |
| 7 | 不默认开 retrieval | `WorkspaceRetrievalState.desired` 仍默认 false |

## 3. ISSUE

| Issue | 状态 |
|---|---|
| ISSUE-023 | **closed**（隐藏未就绪；不改默认开） |
| ISSUE-024 | **closed**（废暖机长等） |
| ISSUE-025 | **closed**（文案跟可见性 / Settings） |
| ISSUE-026 | **closed**（显式 lexical-only） |

## 4. 交叉审

| 对 | 结论 |
|---|---|
| IDE ↔ Tools | `is_searchable` + `sync_pct_from_view` 对齐；Tools 不改重建策略 |
| 后端 ↔ 前端中层 | sync% 契约书面定稿；本波不改 `client_protocol` |

## 5. 门禁自检

- [x] WP-060 / WP-061 done
- [x] ISSUE-023–026 closed
- [x] sync% 契约已给 WP-059
- [x] 禁区未碰；未 commit
- [x] 产物：`WP-057-{ide,tools,backend,sync-pct-contract}.md`

## 6. 请 GM 验收

WP-058 完成。请 `close --to done`（或 reject 并说明）。统一 commit 仍由 GM 收口。
