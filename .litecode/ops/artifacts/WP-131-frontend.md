# WP-133 方案（frontend_mid · direct）

- 父包：WP-131（GM，Andy 已批）。并行包：WP-132（backend_mid · 引擎 detail 合并进度接口）。
- 审批：gm。产物：本文件。子包（批后、接口键名已定稿后再派）：前端-web。
- **GM 未批 plan 不 start；不 commit。只支持一种进度模型，不做新旧字段双读。**

## Goal（只做一项）
Settings → 检索引擎进度条：fill / 颜色 / hover / 轮询 **全部由后端合并状态推出**（code + session），删除写死 45%、静止一律 100% 等估算。

## 现状落点（只读核实，对照 ENGINE-STATUS-research.md）
- **进度条**：`web/src/dockview/panels/settings/engines/EngineView.tsx`
  - `indexTone`（约 :97）— ready 绿；stale/busy 橙；needs_rebuild 与 failed 都红；引擎 stopped/unavailable 灰。
  - `barWidth`（约 :117）— building/refreshing 用 files_done/files_total，无 total 时写死 45%；absent 0%；其余一律 100%。
  - `detailTip`（约 :256）— 一行 code 文件/chunk，无百分比、无 session。
  - 条 UI（约 :346-350）— `title={detailTip}`；`engine-index-fill` + `is-pulse`。
- **轮询**：`web/src/dockview/panels/settings/engines/EnginesSection.tsx`（约 :48）— 仅 busy/warming 时 1s；空闲只拉一次。
- **样式**：`web/src/theme/components/settings.css`（约 :404-431，含 `.engine-index-fill.is-pulse`）。
- **调研全文**：`.litecode/ops/artifacts/ENGINE-STATUS-research.md`。

## 方案（fill 自 `/workspace/wp-drafts/133.md`）
依赖 WP-132 的接口，fill/颜色/hover/轮询全部由后端合并状态推出：
- fill = total 百分比，只来自后端。
- 颜色：ready 且 100% → 绿；在刷 → 橙闪；有待处理未开始刷 → 橙不闪；failed → 红（hover 注明哪个引擎）；needs_rebuild → 空条，开始重建后转橙。
- hover 三行：Total xx%；Code xx% · done/total chunks；Session xx% · done/total chunks。
- 设置页可见时持续轮询：在刷 1s，空闲约 5s；不做推送。LSP 标签不动。

### 颜色映射（与定稿接口对齐）
| 条件 | 呈现 |
|------|------|
| `state=failed` | **红**；hover 写明 `error.engine` + `error.reason`（及 message） |
| `state=needs_rebuild` | **空条**（fill 0）；点重建后 ≤1 个轮询周期变橙闪 |
| `state=refreshing` 且 `phase=embedding` | **橙 + 脉冲**；fill = done/total |
| `state=refreshing` 且 `phase=scanning`（`chunks_total=null`） | **橙 + 脉冲 + 不确定态**（不定百分比，不显示 0% 或假 %） |
| `state=ready` 且 `chunks_pending>0` | **橙，不脉冲** |
| `state=ready` 且 `chunks_pending=0` 且 100% | **绿** |

Hover 三行（数字与 detail 一致；scanning 时该侧可不写假 %，或标 scanning——实现自定，但不得用旧 progress/sync_pct）：
1. `Total xx%`
2. `Code xx% · done/total chunks`
3. `Session xx% · done/total chunks`

## GM 拍板（2026-10-14）
六条开放问题全部定了：
1. **#1 红优先**：一侧 failed、另一侧正常 → 整条红；hover 说明是哪一侧。
2. **#2 先切块拿精确总数**：允许有一段扫描阶段。前端在 scanning 时显示 **不确定态**，不得显示 0% 或假百分比。
3. **#3 按整个索引算进度**：增量刷新从约 98% 爬到 100%（不是按「本次待处理」从 0→100）。
4. **#4 有 pending 但未在刷**：橙，**不闪**。
5. **#5 轮询，不做推送**。
6. **#6 LSP 不动**：只改检索进度条。
7. **补充**：引擎不可用或已停止 → **红**，不显示灰（对接接口 `state=failed` + `error.reason` 为 `engine_unavailable` / `engine_stopped` / `model_missing` 等）。

## 接口键名定稿（WP-132，后端中层 2026-10-14）
GET `/api/workspace/engines/detail` → `retrieval: { total, code, session }`，三块同形：
- `state`: `"ready"` | `"refreshing"` | `"needs_rebuild"` | `"failed"`
- `phase`: `null` | `"scanning"` | `"embedding"`（仅 `state=refreshing` 时非 null）
- `chunks_done`: number
- `chunks_total`: number | null（仅 `phase=scanning` 时为 null；embedding 期间保证 >0）
- `chunks_pending`: number（`state=ready` 且 `chunks_pending>0` → 橙、不闪）
- `error`: `null` | `{ engine: "code"|"session"|"model", reason: "model_missing"|"engine_unavailable"|"engine_stopped"|"index_failed", message: string }`

规则：
- pct = `chunks_done/chunks_total`，分母为整个索引。
- scanning（`refreshing` + `phase=scanning` + `chunks_total=null`）→ 不确定条。
- 不可用/已停止引擎 → `state=failed`，带 `error.engine` + `error.reason`（红）。
- `total.state` 合并优先级：failed > needs_rebuild > refreshing > ready；total 的 done/total/pending 为两侧之和；任一侧 scanning 则 `total.chunks_total=null`；`total.error` = 第一个失败侧的 error。
- 旧字段 `progress` / `sync_pct` / `indexed_chunks` **废弃**，前端不得再读（后端在 WP-133 落地后删除）。
- Warming 仍是引擎级状态，不在上述四态内；仅轮询。

**派工**：前端-web 子包仅在本定稿之后、且 GM 批准 WP-133 plan 并 start 之后创建/开工。

## 验收（= WP-133 done_when）
- 真机：触发 code 重建、session 重建、两者同时，fill 都连续增长到 100% 后变绿；
- agent 后台触发刷新时，不重开页面，进度条 ≤5s 内开始动；
- needs_rebuild 显示空条，点重建后 ≤1 个周期变橙闪；
- 删掉/损坏模型显示红；
- hover 三行的数字与 detail 接口一致；
- 所有估算值和写死值删除（写死 45%、静止一律 100% 等）。

## 不在范围
- WP-132 后端实现 / 旧字段删除时机（后端包）
- LSP 标签与安装轮询
- 推送（WebSocket）
- commit / push；本包未批不 start

## 看板备注
- 产物路径：`.litecode/ops/artifacts/WP-131-frontend.md`（已设）。
- `depends_on`：GM 同意补 `WP-132`；`wp.mjs set` **不支持** `depends_on` 字段（见下方工具错误），未手改 `board.json`，请 GM/工具侧补或批准后另开通道。

## 中层交叉审（WP-136，frontend_mid，2026-10-14）
结论：WP-136 通过，封 done。未 commit。
- 规则逐条核对 retrievalBar.ts / EngineView / EnginesSection：红优先（total 或 code/session 任一 failed）；needs_rebuild 空条 0%；refreshing 橙闪；scanning（chunks_total=null）不确定条、无宽度、hover 写 scanning，无假 %；ready+pending>0 橙不闪；ready+0 绿；pct=floor(done/total) 全索引；hover Total/Code/Session 三行 + failed 行（engine·reason·message）。写死 45%/100%、旧 indexTone/barWidth 已删；LSP 区未动；Warming 仍走引擎级 usable。
- 轮询：1s（total refreshing / retrieval 或 lsp warming / 点刷新后 3 周期），否则 5s；hidden 停、visible 立拉。点刷新只加速轮询，不预设橙（Q4 OK）。
- 轮询「bug 修复」：由 setInterval 改 setTimeout 链后需要 pollTick 在每次 fetch settle（含失败）后重新挂定时器，否则 fetch 失败/detail 未变即断链。改动小、在范围内；风险低（每次 fetch 后 effect 可能触发两次重排定时器，仅重置计时，不会叠加请求）。
- 4 个旧测试仅去旧字段 + 补新三块 fixture；workspace.test 断言由 indexed_chunks 换为 total.chunks_done，未削弱。
- 复跑（node v22.14）：相关 7 文件 48/48；web 全量 175 文件 1307/1307；tsc -b 0。desktop/ 无改动。
- 后端键名静态比对：src/engines/progress.rs CorpusProgress {state,phase,chunks_done,chunks_total,chunks_pending,error}，枚举 snake_case，Option 序列化为 null；status_view.rs 在 retrieval 下并存输出 total/code/session——与前端类型一致。WP-132/134/135/137 仍 in_progress，WP-133 暂不 deliver，待后端落地后做真实静态 e2e。
- ⚠ 风险（不属 WP-136）：WP-137 已改工具首行为 `sync_pct: 98% (refreshing, embedding) chunks a/b` / `sync_pct: scanning (...)` / `failed (...)` / `needs_rebuild` / `unknown`。web toolviews/syncPct.ts 正则要求 `%` 后只有可选括号即行尾，以上新格式全部不匹配 → 首行不被识别、作为正文原样显示（不崩、不编造）。WP-057/062 契约有破坏风险，需后端中层/GM 决定：后端回到兼容格式，或另开前端包放宽解析。

## GM 拍板：sync_pct 首行契约重定（B，2026-10-14）

GM 定 **选项 B**：重定 WP-057 首行契约。不再编造百分比。前端在 WP-133 下开小子包改 `web/src/components/toolviews/syncPct.ts`，与后端 WP-137 **同步落地**，**不做新旧兼容**。

**backend_mid 已确认精确文本（2026-10-14）。** 权威来源：`.litecode/ops/artifacts/WP-057-sync-pct-contract.md`（**v2**）。以下为摘要，以该文件为准。

### 契约正文（已确认）

首行格式：

```
sync_pct: <value>[ (<label>)][ chunks <done>/<total>]
```

规则（与 v2 契约一致）：
- **value**（必填，四选一，**全部小写**）：
  - `<0–100 整数>%`：取自 `CorpusProgress::percent()`，**floor**；运行中最高 **99**，仅 `ready` 且无待处理时为 **100**。
  - `scanning`：total 处于扫描阶段（`chunks_total=null`），无数字。
  - `failed`：`total.state=failed`（模型缺失 / 引擎不可用 / 引擎已停止且 `desired=true` / 索引失败），无数字。
  - `unknown`：拿不到进度；以及工具可见时 `total.state=needs_rebuild`，写为 `sync_pct: unknown (needs_rebuild)`（少见；不兼容重建走 Warming、工具隐藏）。
- **label**（可选）：ASCII 圆括号 `(...)`，无嵌套括号，可含逗号。
- **chunks**（可选）：`chunks <done>/<total>`；**仅当 value 为 `<n>%`（total 确定）时**；`scanning` / `failed` / `unknown` 不带 chunks。
- 段与段之间仅单空格；该行不得有其它内容。
- **替换** WP-057 v1；旧 `sync_label` 与 v1 单数字格式一并废弃；前端解析器**不做新旧兼容**。

示例（摘自 v2）：
- `sync_pct: 98% (refreshing, embedding) chunks 9800/10000`
- `sync_pct: 100% (ready) chunks 5/5`
- `sync_pct: scanning (refreshing, scanning)`
- `sync_pct: failed (model_missing)`
- `sync_pct: unknown (needs_rebuild)`

### 验收
- 前端 + 后端 **各有一个**覆盖四种首行（`N%` / `scanning` / `failed` / `unknown`）的测试。
- 与 WP-137 **同批落地**；无新旧双读/双写。

### 已确认细则（原开放点 → 定稿）
1. **failed**：`total.state=failed`（model missing / engine unavailable / engine stopped with `desired=true` / index failed）。
2. **unknown**：进度不可用；工具可见且 `total.state=needs_rebuild` → `sync_pct: unknown (needs_rebuild)`（少见；不兼容重建为 Warming、工具隐藏）。
3. 四种 value **全部小写**。
4. **percent** = floor(`CorpusProgress::percent()`)；运行中 max 99；100 仅 ready 且无 pending。
5. 旧 `sync_label` 与 v1 单数字格式废弃，无兼容。
6. 正式契约文已由 backend_mid 写入 `.litecode/ops/artifacts/WP-057-sync-pct-contract.md`（v2）。

### 派工
- 子包：**WP-138**（owner=frontend，artifact=`WP-131-web-syncpct.md`），status=`drafting`，approver=`frontend_mid`。
- **backend_mid 已确认** → 可派前端-web fill/submit；本中层本回合 **未 approve / 未 start**（待 web 提交方案后再审）。
- 范围提示：`syncPct.ts` + 必要时 `SyncPctHeader.tsx`（今日 `SyncPctHeader` 只渲染 `pct%` + label，非数字 value 需能展示）；调用方 `CodeSearchToolView` / `SessionSearchToolView` 尽量只靠解析器形状变化；不改引擎进度条（WP-136 已关）。

## 接口增补：files_pending（2026-10-14，后端中层提、前端中层确认）

backend_mid 在 `GET /api/workspace/engines/detail` 的 `retrieval.{total,code,session}` 每个块新增必填字段：

- **`files_pending: number`**：非负整数，表示实际待处理的文件数。`total.files_pending` = `code.files_pending + session.files_pending`。
- 前端**只用** `files_pending > 0` 判断「有没有待处理」（橙色；`ready` + 有待处理 → 橙且不闪）。
- **不算进进度、不用来算百分比**。
- **`chunks_pending` 不再驱动橙色**：该字段仅在拿到精确 chunk 数时才有值，空闲时多半是 0；判断待处理改看 `files_pending`，不要再看 `chunks_pending`。
- 其余字段不变。**不做新旧兼容**。

### 前端落地
- 子包：**WP-142**（owner=`frontend`，artifact=`.litecode/ops/artifacts/WP-131-web-filespending.md`，status=`drafting`，approver=`frontend_mid`）。
- 范围：`web/src/api/workspace.ts` 类型三块补 `files_pending`；`retrievalBar.ts` 橙色判据改为 `files_pending>0`；相关测试（含 `files_pending>0` 且 `chunks_pending=0` 与反例）；全量 vitest + `tsc -b` 0；与后端同批落地。
- 本中层本回合 **未 approve / 未 start**；待前端-web fill/submit 后再审。

## 静态 e2e（2026-10-14）

frontend_mid 对 WP-133 范围做静态联调（未 commit；本回合未 close/deliver WP-142 / WP-133）。

### 结果
- **web**：vitest **1348/1348**（`/exec-daemon/node` v22）；`tsc -b` **0**。
- **backend**（过滤相关）：cargo test **27/27**。
- **`retrieval.index`**：后端仍发送 `exists` / `needs_rebuild` / 等相关字段，**并非**仅 `created_at`（此前口头「只留 created_at」与实发不符；前端类型与之一致即可）。
- **旧字段已删**：`engines/detail` 侧 `progress` / `sync_pct` / `indexed_chunks` / `pending_updates` / 旧 `index.status` 等确认不再出现；web 业务代码不读它们。
- **sync_pct 首行**：后端生产者与 web `syncPct.ts` 解析正则一致（v2 四种 value + 可选 label + 仅百分比可带 chunks）。

### 非阻塞发现（不挡 WP-133 deliver；WP-142 收尾内修或记债）
1. **`model_found` / `tokenizer_found`**：probe 失败时后端可能省略 → web 类型宜标可选（WP-142 待收尾修）。
2. **`dirty_too_large`**：应从 web `IndexWork`（或等价）类型/文案移除（后端已无此路径）。
3. **残留**：后端仍偶发发送 `index.session.status` leftover；前端不依赖；后端可后续清。
4. **`chunks_total=null`**：任一侧 scanning 时 total 亦可为 null——注释/文档对齐即可（行为已正确）。

### 结论
- WP-138 已关；**WP-142 封 done 后即可 deliver WP-133**（静态 e2e 已齐，无阻断缺陷）。
- 通知 backend_mid「前端侧全部完成、可交 WP-132 给 GM」须在 WP-142 close + WP-133 deliver 流程里由中层发出（本回合只写产物）。
