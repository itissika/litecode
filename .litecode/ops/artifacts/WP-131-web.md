# WP-136 方案（前端-web；父包 WP-133；只改 web/，不 commit，未批不 start）

口径：WP-131-frontend.md「GM 拍板」「接口键名定稿」+ WP-131-backend.md 接口。只一种进度模型：只读 `retrieval.{total,code,session}`，不读 `progress` / `sync_pct` / `indexed_chunks`，不做兼容。

## 改动点
1. **`web/src/api/workspace.ts`（类型，必要附带）**：`RetrievalEngineDetail` 加
   `total` / `code` / `session: RetrievalCorpusStatus`，其中
   `{ state: "ready"|"refreshing"|"needs_rebuild"|"failed"; phase: null|"scanning"|"embedding"; chunks_done: number; chunks_total: number|null; chunks_pending: number; error: null|{ engine: "code"|"session"|"model"; reason: "model_missing"|"engine_unavailable"|"engine_stopped"|"index_failed"; message: string } }`。
   旧 `index.progress` / `index.indexed_chunks` 等字段：前端不再读；类型是否删除见疑问 Q2。
2. **`EngineView.tsx`**
   - 删除 `barWidth`（含写死 45%、静止 100%）与旧 `indexTone`，换成纯函数 `retrievalBar(retrieval, rebuildRequestedAt?)` → `{ tone: "ok"|"pending"|"busy"|"failed"|"empty", pulse, pct: number|null, indeterminate }`：
     - 任一 failed（total.state=failed）→ 红（红优先）。
     - needs_rebuild → 空条（fill 0，不闪）；点「重建」后本地标记 `rebuildRequested`，在下一个轮询周期内显示橙+闪，直到后端返回 refreshing / ready / failed 后清除。
     - refreshing + embedding → 橙闪，pct=done/total。
     - refreshing + scanning（chunks_total=null）→ 橙闪 + 不确定条（CSS 动画，不写百分比、不显示 0%）。
     - ready 且 pending>0 → 橙不闪，pct=done/total（增量约 98%）。
     - ready 且 pending=0 → 绿，pct=done/total（=100%）。
     - pct 一律 `chunks_done/chunks_total`（整个索引）；chunks_total=null 或 0 → null（不编）。
   - `detailTip` 改为三行：`Total xx%` / `Code xx% · done/total chunks` / `Session xx% · done/total chunks`；某侧 scanning → 该行写 `scanning`（不写假 %）。failed 时再加一行 `Failed: <engine> · <reason 人话>`（model_missing→模型缺失 / engine_unavailable→引擎不可用 / engine_stopped→引擎已停止 / index_failed→索引失败）+ message。
   - 条 UI：保留 `engine-index-fill`，加 tone class / `data-tone` / `data-indeterminate`，`title` 用新三行 tip。
   - Warming 仍走现有引擎级显示，不进四态；LSP 标签不动。
3. **`EnginesSection.tsx`（轮询）**：Settings 可见时持续轮询——任一 total.state=refreshing 或引擎 warming 或 rebuildRequested → 1s；否则约 5s。用 `document.visibilityState` + 组件挂载判断可见；不可见停轮询，回到可见立即拉一次。无推送。
4. **`web/src/theme/components/settings.css`**：新增 tone 颜色（绿/橙/红/空）、`.is-pulse` 复用、`.is-indeterminate` 来回滑动动画；去掉依赖写死宽度的样式。

## 测试（vitest /exec-daemon/node v22；EngineView.test.tsx / EnginesSection.test.tsx + retrievalBar 纯函数单测）
1. 四色各一例：failed 红；needs_rebuild 空条；refreshing 橙闪；ready+pending=0 绿 100%。
2. ready+pending>0 → 橙不闪。
3. scanning（chunks_total=null）→ 不确定态，无 `%`、无 0%。
4. 增量：done=980,total=1000 → 98%。
5. 红优先：code failed + session ready → 红；hover 写明 engine 与 reason（三种 reason 各断言人话）。
6. hover 三行文本格式与数字。
7. needs_rebuild 点重建 → 下一周期内橙闪（fake timers）。
8. 轮询间隔：refreshing → 1s；空闲 → 5s；不可见停止。
9. 不读旧字段：fixture 只给旧字段 / 给矛盾旧值时，显示完全由新字段决定（如旧 progress=45、sync_pct=100 不影响）。
另：`tsc -b` 0；lint N/A。

## desktop
内嵌 web，不改；静态核对。

## 不在范围
后端（WP-132）、LSP、推送、工具结果 sync_pct 头（WP-062 已有，按输出文本解析，不读 detail）、commit。

## 疑问（待中层确认，不阻塞方案）
- Q1 新块挂载位置：合同写 `retrieval: {total, code, session}`，但现 `retrieval` 已有引擎级 `desired/state/usable/error/index`。方案按「total/code/session 作为 `retrieval` 下新增的三个键，与引擎级字段并存」实现；若是替换整个 retrieval 对象请告知。
- Q2 类型清理：合同说旧字段后端在封包前删除。前端本包是否同时从 `api/workspace.ts` 类型里删掉 `progress/indexed_chunks/sync_pct`（推荐删，可让 tsc 兜底"不读旧字段"），还是等后端删除后再删？
- Q3 改动面多一个文件 `api/workspace.ts`（类型），派工范围里未列，属必要附带。
- Q4 「点重建后一个周期内变橙并闪」：后端若一周期内仍返回 needs_rebuild，前端本地乐观显示橙闪；乐观态超时（如 3 个周期仍未变）是否回落空条？方案默认回落并保持可再点。

## 中层批复（Q1–Q4）

- **Q1**：三块是 `retrieval` 下新增键（`total` / `code` / `session`），与现有引擎级字段（`desired` / `state` / `usable` / `error` / `index`）**并存**，不替换整个 `retrieval`。（后端中层确认中；若其另有定论，中层会在落地前转达。）
- **Q2**：本包从前端类型里删除旧字段 `progress` / `sync_pct` / `indexed_chunks`，让 `tsc` 兜底残留读取。只支持一种格式，不做双读。
- **Q3**：准许改 `web/src/api/workspace.ts` 补类型。
- **Q4**：**不做乐观橙**。点重建后按 1s 轮询，条颜色/填充严格跟后端返回；若连续 3 个周期仍报 `needs_rebuild`，继续显示空条并允许再点。重建请求在途时按钮可 pending/disabled，但条颜色只跟后端态，不本地假变橙闪。据此：方案正文里「点重建后本地 `rebuildRequested` 显示橙+闪」与测试第 7 条改为跟后端态；`rebuildRequested` 仅可用于把轮询收紧到 1s，不得改 bar tone。


---
## 实施记录（WP-136；2026-10-14 HKT）
按方案 + 中层批复 Q1–Q4 落地。只改 web/，未 commit。

### 改动文件
- `web/src/api/workspace.ts`：新增 `RetrievalCorpusStatus` / `RetrievalCorpusError` / `RetrievalCorpusState`；`RetrievalEngineDetail` 加必填 `total` / `code` / `session`（Q1：与引擎级 `desired/state/usable/error/index` 并存）。**Q2**：删除 `IndexingProgress`、`IndexPhase`、`index.progress`、`index.indexed_chunks`（类型不再声明）。
- `web/src/dockview/panels/settings/engines/retrievalBar.ts`（新）：纯函数 `retrievalBar`（tone / pulse / pct / indeterminate）、`retrievalTip`（三行 + 失败行）、`corpusPct`、`reasonText`、`retrievalBusy`。
- `EngineView.tsx`：删除 `resolveIndexStatus` / `indexTone` / `barWidth`（含写死 45%、静止 100%）及旧 stats/workTip；进度条与 hover 只用 `retrievalBar` / `retrievalTip`；加 `data-tone` / `data-indeterminate` / testid。**Q4**：点刷新/重建只调 `onRefreshRequested`（收紧轮询），条颜色不本地变；请求在途时按钮 disabled（沿用 `busy`）。
- `EnginesSection.tsx`：Settings 挂载且页面可见时持续轮询（`setTimeout` 链）：refreshing / warming / 点击后 3 个周期 → 1s，否则 5s；页面隐藏停、回到可见立即拉一次；每次拉取结束都重新排期（即使返回值相同）。导出 `POLL_FAST_MS` / `POLL_IDLE_MS` / `FAST_CYCLES_AFTER_REFRESH`。
- `web/src/theme/components/settings.css`：tone 改为 ok 绿 / pending、busy 橙 / failed 红 / empty 透明；新增 `.is-indeterminate` 滑动动画；删除旧 ready/stale/rebuild/idle tone。
- 测试：新增 `engines/retrievalBar.wp136.test.tsx`（12 例）；`EngineView.test.tsx`、`EnginesSection.test.tsx`、`stores/engineStore.test.ts`、`api/workspace.test.ts` 的 fixture 去旧字段、补 `total/code/session`。

### 测试覆盖（retrievalBar.wp136.test.tsx）
1a–1d 四色（failed 红 / needs_rebuild 空 0% / refreshing+embedding 橙闪 25% / ready 绿 100%）；2 ready+pending>0 橙不闪；3 scanning 不确定条、无 %、hover `Total scanning`；4 增量 980/1000 → 98%；5 红优先（code failed + total ready 仍红）+ hover 写明 engine 与三种 reason 中文；6 hover 三行精确文本；7 点重建本身不变色、连续 needs_rebuild 保持空条且可再点、后端返回 refreshing 后才橙闪；8 轮询：空闲 5s、refreshing 1s、点击后 3×1s 再回 5s、页面隐藏停止；9 夹带旧字段（progress 45、indexed_chunks 999、sync_pct 100）不影响显示。

### rg 核对（Q2）
web/src 非测试代码中 `sync_pct` / `indexed_chunks` / `IndexingProgress` / `index.progress` 无声明无引用。仅剩 `components/toolviews/syncPct.ts` 等——那是 WP-062 解析**工具输出文本首行** `sync_pct:` 的逻辑，不读 engines/detail，不属本包。

### 证据（/exec-daemon/node v22.14.0）
- 相关：engines 目录 + engineStore + workspace 测试 → 7 files / 48 passed。
- 全量：Test Files 175 passed (175)，Tests 1307 passed (1307)。
- `tsc -b` exit 0；lint N/A。desktop / 后端未改；未 commit。

---
## 静态 e2e 摘要（2026-10-14，中层记入）

见 `.litecode/ops/artifacts/WP-131-frontend.md`「静态 e2e（2026-10-14）」：web 1348/1348 + tsc 0；旧 detail 字段确认已删；sync_pct v2 与后端一致；非阻塞项（model_found/tokenizer_found 可选、dirty_too_large 清理、session.status leftover、chunks_total null 注释）在 WP-142 收尾。WP-142 关闭后 WP-133 可 deliver。
