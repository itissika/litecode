# WP-131 / WP-132 后端中层方案（backend_mid）

Parent: WP-131（引擎进度条 code+session 合并）· 依据：ENGINE-STATUS-research.md

## 接口（已冻结，已发 frontend_mid）
GET /api/workspace/engines/detail → `retrieval: {total, code, session}`，每项：
```
{ state: ready|refreshing|needs_rebuild|failed,
  phase: null|scanning|embedding,          // 仅 refreshing 时非 null
  chunks_done: number,
  chunks_total: number|null,               // 仅 scanning 时 null；embedding 时 >0
  chunks_pending: number,                  // ready && pending>0 ⇒ 橙色（不闪）
  error: null|{engine: code|session|model,
               reason: model_missing|engine_unavailable|engine_stopped|index_failed,
               message} }
```
total 合并：state 优先级 failed>needs_rebuild>refreshing>ready；done/total/pending 求和；任一 scanning ⇒ total.chunks_total=null；total.error = 第一个 failed 语料的 error。

## GM 拍板
- 先切块拿精确 total（允许 scanning 阶段）。
- 分母=整个索引（增量刷新约 98%→100%，不从 0 算）。
- 引擎不可用/已停止 = failed（红）。
- Warming 留在引擎级，不是第四态。
- 工具 sync% 由新 total 计算。
- 旧字段 progress/sync_pct/indexed_chunks 暂保留（deprecated），WP-131 封包前必须删除。
- 不留写死/估算值：删除 index_status.rs ~349-357 sync_pct_from_pending（2%/pending）。
- 轮询不推送；LSP 不动；通用实现，不按语言特化。

## 拆分
- IDE（code 侧 + 两库合并）：见 WP-131-ide.md
- Session（session 侧上报）：见 WP-131-session.md
- 依赖：IDE 先落共享进度记录类型（或先约定签名），Session 基于它接入。

## done_when
- 每语料 done≤total，运行中单调递增，结束时 done==total
- 只刷 session 也显示 refreshing 且进度在走
- 运行中 total=0 不超过一个上报周期
- 合并优先级 + Σ 单测覆盖
- 所有估算值删除；旧字段封包前删除
- 云端绿

## 补充口径（后端中层，2026-10-14）
- 单调性只在一个刷新周期内保证：从进入 scanning 到回到 ready，chunks_done 只增不减，结束时 chunks_done == chunks_total。
- 跨周期边界允许下降：有删除时，从入队前的 ready（done=旧 indexed）到入队后的 scanning（done=kept，即扣掉被删和重索引的旧块），done 可能下降。
- `files_pending`：每个块都有，非负整数，不为 null；total 是 code 与 session 之和。只用来判断“有没有待处理”，不参与进度计算。chunks_pending 只在拿到精确 chunk 数时有值，否则为 0。

## WP-132 交付摘要（2026-10-14）
子包 WP-134 / WP-135 / WP-137 均已 done。

落地内容：
- 共享 `CorpusProgress` / `retrieval_progress_for_workspace`；code+session 精确 chunk 进度；增量两遍（scanning→embedding）；`files_pending` 三块均有。
- sync_pct 契约 v2：首行只在 tools 一处生成；旧 SyncPct / sync_label / pending_hint 已删。
- engines/detail 旧字段已删：progress、sync_pct、indexed_chunks、index.status、pending_updates（保留 created_at）。
- job/queue 原子写；过期 Running 只在 worker 加载时清；单调只在一个刷新周期内保证。

推送注意：工作树里还有 WP-141（扫盘）半成品，已能编译、测试绿，但未验收。提交 WP-132 相关改动时请不要夹带 WP-141，或等 WP-141 deliver 后再一并推。

前端：WP-138 已关；WP-142（类型/测试清旧字段）与 WP-133 静态 e2e 由 frontend_mid 收尾，不挡本包 deliver。
