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
> 合并时间 2026-10-13 ~20:55 HKT。来源：WP-104 done（session）、WP-103 review 通过（ide）。Andy/GM 方向：护栏强制；默认 lean；不测 batch=8；清洗~14% 次要；换模型/云暂不做。

### 6.1 2GB 去向（A1）【实测】
权重 mmap 仅 **≈63–65 MiB**（granite q8/q4）；**主体是 code_search worker 的 anon_heap / ORT arena**（默认峰值 **≈2.5 GiB**）。hash 基线 worker 仅 **≈68–90 MiB** → ORT 相关 ≈2.5 GiB。serve 主进程各配置均 ≈230–295 MiB。

| 配置 | worker 峰值 RSS / HWM | 备注 |
|---|---:|---|
| hash 基线 | ~68 / 88 MiB | 无 ORT |
| ORT 默认 release | ~2549 / 2549 MiB | `rel_default` |
| `CPU_ARENA=0` | 采样峰值 1.6 / HWM ~2.0 GiB | `rel_arena0` |
| `ORT_LEAN=1` | ~1375 / 1835 MiB | `rel_lean` |
| 护栏 LEAN+b32 | 中段 ~1.3 GiB；采样峰值可达 ~1.7 GiB | `lim_lean_b32`，AS=4G 内 |

方法/CSV：`.tmp/wp102/{mem_sample.py,a1_mem.csv}`；详表见 `WP-101-research-ide.md` A1。

### 6.2 慢在哪（A2）【实测】
同语料 3850 chunk、release：

| 标签 | 墙钟 | chunks/s |
|---|---:|---:|
| rel_default | 380 s | **10.13** |
| rel_arena0 | 416 s | 9.25 |
| rel_lean | 495 s | 7.78 |
| rel_b8 (batch=8) | — | **负结论：撑爆 15G 无 swap 整机重启** |
| lim_lean_b32（护栏） | 中段 ≈5.7 chunk/s | 可跑、未触看门狗 |

主导墙钟 = ORT 推理（hash 整库秒级 vs ORT 数分钟）。会话积压外推：WP-104 final ≈1.58M cl100k → granite 复核 **1.545M（比 0.985）** → ≈3450 chunk → 按 10 chunk/s ≈ **6 min 全量**；日增 ~1.3 min（5 天样本不可外推）。Andy「数小时」更可能来自重复全量重建/争用/更大语料（见 §6.5）。

护栏强制：`.tmp/wp102/run_limited.sh`（AS=4GiB、taskset 0–3、ORT_T=4、RSS>3.5GiB 杀、cargo 或 MemAvailable<6GiB 拒测）。**禁止再测无护栏 batch=8**。产品默认 batch 保持 32；小 batch 仅允许在护栏下。

### 6.3 语料构成与可省量（B1-B4）
来源：Session 语料回填（WP-104 done）。样本 `.tmp/wp102/corpus/sessions.db`（25.6 MiB）；产物 `WP-101-research-session.md` + `.tmp/wp102/session/out/corpus.json`。标注：**清洗副本实测；跨度仅 5 天（2026-10-08～10-13），外推不代表长期**。token=cl100k_base；引擎 granite 复核比 **0.985**（`session/out/granite_vs_cl100k.json`）→ B 百分比可近似平移。

| 指标 | 数字 |
|---|---:|
| 会话 / 条 / all token | 29 / 4336 / 1,722,427 |
| final（可嵌）条 / token | 3388 / 1,582,667（占 all 91.9%） |
| 工具输出 token 占比（all） | 959,315（55.7%） |
| 助手推理 token 占比（all） | 462,313（26.8%） |
| 精确重复 final token | 1.9% |
| 近重复 final@0.8 token | 1.7%；抽查精度 1.00 |
| 工具输出截到 2k 超额 | ~182k（~10.6% of all） |
| B3 并集整行上界 | ~433k（25.1% of all） |
| 清洗粗算上界（未扣交集） | **~14%** |
| 日均（5 天） | ~867 条 / ~344k token |
| 每会话 P50 / P90 | ~114k / ~175k |

### 6.4 杠杆成本收益表（A3）
| 杠杆 | 省多少 | 质量代价 | 复杂度 | 证据 |
|---|---|---|---|---|
| 清洗/去重/裁剪 | ~14% token 上界（次要） | 误裁召回未评测 | 中 | B 实测 + granite 0.985 |
| ORT LEAN / arena off | 峰值 2.55→1.4–2.0 GiB | 吞吐 −9%～−23% | 低 | A1/A2 实测 |
| 批/线程 | 默认 b32 稳；无护栏 b8 危险 | 过小 batch 易炸 | 低 | 事故 + limited |
| 增量续嵌（A4） | 避免全量重做「数小时」 | 需实现批 save | 中 | A4 设计 |
| 更小模型 | 公开对比即可 | 可能掉点 | 高 | **本阶段不做** |
| 云嵌入 | ~$0.2–0.5/月/人量级 | 隐私/离线 | 高 | **本阶段不做** |
| 再量化权重 | 对 2GB 峰值几乎无帮助 | — | 低 | 权重已非瓶颈 |

### 6.5 不丢进度与续嵌（A4）
**现状**：`build_full_index` 整库嵌完才 `save()`；中断则已嵌全部丢失（会话 dense 同）。  
**开源对照**：Continue 分批 `markComplete`+内容哈希；Tabby `is_indexed`+批末 `commit`；Qdrant WAL `op_num` 重放；LanceDB 版本化追加。  
**最小设计（不实现）**：chunk 内容哈希为键；每 N 批调用现有 `save()`（tmp+rename）；重启后「扫描 − 已存哈希」走现有 silent 增量刷新；不新增 Warming/状态机。关进程只丢未提交的最后不到一批。

### 6.6 结论
1. 2GB 是 ORT worker arena，不是模型权重。  
2. 默认 ~10 chunk/s；LEAN 换内存、略慢。batch=8 无护栏可拖垮 15G 盒。  
3. 语料清洗约一成多 token，次要。  
4. 「数小时积压」与关进程丢进度，主因更可能是**无续嵌的全量重做**，应用 A4 批提交解决。

### 6.7 推荐组合
1. **落地 A4 批提交续嵌**（最高优先）。  
2. **默认 `LITECODE_ORT_LEAN=1`**（或文档推荐 arena=0）；batch 保持 32。  
3. 真模型压测/CI **强制** `run_limited.sh` 级护栏。  
4. 清洗规则作次要优化（~14%）。

### 6.8 不做清单
- 为省 RAM 换更小模型或上云嵌入。  
- 进一步激进量化权重。  
- 无内存上限的小 batch 压测；复测无护栏 batch=8。  
- 新增引擎状态机 / 新 Warming 语义。

### 6.9 待 Andy/GM 裁定
- 是否立即开实现包做 A4 续嵌 + 默认 LEAN（本包仅调研）。  
- 清洗规则是否进后续实现包（阈值与误裁评测范围）。  
- 空闲卸载（COOL_SECS）是否值得单开对照实测（本包未满测）。

## 7. 合并计划
子包 deliver → 中层审证据（无方法的数字退回）→ 将 A/B 合并进 §6，写 §6.6-6.9 → WP-102 deliver 给 GM。

## 8. 不在本包
任何代码改动/实现；换模型落地；云接入；UI。

## 9. 验收
- [x] WP-103/104 done 或明确 blocked；§6 每节有带方法的数字或带链接的公开数据
- [x] 推荐组合符合 §2 规则；有不做清单
- [x] 未改 src/tests、脚本仅 .tmp；validate_all ok:true；未 commit
