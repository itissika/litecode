# WP-124-research-session · 可对齐金标 query 集（WP-126）

- 角色：Session；包 WP-126（父 WP-125）。只读；未改 src；未 commit。时间 2026-10-13 HKT。
- **相对 WP-123：** 已废除 length-seed；主 gold 必须与 query 词面可核验交集。
- 样本：`.tmp/wp102/corpus/sessions.db`（ro）。白名单同 WP-120/123（稠密三态；稀疏+tool_call；tool_result 不计入体积）。
- 正式集：`.tmp/wp124/out/queries.json`（IDE 不得覆盖本目录）。
- 裁定：Q1 short_user overlap≥1（长度≥2）；Q2 仅主 gold 强制对齐。

## 1. 对齐方法
1. 按 kind_bucket 收集行，正文 leaf_text 分词（汉字 2–8 字串 + 二元片；英文标识 2–40）。
2. 候选按 **内容 sha256** 分层选取（非 body 长度）。
3. 从该行实义词嵌入短 query（≤80 字），使 `overlap_tokens = query_tokens ∩ gold_line_tokens`：
   - 一般 ≥2；`short_user` ≥1。
4. 主 gold 不通过则换行/改写；辅 gold 可选、不强制 overlap。
5. `failure_triage`：gold 可含 `tool_result`，且 `expect_degraded: true`。

## 2. 覆盖
| intent | 条数 | ids |
|---|---:|---|
| short_user | 2 | q01, q02 |
| pure_dialogue | 2 | q03, q04 |
| assistant_thinking | 2 | q05, q06 |
| tool_intent | 2 | q07, q08 |
| failure_triage | 2 | q09, q10（均 expect_degraded） |
| **合计** | **10** | |

全部 10 条主 gold 对齐核验通过。session_id 仅 `sha256[:12]`；无长正文。

## 3. 对齐抽查（≥3；仅关键词，不落 body）
| id | query | overlap_tokens | primary kind |
|---|---|---|---|
| q01 | What about useeventcallback? | useeventcallback | user |
| q03 | 请说明 commentary litecode 的含义 | commentary, litecode | assistant_message |
| q07 | 调用 start_line file_path 在哪 | start_line, file_path | tool_call |

（另：q09 `litecodeterminal citationresolve 报错怎么查` ↔ litecodeterminal, citationresolve；tool_result / degraded。）

## 4. 复现
```
.tmp/wp124/venv/bin/python .tmp/wp124/make_aligned_queries.py \
  .tmp/wp102/corpus/sessions.db .tmp/wp124/out/queries.json
```

## 5. 缺口
- Query 由行内词面自动拼装，可读性一般；未做人工改写润色。
- 跨度仍 5 天；体积数字见 WP-120/123，本包未重算。
- 对齐是词面交集，不是语义标注；IDE 命中率仍属摸底。
