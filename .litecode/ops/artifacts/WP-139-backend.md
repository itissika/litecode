# WP-140 后端中层方案：code 索引定期扫盘对账 + 印章持久化（隶属 WP-139）

kind=direct（执行由 IDE 子包 WP-139-ide 承担，approver=backend_mid）。依据 /workspace/wp-drafts/scan.md（WP-139）与后端中层只读核实（2026-10-14）。

## 1. 范围
- 定期扫盘为事实来源：固定周期 walk，只 stat，比 mtime+size；有变化才读内容/切块。watcher 仅加速发现。
- watcher 事件、notify 报错（watcher.rs:36-38 现丢弃）、Rescan/溢出（:98-103 现过滤）一律触发同一次全量扫盘。
- mtime+size 印章持久化到磁盘（现仅内存，reconcile.rs:18-19/:69/:73），重启首扫只 stat。
- 冷卸载（reconcile.rs:23 现跳过）：变化不得永久漏掉——卸载期间仍做 stat 扫盘并入队，或重载时强制全量对账（二选一由 IDE 方案定，须测）。
- 刻意不做：内容 hash、事件去重合并、Watchman/变更日志游标；session 引擎不改；不做语言特判。

## 2. 文件边界与并行冲突（git status 2026-10-14 实测）
| 文件 | WP-140 要做 | 当前在改 | 结论 |
|---|---|---|---|
| code_search/reconcile.rs | 扫盘只 stat、读写持久印章、冷卸载 | WP-134 已改（write_code_queue） | 冲突，等 134 |
| code_search/fs_notify.rs | 入队路径 | WP-134 已改 | 冲突，等 134 |
| code_search/mod.rs | mod 声明/runtime 字段/扫盘入口 | WP-134 已改 | 冲突，等 134 |
| code_search/index_status.rs | 30s tick 读新 pending 信号 | WP-134 重写中 | 冲突，等 134 |
| engines/mod.rs (:931/:981) | 定期扫盘调度 | WP-134 + WP-135 已改 | 冲突，等 134/135 |
| engines/progress.rs | 扫盘计入 scanning / chunks_pending（只用不改） | WP-134 新建 | 只读依赖 |
| code_search/meta.rs, store.rs | 印章存储/schema | 未改 | 可独立，但需 mod.rs 接线 |
| code_search_ipc/server.rs (:108/:201) | 启动/Lagged 全量对账入口 | 未改 | 无冲突 |
| workspace/watcher.rs | 报错/Rescan 上报而非丢弃 | 未改 | 无冲突 |
| serve/router.rs (:156-164/:215-224) | tick/Lagged 路由 | 未改 | 无冲突 |
| tools/*, session_search/* | 不碰 | WP-135/137 | 禁止触碰 |

## 3. 顺序（中层定）
- Phase 0（现在）：IDE 只做方案/设计，不改代码；方案须给出对 progress.rs 与新 pending 信号的接线点。
- Phase 1：WP-134 deliver 且我验收（progress 记录 + 替代 pending_hint 的精确信号稳定、Session 交叉审完）后才 start 编码。理由：扫盘结果必须进同一 pending 队列/信号并在 progress 报 scanning，核心文件 reconcile/fs_notify/mod/index_status 正被 134 改。
- 无冲突文件（watcher.rs/router.rs/server.rs/meta.rs/store.rs）也不提前：它们单独改无法独立验收，且印章模块需 mod.rs 接线；提前收益小于合并风险。
- WP-137/135 不受影响；WP-140 不改 tools/ 与 session 分支。

## 4. 硬门
1. 语言通用，不做语言/扩展名特判（沿用既有 scan_policy/filter）。
2. 不新增 Warming 或状态机状态；只用 WP-134 的 CorpusState/CorpusPhase，扫盘期间 phase=Scanning、chunks_total=null。
3. 扫盘发现的待处理计入 chunks_pending（精确，不估算、不写死）。不复活 pending_hint。
4. watcher 报错/Rescan/溢出/Lagged → 全量扫盘对账，不丢弃。
5. 冷卸载后变化不永久漏。
6. 持久印章失效：DOC/INDEX schema bump、印章格式版本变、workspace root 变、索引重建/损坏 → 丢弃印章并全量读；印章写入与索引提交一致（先索引后印章，崩溃时宁可多读不漏读）。
7. 不改 DENSE_CHUNK_TOKENS/CHUNK_TOKENS=448、RRF 常量。

## 5. 验收（来自 WP-139）
- 停机期间增/改/删，重启后补上。
- 关掉/打断 watcher 后运行中改文件，一个扫盘周期内补上。
- watcher 报错或溢出触发对账（测试注入）。
- 未改文件重启不读任何内容（测试证明只 stat）。
- 只 touch 不改内容不重复 embed。
- 冷卸载后变化不永久漏。
- 扫盘待处理计入 chunks_pending，进度条可见；扫盘阶段 total=null。
- 单测与集成测试全绿；deliver 前 rg 证 448/RRF 未动、无 pending_hint 新引用。

## 6. 待定（tier-1，需 GM/Andy 拍板）
- 扫盘周期取值（建议默认 60s，可配置？是否复用 30s tick）。
- 印章存储位置/格式（建议与 code 索引同目录独立文件 stamps.v1，随索引失效）。
- 冷卸载策略：卸载期间继续 stat 扫盘 vs 重载时全量对账。
- 大仓库单次 stat 扫盘耗时上限/是否需要节流。

## WP-140 交付摘要（2026-10-14）
子包 WP-141 done。code 定期扫盘（复用 30s tick）+ stamps.v1 持久化；只 stat；冷卸载/NotWarm 仍扫盘入队不消化；内容未变只刷新印章；watcher 错误/Rescan/溢出 → 与 Lagged 同入口全量对账。router 接线 Session 交叉审通过。Linux 浅克隆冷扫中位 ~3.6s / 热扫 ~3.0s（debug）。index.session.status 漏删已顺手去掉。

## WP-140 v2 交付摘要（2026-10-14）
子包 WP-143（IDE）/ WP-144（Session）done。首建/不兼容重建删掉独立全量路径，改为空正式库 + 全量入队走增量；按批 append 存盘、检查点原子替换，崩溃截回已提交长度；session 同口径；半截索引可搜（refreshing + 真实百分比），不新增状态。扫盘/印章保留。详见 WP-140v2-ide.md、WP-144-session.md。跟进 ISSUE-058（session 日志压缩）、ISSUE-059（BM25 打开失败后全量重建）。
