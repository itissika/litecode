# WP-142 方案（前端-web；父包 WP-133；只改 web/，不 commit，未批不 start）

口径：`WP-131-frontend.md`「接口增补：files_pending」。与后端同批落地，不做新旧兼容。

## 改动点
1. **`web/src/api/workspace.ts`**：`RetrievalCorpusStatus` 加必填 `files_pending: number`（注释：非负整数、实际待处理文件数；total = code + session；不进进度）。total / code / session 共用该类型，三块自动都有。
2. **`web/src/dockview/panels/settings/engines/retrievalBar.ts`**（约 :55，ready 分支）：
   - `if (t.chunks_pending > 0)` → `if (t.files_pending > 0)`：ready + files_pending>0 → 橙、不闪；否则绿。
   - `chunks_pending` 不再参与任何颜色判断；`files_pending` 不参与 pct（pct 仍 = chunks_done / chunks_total）。
   - failed / needs_rebuild / refreshing 分支不变（refreshing 仍橙闪，与 files_pending 无关）。
   - hover 文本不变（见疑问 Q2）。
3. **测试 fixture 补 `files_pending`**（tsc 必填兜底）：`retrievalBar.wp136.test.tsx`（`c()` 默认 `files_pending: 0`）、`EngineView.test.tsx`、`EnginesSection.test.tsx`、`stores/engineStore.test.ts`、`api/workspace.test.ts`。

## 测试
- 新增：ready + `files_pending>0` 且 `chunks_pending=0` → tone `pending`、不闪。
- 新增：ready + `files_pending=0` 且 `chunks_pending>0` → 不橙（tone `ok`）。
- 改：原 wp136 第 2 条「ready + chunks_pending>0 → 橙」改用 files_pending 驱动。
- 新增：refreshing + files_pending>0 → 仍为 busy 橙闪（不被降级成不闪橙）。
- web 全量 vitest（/exec-daemon/node v22）+ `tsc -b` 0；lint N/A。

## 不在范围
后端、hover 文案扩展、轮询逻辑、desktop、commit。

## 疑问（待中层确认，不阻塞提交）
- **Q1 橙与闪烁关系**：方案按优先级 failed > needs_rebuild > refreshing（橙闪）> ready+files_pending>0（橙不闪）> 绿；即 refreshing 时不看 files_pending。是否认可？
- **Q2 hover 是否展示 files_pending**：方案不改 hover（三行仍只写 chunks）。是否需要加如 `· N files pending`？若要，加在哪一行（Total 还是各侧）？
- **Q3 绿色但 <100%**：ready + files_pending=0 而 chunks_pending>0（done<total）时，按新规则为绿，但 pct 可能 <100%。方案照实显示绿色 + 实际 pct，不改成 100%。是否认可，还是绿色时强制满条？
- **Q4 total 求和校验**：前端不校验 total.files_pending == code + session，只读 total。是否认可？

---
## 中层批复（Q1–Q4）
- Q1：优先级 failed > needs_rebuild > refreshing（橙闪，不看 files_pending）> ready 且 files_pending>0（橙不闪）> 绿。
- Q2：hover 不加 files_pending。
- Q3：绿色按实际百分比显示，不强制填满。
- Q4：只读后端 total，不校验 total = code + session。

## 实施记录（WP-142；2026-10-14 HKT）
### 改动文件（只改 web/）
- `web/src/api/workspace.ts`：`RetrievalCorpusStatus` 加必填 `files_pending: number`（total/code/session 共用）。
- `web/src/dockview/panels/settings/engines/retrievalBar.ts`：ready 分支橙色判据 `chunks_pending>0` → `files_pending>0`；其余分支、pct、hover 不变。
- 测试 fixture 补 `files_pending: 0`：`retrievalBar.wp136.test.tsx`、`EngineView.test.tsx`、`EnginesSection.test.tsx`、`stores/engineStore.test.ts`、`api/workspace.test.ts`。
- `retrievalBar.wp136.test.tsx`：原第 2 条改为 files_pending 驱动；新增 4 例——files_pending>0 且 chunks_pending=0 → 橙不闪；files_pending=0 且 chunks_pending>0 → 绿、pct=80（Q3）；refreshing + files_pending>0 → 仍橙闪（Q1）；files_pending 不改百分比与 hover（Q2）。

### 证据（/exec-daemon/node v22.14.0）
- 相关：engines 目录 + engineStore + workspace → 7 files / 52 passed。
- 全量：Test Files 175 passed (175)，Tests 1348 passed (1348)。
- `tsc -b` 0 错误。lint N/A。未改后端 / desktop；未 commit。

### 修正（交叉审小问题；2026-10-14 HKT）
- `web/src/api/workspace.test.ts` 的 engines/detail mock JSON 原来只有 `total`，缺 `code` / `session` 两块；补上两块（同形，均含 `files_pending: 0`）。
- 证据（/exec-daemon/node v22.14.0）：`vitest run src/api/workspace.test.ts` → 6 passed；`tsc -b` exit 0。未重新 deliver，未 commit。

### 追加收尾：删旧引擎字段（中层派；2026-10-14 HKT）
- `web/src/api/workspace.ts`：删 `IndexStatus` 类型（删后无其他引用）、`index.status`、`index.pending_updates`、`index.session.status`、`index.session.pending_updates`。`progress` / `sync_pct` / `indexed_chunks` 在引擎接口类型里已于 WP-136 删除。产品代码无引用（tsc 只报测试 fixture），产品代码未改。
- 测试跟改：
  - `api/workspace.test.ts`：mock `index.status:"ready"` → `exists:true`；断言 `index.status` → `index.exists`。
  - `EnginesSection.test.tsx`：`{...index, status:"absent"}` → `exists:false`；fixture 删 `status:"building"`。
  - `EngineView.test.tsx`、`engineStore.test.ts`、`retrievalBar.wp136.test.tsx` fixture 删 `index.status`。
  - wp136 第 9 例（矛盾旧值不被读）保留：仍用 `Object.assign` 在类型外塞旧键，额外加 `pending_updates: 999`、`session: { status, pending_updates }`。
- rg（web/src，引擎接口路径 api/workspace.ts、engines/、engineStore）：无 `IndexStatus` / `pending_updates` / `indexed_chunks` / `sync_pct` / `index.status`。余下命中：
  - `progress`：`InstallTask.progress`（LSP 安装下载进度）及 EngineView 的 `InstallProgress` —— 与检索进度无关；其余为注释。
  - `sync_pct`：`components/toolviews/syncPct.ts` 及 Code/Session 搜索视图与测试 —— 工具输出首行解析（WP-138），与引擎接口无关，未动。
  - `retrievalBar.wp136.test.tsx`：第 9 例故意在类型外注入的旧键。
- 前端不跨刷新周期保持最大值，维持现状。
- 证据（/exec-daemon/node v22.14.0）：全量 Test Files 175 passed / Tests 1348 passed；`tsc -b` exit 0。未重新 deliver，未 commit。

### 关包前三处类型/注释收尾（中层派；2026-10-14 HKT）
只改 `web/src/api/workspace.ts` 类型与注释，产品代码未动：
1. `RetrievalEngineDetail.model`：`model_found` / `tokenizer_found` 改为可选（探针失败时后端只发 `{ ready: false }`）。
2. `IndexWork` rebuild `reason` 删除 `dirty_too_large`（后端已不发）。
3. `chunks_total` 注释：任一侧在 scanning 时，`total.chunks_total` 都可能为 null，不限于本块 `state` 为 scanning。
证据（/exec-daemon/node v22.14.0）：相关 7 files / 52 passed；`tsc -b` exit 0。未 commit。
