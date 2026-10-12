# WP-057 sync% 契约（后端书面定稿 · 供 WP-059）

生效：2026-10-12。后端 WP-058 / 前端 WP-059。**勿改 `client_protocol`**，除非双方中层另开 Session 窄包。

## 字段

| 名 | 类型 | 含义 |
|---|---|---|
| `sync_pct` | integer 0–100 | 索引同步完成度。100=与当前可搜视图一致（非「库永远完美」）。刷新中可为中间值。 |
| `sync_label` | string（可选） | 短标签，如 `ready` / `syncing` / `building`；给人读，不代替 `sync_pct`。 |

## 落点（本波）

1. **工具结果**：`code_search` / `session_search` 成功或可展示结果时，**正文开头第一行**固定为：
   - `sync_pct: <n>%`（例：`sync_pct: 87%`）
   - 若有 label：同行列 `sync_pct: 87% (syncing)` 亦可；前端以解析 `sync_pct` 数字为主。
2. **数据源**：IDE `IndexingProgress` / `resolve_index_view`（或等价）→ 单一百分比；Tools 只消费、不发明第二套进度。
3. **Warming / 工具不可见**：无结果视图；不发假 100%。
4. **空命中**：仍先出 `sync_pct` 行，再写「没有匹配」类正文（没命中 ≠ 不能搜）。

## 前端 WP-059

- 工具结果视图**开头**渲染该百分比（读首行 `sync_pct` 或结构化字段若后续同形加入 JSON 结果）。
- **本波不改协议文件**；若必须进 `client_protocol`，经后端中层开 Session 子包后再动。

## 非目标

- 不把 sync% 当成「默认开 retrieval」的替代。
- 不在 Warming 时展示可点工具。
