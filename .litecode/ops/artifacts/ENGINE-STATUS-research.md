# ENGINE-STATUS 调研：检索引擎进度条（code + session）

分支 ops/team-framework，只读调研，未改产品代码。

## 1. 现状

### 后端
- 接口：`GET /api/workspace/engines/detail`（src/workspace/routes.rs:211,319），每次请求现算 `engines_detail_view`（src/engines/status_view.rs:18）。**纯轮询，没有推送。**
- 数据来源全是磁盘文件（job.json / meta / pending hint），不是内存实时状态：
  - code 进度：`job.json` 里的 `IndexingProgress{phase, files_done, files_total, chunks_done}`（code_search/index_status.rs:38），**没有 chunks_total**。
  - 写入时机：全量构建每攒满一个 embed batch 写一次（code_search/build.rs:73-92 → log_progress:147）。增量刷新只在开始时写 `begin_refreshing`，`files_total=0`（index_status.rs:187-201），中途不再更新。
  - session：**没有任何进度/job 上报**。`session_index_status` 只能给出 Absent/NeedsRebuild/Stale/Ready（session_search/semantic_index.rs:756-770）；session 刷新走 `touch_code_job=false`，不写 job.json（engines/mod.rs:863-895、983-1010）。
- 合并逻辑：
  - `status` = `merge_retrieval_status` 按优先级取两者中更“严重”的一个（status_view.rs:195-222），只是挑一个，不合成进度。
  - `progress` = **只有 code 的** `index_view.progress`（status_view.rs:126）。
  - `work` = `merge_index_work`（engines/mod.rs:194），dirty 是**文件/行数**相加，不是 chunk。
  - `sync_pct`：Building/Refreshing 时用 files_done/files_total（index_status.rs:338-347）；Stale 时用 “每个 pending 扣 2%” 的估算（:349-357），**是猜的**。前端进度条没用它。
  - `indexed_chunks` 只取 code meta（status_view.rs:136）；session 的 chunk 数在 session meta（semantic_index.rs:84 `shell(embedder_id, indexed_chunks)`）里有，但没上报。

### 前端
- 组件：web/src/dockview/panels/settings/engines/EngineView.tsx `RetrievalSection`（:188），进度条在 :343-355，样式 web/src/theme/components/settings.css:404-431。
- fill：`barWidth`（EngineView.tsx:117-133）
  - building/refreshing：`files_done/files_total`；没有 total 时**写死 45%**；
  - absent：0%；
  - **其他一律 100%**（包括 stale、needs_rebuild、failed）。
- 颜色：`indexTone`（:97-115）：ready 绿；stale/busy 橙；needs_rebuild **和 failed 都是红**；absent 或引擎 stopped/unavailable 灰。闪烁 = building/refreshing 时的 is-pulse。
- hover：`title={detailTip}`（:240-256, :349）：“N files · M chunks · X to update”，只有 code 的数，没有百分比，没有 session 行。
- 刷新：EnginesSection.tsx:38-49，只有 `index.status` 是 building/refreshing 或引擎 warming 时才每 1s 轮询，否则只拉一次。

## 2. 不准、不实时的根因
1. **只算了 code**（产品缺陷/架构）：progress 和 indexed_chunks 只取 code；session 没进度源。
2. **分母用文件数**（产品缺陷）：fill 用 files_done/files_total，不是 chunk；大文件 chunk 多，进度会不匀。后端没有 chunks_total。
3. **假 fill**（产品缺陷）：stale/needs_rebuild/failed 显示 100%；进行中没 total 时写死 45%；增量刷新 files_total=0，整个刷新期间都是 45%。
4. **session 刷新不被视为 busy**（常见 bug）：session 重建/刷新不写 job，合并后 status 仍是 needs_rebuild/stale，前端不轮询，用户看到的是红条或满格橙条，一直不动，要重开页面才会更新。
5. **轮询启停依赖上一次快照**（常见 bug）：静止状态只拉一次；后台（agent 调用 code_search/session_search）触发的刷新不会被察觉，也就“不实时”。
6. **粒度粗**（罕见/小问题）：全量构建每个 batch 才写一次磁盘，小仓库的进度会跳。

## 3. 逐条对照预期

| 预期 | 现状 | 结论 | 档位 |
|---|---|---|---|
| fill = code+session 按 chunk 归一 | 只算 code，按文件，静止时 100%/45% | 不符 | 架构设计错误（缺统一的 chunk 进度模型，session 无进度源）+ 产品缺陷 |
| 绿 = 100% 无待处理 | ready 绿；但 session 有 pending 时也会被合并为 stale，这一点对 | 基本相符 | — |
| 橙 = 未满，闪烁 = 在刷 | 橙对；但 stale 时是满格橙；session 在刷不闪 | 部分相符 | 产品缺陷 + 常见 bug |
| 红 = 错误不可用 | failed 和 needs_rebuild 都红；model 缺失/引擎 unavailable 反而灰 | 不符 | 产品缺陷 |
| 空 = 需重建，开始后进橙 | needs_rebuild 是满格红；只有 absent 是空 | 不符 | 产品缺陷 |
| hover 三行 total/code/session | 一行，只有 code 的文件/chunk 数 | 不符 | 产品缺陷 |
| 实时 | 只在 code job 存在时 1s 轮询 | 不符 | 常见 bug |

## 4. 改造方案（最小机制）

**统一模型**：每个语料（code、session）都上报 `{state, chunks_done, chunks_total, error}`，state 只有 `ready | refreshing | needs_rebuild | failed`。total = Σdone / Σtotal。颜色和 fill 都由这一份数据推出来，前后端不再各自猜。

### 后端（中等，约 1.5–2.5 人天）
- code 和 session 都能上报 chunk 的已完成数和总数：完成态用 meta 的 indexed_chunks；进行中要有总数（需要先确定待处理的 chunk 总数再开始 embed，或者在开始前估一个总数，见拍板点 2）。
- session 刷新/重建也要上报进行中状态和进度（和 code 用同一种机制，不能只靠 code 的 job.json）。
- detail 返回按语料拆开的块 + 一个 total；删掉或停用 `sync_pct_from_pending` 这类估算。
- 一个语料失败 → 总体为 failed；model 缺失/引擎不可用也归 failed（红）。
- 验收：
  - 两个语料各自 `chunks_done ≤ chunks_total`，进行中单调递增，完成后 done==total；
  - 只刷 session 时，detail 也显示 refreshing 并且进度在走；
  - 不会出现“进行中但 total=0”超过一个上报周期的情况；
  - 单测覆盖：合并规则（failed > needs_rebuild > refreshing > ready）、Σ归一化。

### 前端（小，约 0.5–1 人天）
- fill = total 百分比，只来自后端，去掉写死的 45%/100%。
- 颜色：ready 且 100% → 绿；refreshing → 橙 + 闪；未满但没在刷 → 橙不闪；failed → 红；needs_rebuild → 空条（灰轨道，fill 0），开始重建后转橙。
- hover 三行：`Total xx%`、`Code xx% · done/total chunks`、`Session xx% · done/total chunks`。
- 刷新：设置页可见时一直轮询（busy 1s、idle 放慢，例如 5s），保证后台触发的刷新也能看到；或者以后接推送。
- 验收：
  - 真机：触发 code 重建、session 重建、两者同时，fill 都连续增长到 100% 后变绿；
  - agent 在后台触发刷新时，不重开页面，进度条 ≤5s 内开始动；
  - needs_rebuild 显示空条，点重建后 ≤1 个周期变橙闪；
  - 删掉/损坏模型显示红；
  - hover 三行的数字和 detail 接口一致。

总工作量约 2–3.5 人天，后端是主体；风险在“进行中拿到 chunk 总数”。

## 5. 需要 Andy 拍板
1. **failed 与 needs_rebuild 的优先级**：一个语料失败、另一个正常时，整条是红（不可用）还是橙（部分可用）？建议红，hover 说明是哪一个。
2. **进行中的 chunk 总数**：(a) 先把待处理文件全部切块算出精确总数再 embed（切块比 embed 便宜很多，但开头会多一段“扫描中”）；(b) 用估算总数，完成时修正。建议 (a)。
3. **增量刷新的分母**：按“本次要处理的 chunk”算（会从 0% 走到 100%），还是按“整个索引”算（会从 98% 到 100%）？Andy 说的 fill 是总体进度，建议按整个索引：total = 已入库 chunk / (已入库 + 待处理)。
4. **未满但没在刷**（例如有 pending 但还没触发刷新）：橙不闪，可以吗？
5. **实时性**：先用持续轮询（简单）还是一次到位做推送（WebSocket 已有 agentWs，但工作量会多 1–2 天）？建议先轮询。
6. **LSP**：这次是否只改检索进度条，不动 LSP 的标签？建议只改检索。
