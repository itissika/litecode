# WP-059 汇总（前端中层）

父包：WP-057 · 子包：WP-062（web）已验收 done  
明细：`WP-057-web.md`  
契约：`WP-057-sync-pct-contract.md`  
范围：仅 `web/` 工具结果视图。未建 desktop（壳复用 web）。未碰后端/`client_protocol`/symlink。未 commit。

## 完成对照（口径 #6）

| 条款 | 落点 | 抽样 |
|------|------|------|
| 结果视图开头固定 sync% | `SyncPctHeader` + `splitSyncPct`；`code_search`/`session_search` 结果区最上 | syncPct.ts；CodeSearchToolView；SessionSearchToolView |
| 首行 `sync_pct: <n>%`（可选 label） | 只认首行；解析 0–100 整数 | syncPct.test.ts |
| 缺字段省略、不假 100% | `sync: null` 不渲头；越界/畸形整段原样 | 同测；SyncPctHeader |
| 空命中仍先有 % 再「没有」 | 头 + empty/body rest | SessionSearch / CodeSearch 测 |
| 本波不改协议 | 未改 client_protocol | 产物声明 |

## 子包结果

- **WP-062**：新建 `CodeSearchToolView` 并登记 `TOOL_VIEWS` + `OUTPUT_OWNED_BY_VIEW`；`SessionSearchToolView` strip 首行后走既有文法；脚手架消费 output 文本首行（跟 WP-061，后端未落行时 UI 省略）。
- 中层复跑：`/exec-daemon/node` vitest → **4 files / 29 passed**。

## 与 WP-061

正式端到端结果头仍待 WP-061 落地后再验一手；前端 UI 已就绪，缺字段省略。

## 非目标（本包未做）

- 不默认开 retrieval；不改 Warming/工具可见性（属 WP-058）。
- 不接桌面独立结果面；不 commit。

## 结论

口径 #6 前端腿已落；请 GM 整包验收。未 commit。
