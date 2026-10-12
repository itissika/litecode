# WP-057-web（WP-062）— 工具结果视图 sync%

父包：WP-059（frontend_mid） / 祖包：WP-057  
执行：frontend（前端-web）  
范围：仅 `web/`。不 commit。未碰 desktop / 后端 / `client_protocol` / symlink。

## 契约对照（WP-057-sync-pct-contract）

| 条款 | 落地 |
|---|---|
| 结果正文首行 `sync_pct: <n>%`（可选 `(label)`） | `splitSyncPct` 只认首行；解析数字为主 |
| 缺字段 → 省略，禁止假 100% | `sync: null` 时不渲染头；越界/畸形整段原样保留 |
| 空命中仍先有 sync 行再「没有」 | session/code 视图：头 + empty/body rest |
| 本波不改 client_protocol | 未改 |
| 正式接线跟 WP-061 | **本包仅脚手架**：消费工具 output 文本首行；后端未落行时 UI 省略。测覆盖有/无两种 |

## API（web）

- `splitSyncPct(raw) → { sync: { pct, label? } \| null, rest }` — `web/src/components/toolviews/syncPct.ts`
- `SyncPctHeader({ sync })` — `data-testid="tool-sync-pct"` / `tool-sync-pct-value` / `tool-sync-pct-label`
- `CodeSearchToolView` — 新建；登记 `TOOL_VIEWS` + `OUTPUT_OWNED_BY_VIEW`；`data-testid="code-search-view"` / `code-search-body`
- `SessionSearchToolView` — 剥首行后走既有 `parseSessionSearch(body)`；头在 scope 之上

## 改动文件

| 文件 | 动作 |
|---|---|
| `web/src/components/toolviews/syncPct.ts` | 新增 |
| `web/src/components/toolviews/syncPct.test.ts` | 新增 |
| `web/src/components/toolviews/SyncPctHeader.tsx` | 新增 |
| `web/src/components/toolviews/CodeSearchToolView.tsx` | 新增 |
| `web/src/components/toolviews/CodeSearchToolView.test.tsx` | 新增 |
| `web/src/components/toolviews/SessionSearchToolView.tsx` | 接 Sync 头 + strip |
| `web/src/components/toolviews/SessionSearchToolView.test.tsx` | +3 有/无/空+sync |
| `web/src/components/toolviews/registry.tsx` | 登记 `code_search` |
| `web/src/components/toolviews/paramMeta.ts` | `code_search: { primary: ["query"] }` |

## vitest

命令（`/exec-daemon/node` v22.14.0）：

```
/exec-daemon/node ./node_modules/vitest/vitest.mjs run \
  src/components/toolviews/syncPct.test.ts \
  src/components/toolviews/CodeSearchToolView.test.tsx \
  src/components/toolviews/SessionSearchToolView.test.tsx \
  src/components/toolviews/sessionSearchParse.test.ts
```

结果：**4 files / 29 tests passed**（2026-10-12 HKT）。

## 与 WP-061

WP-061（backend tools 结果头）仍 drafting。本包 **不依赖后端已落行**：缺字段省略；假数据测覆盖有 sync / 无 sync / 空命中+sync。正式端到端接线等 WP-061 可消费后再验一手即可，本包 UI 已就绪。

## 未 deliver

测绿后按中层要求停住，等交叉审再 deliver。
