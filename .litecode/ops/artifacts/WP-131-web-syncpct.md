# WP-138 方案（前端-web；父包 WP-133；只改 web/，不 commit，未批不 start）

口径：`WP-057-sync-pct-contract.md` **v2**（权威）+ `WP-131-frontend.md`「GM 拍板：sync_pct 首行契约重定（B）」。与后端 WP-137 同步落地，不做新旧兼容。

## 首行语法（严格）
`sync_pct: <value>[ (<label>)][ chunks <done>/<total>]`
- value ∈ `<0–100 整数>%` | `scanning` | `failed` | `unknown`（全小写）
- label：ASCII `(...)`，内无括号，可含逗号
- chunks：仅 value 为 `<n>%` 时允许
- 段间单空格；行尾无其它内容

## 改动点
1. **`web/src/components/toolviews/syncPct.ts`**
   - 类型改为判别联合：
     ```ts
     type SyncPct =
       | { kind: "pct"; pct: number; label?: string; chunks?: { done: number; total: number } }
       | { kind: "scanning" | "failed" | "unknown"; label?: string };
     ```
   - 严格正则（锚定整行，单空格）：
     `^sync_pct: (?:(\d{1,3})%|(scanning|failed|unknown))(?: \(([^()]*)\))?(?: chunks (\d+)\/(\d+))?$`
     - pct 必须 0–100 整数（`\d{1,3}` 后再判范围，拒绝前导多余如 `101%`）。
     - chunks 只在数字分支接受；非数字 value 带 chunks → 整行不认。
     - 大写 / 多空格 / 尾随内容 / 未知 value → 不认。
   - 不认时：`sync: null`，`rest = raw`（首行原样当正文显示，不编造）——见疑问 Q1。
   - `splitSyncPct(raw)` 签名不变（返回 `{ sync, rest }`），调用方无需改调用方式。
   - 删除任何 v1 专用分支 / 注释；web 中无 `sync_label` 引用（已 rg 确认，无需删）。
2. **`SyncPctHeader.tsx`**
   - `kind:"pct"` → `98%`，有 chunks 再显示 `9800/10000 chunks`；`scanning` / `failed` / `unknown` → 显示该词本身（failed 用红色，其它 muted）；label 照旧显示在后。
   - `data-testid`：`tool-sync-pct`、`-value`、`-label`、`-chunks`；`data-kind` 便于测试。
3. **`CodeSearchToolView.tsx` / `SessionSearchToolView.tsx`**：仅随类型变化做最小跟改（若只透传 `sync` 则无需改代码，只回归测试）。

## 测试（vitest `/exec-daemon/node` v22）
- `syncPct.test.ts`（重写）：
  - 接受：`98% (refreshing, embedding) chunks 9800/10000`、`100% (ready) chunks 5/5`、`0%`、`42% (ready)`、`scanning (refreshing, scanning)`、`scanning`、`failed (model_missing)`、`unknown (needs_rebuild)`、`unknown`。
  - 拒绝（sync=null，rest=raw）：v1 无百分号单数字 `sync_pct: 87`、`101%`、`-1%`、大写 `Scanning` / `FAILED`、`failed chunks 1/2`、`scanning (x) chunks 1/2`、双空格、尾随内容、嵌套括号、`sync_label:` 行。
  - rest：首行剥掉后正文正确；空命中「没有匹配」仍在 rest。
- `SyncPctHeader` / `CodeSearchToolView.test.tsx` / `SessionSearchToolView.test.tsx`：四种 value 渲染（含 label/chunks 组合）；failed 红；无首行不渲染头。
- web 全量 vitest + `tsc -b` 0；lint N/A。

## 不在范围
后端 WP-137、engines 进度条（WP-136 已关）、client_protocol、desktop、commit。

## 疑问（待中层确认，不阻塞提交）
- **Q1 旧格式被拒后 UI 显示**：方案按现行口径「不认 → 不出头部、首行原样留在正文」。是否改为剥掉以 `sync_pct:` 开头的坏首行（不显示）或显示 `unknown`？
- **Q2 v1 与 v2 重叠**：v1 `sync_pct: 87%`、`sync_pct: 87% (syncing)` 在 v2 语法下本身合法（`<n>%` + 可选 label）。"拒绝旧格式"能拒的只有 v1 中 v2 不允许的形状（如无 `%`）。是否接受这一理解？
- **Q3 label 字符集**：契约只说"无括号、可含逗号"。方案接受除 `(` `)` 与换行外任意字符（含空格、下划线），不允许空括号 `()`？请确认空 label 处理（方案：`()` 视为不认）。
- **Q4 chunks 约束**：done ≤ total、total > 0 是否需前端校验？方案：只校验为非负整数，不做 done≤total 校验（以后端为准）。非数字 value 带 chunks 按契约整行不认。
- **Q5 落地顺序**：须与 WP-137 同批。若 web 先合、后端仍出 v1 以外旧形状，首行会按 Q1 处理；需要中层协调同步合入时机。

---
## 中层批复（Q1–Q5）
- Q1：首行不认 → 不显示头部，首行原样留在正文；不显示 unknown，不删行。
- Q2：只拒 v2 不允许的形状（不带 % 的纯数字、大写、越界、未知词等）；v1 中与 v2 语法重合的写法照常接受。
- Q3：label 除括号、换行外都允许；空括号 `()` 不认。
- Q4：chunks 只校验非负整数，不查 done ≤ total。
- Q5：由中层协调与 WP-137 同步落地。

## 实施记录（WP-138；2026-10-14 HKT）
### 改动文件（只改 web/）
- `web/src/components/toolviews/syncPct.ts`：`SyncPct` 改为判别联合（`kind: "pct"` 带 pct/label/chunks；`kind: "scanning"|"failed"|"unknown"` 带 label）；新增 `parseSyncPctLine`，严格整行正则；`splitSyncPct` 签名不变。无 v1 分支。
- `web/src/components/toolviews/SyncPctHeader.tsx`：数字显示 `n%`，有 chunks 再显示 `done/total chunks`；三个词直接显示，failed 红；label 照旧；加 `data-kind`、`tool-sync-pct-chunks`。
- `CodeSearchToolView.tsx` / `SessionSearchToolView.tsx`：只透传 `sync`，无需改代码（回归测试覆盖）。
- 测试：`syncPct.test.ts` → 重写为 `syncPct.test.tsx`（12 条接受、21 条拒绝、rest 处理、头部四种值渲染）；`CodeSearchToolView.test.tsx` 加 1 例（failed、chunks、被拒首行留正文）。原 Session/Code 视图测试中的 `87% (syncing)`、`12%`、`42%` 均为 v2 合法写法，保持不变且通过。

### 证据（/exec-daemon/node v22.14.0）
- 相关：`src/components/toolviews` → 11 files / 100 passed。
- 全量：Test Files 175 passed (175)，Tests 1344 passed (1344)。
- `tsc -b` exit 0。lint N/A。未改后端 / desktop / client_protocol；未 commit。
