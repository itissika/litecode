# WP-057 sync% 契约（后端书面定稿 · v2 · WP-131 重定）

生效：2026-10-14（GM 拍板方案 B，替代 2026-10-12 v1）。后端 WP-132（Tools WP-137）/ 前端 WP-133 同步落地，**不做新旧兼容**。**勿改 `client_protocol`**。

## 首行语法

`code_search` / `session_search` 工具结果**正文第一行**固定为：

```
sync_pct: <value>[ (<label>)][ chunks <done>/<total>]
```

- `value` 只能取四种之一，**全部小写**：
  - `<n>%`：n 为 0–100 整数，取自 `CorpusProgress::percent()`（向下取整；运行中最高 99，仅 ready 且无待处理时为 100）。
  - `scanning`：total 处于扫描阶段（`chunks_total=null`），无数字。
  - `failed`：total.state=failed（模型缺失 / 引擎不可用 / 引擎已停止且 desired=true / 索引失败），无数字。
  - `unknown`：拿不到进度（进度读取失败），或工具可见时 total.state=needs_rebuild；无数字。
- `label` 可选：英文圆括号，括号内不嵌套括号，可含逗号。例：`(refreshing, embedding)`、`(ready)`、`(model_missing)`、`(needs_rebuild)`。
- `chunks <done>/<total>` 可选：仅当 value 为 `<n>%`（total 确定）时输出；scanning / failed / unknown 不输出。
- 各段之间单个空格；行尾无其它内容。

示例：
```
sync_pct: 98% (refreshing, embedding) chunks 9800/10000
sync_pct: 100% (ready) chunks 5/5
sync_pct: scanning (refreshing, scanning)
sync_pct: failed (model_missing)
sync_pct: unknown (needs_rebuild)
```

## 数据源

IDE `retrieval_progress_for_workspace(...).total`（WP-134 `src/engines/progress.rs`）→ 单一来源；Tools 只消费，不发明第二套进度，不估算。

## 不变的口径

1. Warming：工具隐藏，无结果视图，不出首行。
2. 空命中：仍先出首行，再写「没有匹配」（没命中 ≠ 不能搜）。
3. 不在 Warming 时展示可点工具；sync% 不替代「默认开 retrieval」。

## 验收

前后端各一测试，覆盖 `N%` / `scanning` / `failed` / `unknown` 四种首行；前端解析器按本语法；旧 `sync_label` 与 v1 单数字格式一并删除。
