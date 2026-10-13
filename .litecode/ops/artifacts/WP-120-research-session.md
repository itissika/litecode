# WP-120-research-session · 白名单切片 + query 集（WP-123）

- 角色：Session；包 WP-123（父 WP-121）。只读；未改 src/tests；未 commit。时间 2026-10-13 HKT。
- **口径（白名单，只要什么）：**
  - **稠密：** `item/user`；`item/assistant` 非 reasoning（助手消息）；`item/assistant` + reasoning（思考）。
  - **稀疏：** 稠密三态 + `item/tool_call`。
  - `tool_result` **不计入**白名单体积；可作 query gold 且必须 `expect_degraded: true`（Q1）。
- 样本：`.tmp/wp102/corpus/sessions.db`（ro）；跨度 2026-10-08～10-13（5 天）。标注：清洗副本实测，外推不代表长期。
- 数据：`.tmp/wp120/out/whitelist_slices.json`、`.tmp/wp120/out/queries.json`。
- 脚本：`.tmp/wp120/whitelist_slices.py`、`make_queries.py`（逻辑自 wp116 复制，未改封存包）。
- Token：cl100k_base。主+子分列（Q3）。产物无长正文、无明文 session_id（gold 用 sha256[:12]）。

## 1. 白名单体积对照

| 层 | 条数 | token | 主 token | 子 token |
|---|---:|---:|---:|---:|
| all | 4336 | 1,722,427 | 1,232,773 | 489,654 |
| final（现状可嵌，含 tool_result） | 3388 | 1,582,667 | 1,115,271 | 467,396 |
| **dense 白名单** | **932** | **535,639** | **445,546** | **90,093** |
| **sparse 白名单** | **2160** | **623,352** | **521,898** | **101,454** |

相对 final 保留：dense **33.84%** token；sparse **39.39%** token。  
sparse − dense = **87,713** token（全部为 tool_call，占 sparse 的 14.07%）。  
现状 final 中 tool_result 约占 **60.61%** token（不进白名单，对照用）。

含空/不含空：仅 1 条空行，占比不变。

**产品话术草案：** search 只取白名单——稠密={user, 助手消息, 助手思考}；稀疏=稠密+tool_call；session 新增 kind 默认不可搜，除非改白名单。

## 2. Query 集（≥8）
路径：`.tmp/wp120/out/queries.json`。seed=123；**n=10**。

| intent | 条数 | 说明 |
|---|---:|---|
| short_user | 2 | q01, q09 |
| pure_dialogue | 2 | q02, q08 |
| assistant_thinking | 1 | q03 |
| tool_intent | 3 | q04, q05, q10 |
| failure_triage | 2 | q06, q07（`expect_degraded: true`，gold 含 tool_result） |

每条含短 query（≤80 字，可含中文产品词）、hashed `session_key`、seq、`kind_bucket`；无长正文、无 >40 字代码/长路径。

## 3. 复现命令
```
.tmp/wp120/venv/bin/python .tmp/wp120/whitelist_slices.py .tmp/wp102/corpus/sessions.db --out .tmp/wp120/out/whitelist_slices.json
.tmp/wp120/venv/bin/python .tmp/wp120/make_queries.py .tmp/wp102/corpus/sessions.db .tmp/wp120/out/queries.json
```

## 4. 缺口
- 跨度 5 天；cl100k ≠ 嵌入 tokenizer（归 IDE）。
- Query 为合成短问 + 分层抽 seq，**未**做人工相关性标注；IDE 小样本命中率仅供摸底。
- 未算 chunk 数（归 WP-122）。
