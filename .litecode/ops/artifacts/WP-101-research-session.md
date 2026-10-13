# WP-101-research-session · 会话语料构成（WP-104，B1–B4）

- 角色：Session；包 WP-104（父 WP-102）。只读调研；未改 src/tests；未 commit。时间 2026-10-13 HKT。
- 语料裁定：方案 (c)。WP-104 已 deliver；本节是 Andy 清洗副本到位后的**数据回填**（不 reopen）。
- **样本：** `.tmp/wp102/corpus/sessions.db`（25.6 MiB），只读 `mode=ro&immutable=1`。29 会话、4336 条 transcript、约 1.72M cl100k token，日期跨度 2026-10-08～10-13（5 天，外推不可靠）。
- 标注：**「清洗副本实测；时间跨度短，外推不代表长期」**。box 空样本那次（全 0）仍见历史；本节以本库为准。
- 输出：`.tmp/wp102/session/out/corpus.json`（仅聚合，无文本字段）。耗时 122s。

## 0. 环境与重跑
- 环境：box，8 vCPU，15 GiB，Linux 6.12；venv 仅 tiktoken。
- 脚本：`.tmp/wp102/session/{corpus_stats.py,run.sh,selftest.py}`。
- 重跑：
```
/workspace/litecode/.tmp/wp102/session/run.sh /workspace/litecode/.tmp/wp102/corpus/sessions.db /workspace/litecode/.tmp/wp102/session/out/corpus.json
```

## B1 语料构成
- 方法：kind/item_type → 来源；主/子按 `parent_session_id`；两层 all / final（`corpus.rs` Policy::Final：user|assistant|tool_call|tool_result）；token = cl100k_base（同 `estimate.rs:56`），对照列 `token_estimate`。
- 实测（清洗副本）：

| 层 | 条数 | token (cl100k) |
|---|---:|---:|
| all | 4336 | 1,722,427 |
| final（可嵌） | 3388 | 1,582,667（占 all 的 91.9%） |

- all 按来源 token（主+子合计）：工具输出 959,315（55.7%）、助手推理 462,313（26.8%）、工具调用 87,713、控制/其他 90,779、提醒 48,981、助手文本 69,448、用户 3,878。
- 主会话约 1.23M token，子会话约 0.49M（all）。
- `token_estimate` 列合计 1,624,851，与 cl100k 重算偏差约 −5.7%（列偏低；以重算为准）。
- 嵌入模型 tokenizer ≠ cl100k → WP-103。

## B2 重复与近重复
- 方法：精确 sha256（规范化文本）；近重复：≥200 字符、去精确重复后，5-gram + MinHash128 + LSH，候选对算精确 Jaccard，阈值 0.8 / 0.9。
- 实测 all：精确重复 908/4335 条（20.9%），但仅 58,826 token（3.4%）——多为短 control/other（751/908）。
- 实测 final：精确重复 121 条 / 29,770 token（占 final token 1.9%）。
- 近重复 all @0.8：87 条 / 89,262 token（5.2%）；@0.9：53 条 / 74,554 token。
- 近重复 final @0.8：46 条 / 26,917 token（1.7%）；@0.9：18 条 / 14,510 token。
- **人工抽查：** 在 Jaccard≥0.8 的 103 对中按间隔抽 20 对，看前 60 字符（路径已打码，不落盘）。判定 20 TP / 0 FP，**精度 1.00**。注：部分对是 `item/assistant` 与 `turn/end` 同文（控制面复述终稿），算法上仍是近重复；清洗若只对 final 层做，这类会自然消失。

## B3 可裁剪量
- 方法：大工具输出超额（截到 2k/8k）、日志正则、大 diff、base64、重复提醒（≥5 次）；各类单独 + 行并集。
- 实测：
  - 大工具输出 >2k：118 条，超额 181,998 token；>8k：4 条，超额 1,105。
  - 大 diff：14 条 / 32,544 token。
  - 日志：1 条 / 525 token（阈值严，可能漏报）。
  - base64 / data:image / 外置 blob：0。
  - 重复提醒模板：4 个；去重可省 819 token（量小）。
  - **并集：** 129 条 / **432,933 token**（占 all 的 25.1%；注意并集用整行 token，大于各类「超额」之和——大输出整行被计入）。
- 给 A3：若按「截到 2k」口径，工具超额约 182k（占 all ~10.6%）；若按并集整行剔除则上界 ~25%。与 B2 exact+near(final) 合计粗算可省上界约 1.9%+1.7%+10.6% ≈ 14%（未扣交集，偏乐观）。

## B4 增长速率
- 方法：`created_at` 毫秒，UTC+8 日界；每会话 P50/P90。
- 实测：5 天（10-08～10-13），`reliable_extrapolation=false`。
- 日均（5 天）：约 867 条 / 344k token。
- 每会话（有 transcript 的 18 个）：均值 95.7k，P50 114k，P90 175k token。
- 缺口：跨度不足 7 天，不能外推月/年。

## 自测与隐私
- `selftest.py` 合成库 9/9 仍通过（脚本正确性，非语料）。
- 产物与 `corpus.json` 均无消息正文、无会话 id 列表；抽查仅终端短 peek。

## 给 WP-103（A3）
可用输入：final 总 token ≈ 1.58M；精确重复 final ≈ 1.9%；近重复 final@0.8 ≈ 1.7%；工具输出截断到 2k 超额 ≈ 182k（~10.6% of all）。日志阈值偏严、base64 本样本为 0。
