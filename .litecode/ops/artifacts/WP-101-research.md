# WP-101-research · 向量检索引擎开销与体验调研（WP-102 方案 + 发现骨架）

- 角色：backend_mid；父 WP-101（gm，approver=andy）；本包 WP-102（split，approver=gm）。
- 规则：**只读调研**；不改 `src/`、`tests/`；测量脚本只放 `.tmp/wp102/`；不 commit；不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13 19:4x HKT。

## 1. 背景（Andy）
引擎启动后 >2GB；会话语料增长最快、嵌入量大且慢，积压可达数小时；嵌入中途关 litecode 丢整批进度。现模型在同尺寸档质量最好。

## 2. 锁定引擎规则（结论必须遵守）
静默后台刷新；只在首次/不兼容时重建；Warming = 已起但不可检索；Warming 期间检索工具隐藏。机制从简。

## 3. 子包
| 包 | owner | 范围 |
|---|---|---|
| WP-103 | ide（IDE 基座，`src/engines/**`，session_search 在 `src/engines/session_search`） | 主力：A1-A4 |
| WP-104 | session | 小包只读：B1-B4 会话语料构成。理由：语料/transcript 归 Session 域，"清洗/去重/裁剪能省多少"必须先有语料构成数据，IDE 不掌握数据模型 |
顺序：WP-103 与 WP-104 **并行**；WP-104 的 B 结果交 WP-103 用于 A3 清洗杠杆的收益估算（经中层转）。产物：`WP-101-research-ide.md`、`WP-101-research-session.md`。

## 4. 每包问题
### A · WP-103 IDE
- A1 内存：2GB 去向——模型权重、运行时/推理库（ORT/candle 等）、tokenizer、索引（向量+稀疏/词法）、缓存、进程数（code_search_ipc 等）。测法：box 上冷启动→Warm→刷新中三个时点的 RSS/PSS（`/proc/<pid>/smaps_rollup`）、按库映射拆分；空语料 vs 实际语料对照。
- A2 慢：嵌入吞吐（chunks/s、tokens/s）、batch/线程/序列长度影响、CPU 利用率；切块（chunk.rs/trimmer.rs）产出量；一次积压的总 chunk 数与耗时外推。
- A3 杠杆（每项：预期节省、质量代价、复杂度、证据）：①清洗/去重/裁剪（用 B 数据）；②更小模型（MTEB 检索分数对比，附 https://huggingface.co/spaces/mteb/leaderboard 链接与具体模型）；③量化（int8/fp16）；④云嵌入（费用/延迟/隐私/离线不可用）；⑤其他：截断长度、按需/懒加载模型、空闲卸载、批量与线程调参、增量只嵌新消息。
- A4 体验：常驻刷新下如何不丢进度、重启续嵌。要求调研 ≥3 个成熟开源实现并给引用（文件/文档链接），如 Continue（代码库索引 + 内容哈希缓存）、Sourcegraph/Zoekt、LanceDB/Chroma 增量写、Qdrant WAL、Tabby 等；归纳最小机制（建议方向：按 chunk 内容哈希逐批提交 + 已嵌集合持久化，重启从未嵌部分继续；不引入新状态）。
### B · WP-104 Session
- B1 语料构成：按来源（用户/助手/工具输出/子会话 transcript）、类型占比（条数+token）。
- B2 重复/近重复率（精确哈希 + 简单 shingle）。
- B3 可裁剪内容占比：工具大输出、日志、大 diff、base64/图片引用、重复 system/提醒文本。
- B4 增长速率（每日/每会话）。
- 测法：只读 `.litecode/sessions` 与 sqlite（只读打开），脚本 `.tmp/wp102/`；写清样本范围。

## 5. 证据要求
- 实测：每个数字附方法（命令/脚本路径）、环境（box CPU/内存/OS）、样本规模、重复次数。
- 公开：附链接（MTEB leaderboard、模型卡、云厂商定价页、开源实现源码链接）；注明日期。
- 区分"实测/公开/推断"。

## 6. 发现（各节由子包回填，中层合并）
### 6.1 2GB 去向（A1）
### 6.2 慢在哪（A2）
### 6.3 语料构成与可省量（B1-B4）
来源：Session 语料回填（WP-104 done 后补数，不 reopen）。样本 `.tmp/wp102/corpus/sessions.db`（25.6 MiB，Andy 时间裁剪）；产物 `WP-101-research-session.md` + `.tmp/wp102/session/out/corpus.json`。标注：**清洗副本实测；跨度仅 5 天（2026-10-08～10-13），外推不代表长期**。方法见 session 产物；token=cl100k_base。

| 指标 | 数字 |
|---|---:|
| 会话 / 条 / all token | 29 / 4336 / 1,722,427 |
| final（可嵌）条 / token | 3388 / 1,582,667（占 all 91.9%） |
| 工具输出 token 占比（all） | 959,315（55.7%） |
| 助手推理 token 占比（all） | 462,313（26.8%） |
| 主 / 子 token（all） | ~1.23M / ~0.49M |
| 库内 token_estimate vs cl100k | 1,624,851（约 −5.7%） |
| 精确重复 all 条 / token | 20.9% / 3.4%（多为短 control） |
| 精确重复 final token | 1.9% |
| 近重复 final@0.8 token | 1.7%；人工抽查 20/20 TP，精度 1.00 |
| 近重复 all@0.8 token | 5.2% |
| 工具输出截到 2k 超额 | ~182k（~10.6% of all） |
| B3 并集整行上界 | ~433k（25.1% of all） |
| 大 diff / 日志 / base64·blob | 14 条·33k / ~0 / 0 |
| 日均（5 天） | ~867 条 / ~344k token |
| 每会话 P50 / P90 | ~114k / ~175k；`reliable_extrapolation=false` |

给 A3 粗算上界（未扣 B2∩B3 交集，偏乐观）：final 去重约 1.9%+1.7% + 截断超额 ~10.6% ≈ **~14%**。嵌入 tokenizer ≠ cl100k → 归 WP-103 实测。

### 6.4 杠杆成本收益表（A3）
| 杠杆 | 省多少 | 质量代价 | 复杂度 | 证据 |
|---|---|---|---|---|
| 清洗/去重/裁剪（用 B） | 粗算上界 ~14% token（见 §6.3）；并集整行剔除上界 25% | 待 IDE：误裁对召回的影响 | 中（规则+哈希） | B 实测；交集未扣 |
| （其余行等 WP-103） | | | | |

### 6.5 不丢进度与续嵌（A4，含开源对照）
### 6.6 结论
### 6.7 推荐组合
### 6.8 不做清单
### 6.9 待 Andy/GM 裁定

## 7. 合并计划
子包 deliver → 中层审证据（无方法的数字退回）→ 将 A/B 合并进 §6，写 §6.6-6.9 → WP-102 deliver 给 GM。

## 8. 不在本包
任何代码改动/实现；换模型落地；云接入；UI。

## 9. 验收
- [ ] WP-103/104 done 或明确 blocked；§6 每节有带方法的数字或带链接的公开数据
- [ ] 推荐组合符合 §2 规则；有不做清单
- [ ] 未改 src/tests、脚本仅 .tmp；validate_all ok:true；未 commit
