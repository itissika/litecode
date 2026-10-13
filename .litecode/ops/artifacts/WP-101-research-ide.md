# WP-101-research-ide · 向量检索引擎内存/速度/杠杆/续嵌（WP-103）

- owner：ide；父 WP-102 / WP-101；包 WP-103。
- 规则：只读调研；不改 `src/`、`tests/`；脚本/CSV 在 `.tmp/wp102/`；不 commit；不碰 `windows-*.yml`。
- 时间：2026-10-13（HKT）；box 8 vCPU / 16 GiB RAM、**无 swap**；内核与 `nproc`/`free` 实测。
- 证据标签：【实测】【公开】【推断】。方法：脚本路径 + 环境 + 样本量。

## 0. 事故与护栏（必读）

20:32 无护栏压测 `rel_b8`（`LITECODE_ORT_BATCH=8`，3850 chunk）内存冲高，15G 盒无 swap → 20:33 整机重启、`.git` 损坏。仓库已重拉到 `e408fe4`；旧数据在 `/workspace/litecode.partial/.tmp/wp102`。

**此后真模型压测一律走** `.tmp/wp102/run_limited.sh`：
| 护栏 | 值 |
|---|---|
| `prlimit --as=` | 4 GiB（RLIMIT_AS，worker 子进程继承） |
| `taskset` | CPU 0–3（4 核） |
| `LITECODE_ORT_INTRA_THREADS` | 4 |
| RSS 看门狗 | 任一相关进程 VmRSS > 3.5 GiB → SIGKILL |
| 拒测 | 已有 `cargo`/`rustc`，或 `MemAvailable < 6 GiB` |
| 默认旋钮 | `LITECODE_ORT_LEAN=1`；**禁止再测 batch=8** |

原始崩溃前数据：`a1_mem.csv`、`a2_wall.txt`、`a2_progress.txt`（本树与 partial 均有）。

---

## A1 · 2 GB 去向

### 结论（核心）
**【实测】** granite q8/q4 权重 **mmap ≈ 63–65 MiB**（文件 `.onnx` 356 KiB + `.onnx.data` ≈ 62.9 MiB），**不是** 2 GiB 主体。  
**【实测】** 主体在 **code_search worker 的 anon_heap（≈ 2.4–2.5 GiB）**——ORT CPU arena / 激活与临时缓冲，而非索引文件或 serve 主进程。  
**【实测】** `LITECODE_CODE_SEARCH_USE_HASH=1` 时 worker 峰值仅 **≈ 68–90 MiB** → ORT 相关开销 ≈ 真模型峰值 − hash 基线 ≈ **2.5 GiB**。

### 拆分表（release 二进制，语料 = 本仓 src+tests 副本，503 文件 / 3850 chunk）

| 配置 | 进程 | 峰值 RSS | HWM | model_mmap | anon_heap（稳态建索中） | 证据 |
|---|---|---:|---:|---:|---:|---|
| hash 基线 | serve | ~295 MiB | 338 MiB | — | ~215 MiB | a1_mem `hash_baseline` |
| hash 基线 | **worker** | **~68 MiB** | 88 MiB | — | ~18 MiB | 同上 |
| ORT 默认（debug） | worker | **2571 MiB** | 2571 MiB | **~63 MiB** | **~2446 MiB** | `ort` |
| ORT 默认（release） | worker | **2549 MiB** | 2549 MiB | ~63 MiB | ~2446 MiB | `rel_default` |
| `CPU_ARENA=0` | worker | 峰值采样 1.6 GiB / HWM **~2.0 GiB** | 2008 MiB | ~63 MiB | 稳态约 **1.27 GiB** | `rel_arena0` |
| `ORT_LEAN=1` | worker | **1375 MiB** | 1835 MiB | ~61 MiB | ~1273 MiB | `rel_lean` |
| serve（各配置） | serve | ~230–295 MiB | ~300–340 MiB | 无模型 | ~215 MiB | 与 hash 接近 |

方法：`.tmp/wp102/mem_sample.py` 每 1s 读 `/proc/<pid>/status`（VmRSS/VmHWM）+ 每 5s 归类 `/proc/<pid>/smaps`（`model_mmap` / `anon_heap` / `binary` / `other_file`）；树扫描含 worker 子进程。环境：loadavg 约 6–9（与其他 agent 共享盒）；`HOME=.tmp/home`。

**【推断】** 空闲冷却（`LITECODE_EMBEDDER_COOL_SECS`）后是否吐回 arena，本次未在护栏下做满 120s 卸载对照；优先靠 **LEAN + arena off** 压峰值。

---

## A2 · 慢在哪

### 吞吐（同语料 3850 chunk，release，无护栏时代；loadavg 见 `a2_wall.txt`）

| 标签 | 墙钟 | chunks/s | 备注 |
|---|---:|---:|---|
| `rel_default` | 380 s | **10.13** | 默认 batch=32、arena on |
| `rel_arena0` | 416 s | 9.25 | 约 −9% 吞吐，内存明显下降 |
| `rel_lean` | 495 s | 7.78 | LEAN；更省内存、更慢 |
| `rel_b8`（batch=8） | — | — | **负结论：撑爆 15G 盒 → 整机重启**【实测事故】 |

首建日志形态：`[build_full_index] … rate=… chunks/s`（`src/engines/code_search/build.rs`）；进度在 `index/job.json`。

### batch=8 崩溃（负结论，不再复测无护栏）
- 20:32:02 启动 `rel_b8`，`a2_progress` 仅记到 `chunks_done≈224` 即断。
- 【推断】小 batch → 更多次 ORT 调用 / 更差的缓冲复用 + 当时 load≈9、MemAvailable≈9.5 GiB，与默认 2.5 GiB worker 叠加触发 OOM killer / 无 swap 崩机。
- **产品建议：默认保持 batch≥32；禁止在无 AS 上限的环境用小 batch 压真模型。**

### 小阶梯（护栏下 · `run_limited.sh`）
矩阵：`LEAN=1` × `BATCH∈{32,16,4}` × `INTRA_THREADS=4`，AS=4GiB。  
**【实测中段】** `lim_lean_b32`：t≈146s 时 `chunks_done=832`（≈5.7 chunk/s，受 4 核+LEAN 约束低于无护栏 7.8）、worker RSS **≈1.29–1.32 GiB**，未触 3.5GiB 看门狗、未崩机。证明 LEAN+batch32 在 4G 上限内可运行；完整收尾数字见 `.tmp/wp102/a2_wall.txt` 的 `lim_*` 行（交付后后台阶梯可继续写 CSV）。

### 管线分解【推断+实测】
- 切块/扫描：相对嵌入可忽略（首建日志「scanning done」后长时间停在 embedding 相）。
- **ORT 推理**：主导墙钟（hash 基线整库秒级 vs ORT 数分钟）。
- 写索引（usearch/tantivy）：在 `save` 阶段一次落盘；首建过程中内存索引增长，但相对 anon_heap 仍小。

### 会话积压外推【推断×实测】
- 会话 dense：`DENSE_CHUNK_TOKENS=448`（`session_search/semantic_index.rs`）。
- WP-104 final ≈ 1.58M cl100k；引擎 tokenizer 复核 granite **1.545M**（比 0.985×）→ ≈ **3450 chunk**。
- 按 10 chunk/s → 全量约 **6 min**；日增 ~340k token → ~760 chunk ≈ **~1.3 min/日**（样本仅 5 天，B4 不可外推）。  
- Andy「数小时」更可能来自：更大/多工作区、更慢机器、与编译争用、或重复全量重建（见 A4）。

---

## A3 · 杠杆表

清洗数据来自 WP-104（`.litecode/ops/artifacts/WP-101-research-session.md` + `corpus.json`）；**【实测】** 引擎 granite tokenizer 对 final 全文重计：`session/out/granite_vs_cl100k.json`，总比 **0.985**（工具输出 ≈1.00），故 B 侧百分比可近似平移。

| 杠杆 | 省多少 | 质量代价 | 复杂度 | 证据 |
|---|---|---|---|---|
| ①清洗/去重/裁剪 | final 粗算上界 **~14% token**（精确 1.9%+近重 1.7%+工具截断超额 ~10.6%；并集整行 25%）；granite 复核后仍同量级 | 误裁影响召回（未做检索评测） | 中 | B 实测 + IDE tokenizer 实测；**次要杠杆**（相对内存/续嵌） |
| ②更小模型 | 公开：granite-97m-ml-r2 MTEB-ML Retrieval **60.3**，同档最高；30m/e5-small/MiniLM 更小更快但分更低（见模型卡） | 可能掉点；中文/代码需核对 | 高（加载器绑 granite pooling） | 【公开】https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2 （访 2026-10-13）；**不下载实测** |
| ③量化 | 现已 q8/q4 WOQ；权重仅 ~66MB，再压对 **2GB 峰值几乎无帮助** | 已量化底 | 低 | 【实测】mmap；【推断】瓶颈在 arena 非权重 |
| ④云嵌入 | OpenAI `text-embedding-3-small` **$0.02/1M**；Voyage-4-lite **$0.02/1M**；Jina 典型 **$0.02–0.05/1M**。按日 340k → 月 ~10M ≈ **$0.2–0.5/月/人** 量级 | 隐私/离线不可用/延迟 | 高（产品+网络） | 【公开】platform.openai.com/docs/pricing、docs.voyageai.com/docs/pricing、jina.ai（访 2026-10-13）；**不接云** |
| ⑤ORT LEAN / arena off | 峰值 **2.55→1.4–2.0 GiB**（−0.5–1.1 GiB） | 吞吐 −9%～−23% | 低（仅环境变量） | 【实测】`rel_arena0`/`rel_lean` |
| ⑤批/线程 | 默认 batch=32 稳；batch=8 **危险**；线程≤4 护栏下 | 过小 batch 易炸内存 | 低 | 【实测】事故 + limited 阶梯 |
| ⑤截断长度 | `EMBED_MAX_LENGTH=512` 常量；会话已 448 token/chunk | 上下文丢失 | 中（改常量需兼容） | 代码只读 |
| ⑤空闲卸载 / 懒加载 | COOL_SECS 可降闲时 RSS【推断】 | 冷启动延迟 | 低 | 未满测 |
| ⑤增量只嵌新消息 | 避免全量重建 → 省「数小时」级【推断】 | 需 A4 续嵌 | 中 | 见 A4 |

**推荐优先级（对 Andy）**  
1. **续嵌（A4）** — 解决关进程丢进度与假「数小时」积压。  
2. **默认 `LITECODE_ORT_LEAN=1`（或 arena=0）** — 直接砍 ~0.5–1 GiB，权重不是问题。  
3. **清洗作次要**（~14%）。  
4. **不做：为省内存换更小模型 / 上云嵌入**（公开对比足够；现模型同档质量最好且权重已很小）。

---

## A4 · 不丢进度与续嵌

### 现状【只读核实】
- `build_full_index`（`src/engines/code_search/build.rs`）整库嵌入在内存 `CodeSearchIndex`，**仅调用方最后 `save()`**（`store.rs:91` 一次写 usearch + chunks.jsonl + meta）。
- 中断 → `clear_index_job` 或进程死 → **已嵌 chunk 全部丢失**，重启全量重做。
- 会话 dense lane 同样「embed 后 `save` 覆盖」（`session_search/semantic_index.rs`）。
- 与锁定规则一致：静默刷新；仅首次/不兼容重建；Warming 不可检索。

### 开源对照（shallow 于 `.tmp/ref/` / partial，只读）

1. **Continue** `continuedev/continue@5522c6f`  
   - `core/indexing/CodebaseIndexer.ts:54` `filesPerBatch=200`；分批后 `markComplete`（约 :623–636、LanceDbIndex 批末）。  
   - `refreshIndex.ts`：按 `cacheKey` 的 tag_catalog / global_cache → **内容哈希跳过已索引**。  
2. **Tabby** `TabbyML/tabby@21b2904`  
   - `crates/tabby-index/src/indexer.rs`：`commit`(:317)、`is_indexed`(:325)、`has_failed_chunks`(:382)。  
   - `code/index.rs`：按 `source_file_id` 跳过已索引且无失败块；批末 `index.commit()`。  
3. **Qdrant** `qdrant/qdrant@016542a`  
   - `lib/collection/src/shards/local_shard/mod.rs:768` `load_from_wal`：按 `op_num` 重放；**仅 flush 确认后截断 WAL**（注释 :863–864）。  
4. **（补充）LanceDB** `lancedb/lancedb@3837b7a`：`Table::add` / `checkout_latest` 版本化追加，利于批提交后可读快照。

### 最小机制（只设计，不实现）

遵守锁定口径（**不新增 Warming / 不新增状态机**）：

1. **Chunk 内容哈希**（path + 文本或已有 chunk id 规则）作为稳定键。  
2. **每 N 批（如 32–200 chunk）调用现有 `save()`**（或等价原子写：tmp + rename），把已嵌集合写入现有 index/meta。  
3. **启动 / 续跑**：`load` 后把「语料扫描结果 − 已存哈希」当作普通 **增量刷新**（与今日 silent refresh 同路径），而非新生命周期。  
4. 失败块可仿 Tabby `has_failed_chunks` 留在 meta 侧待重试；不必新状态枚举。  
5. 会话 lane 同样：按消息/chunk 哈希跳过，批间 `save`。

这样：关 litecode 只丢「未提交的最后不到一批」；重启接着嵌；Warming 语义不变。

---

## 推荐组合草案 / 不做清单草案（供中层 §6.6–6.8）

**推荐**  
- 落地 A4 批提交续嵌。  
- 默认开启 ORT LEAN（或文档推荐 `CPU_ARENA=0`）。  
- 压测/CI 真模型必须 `run_limited.sh` 级护栏。  
- 清洗规则作为次要优化（~14%）。

**不做（本阶段）**  
- 为省 RAM 换小模型或上云嵌入。  
- 进一步激进量化权重（已非瓶颈）。  
- 无内存上限的小 batch 压测。  
- 新增引擎状态机 / 新 Warming 语义。

---

## 未推改动核对（崩机后）

对 `/workspace/litecode.partial` vs 重拉树 `e408fe4`：`src/lsp/{deps,conn,server,hub,server_map,mod}.rs`、`tests/optional_lsp.rs`、`src/engines/mod.rs`、`code_search_engine.rs`、`tests/code_index_tick_death_list.rs`、`optional_code_search.rs`、`code_search_ipc.rs`、产物 `WP-070-ide-045.md` / `WP-076-ide.md`、`ISSUE-051/052`、board 状态（WP-074/079 **done**，WP-103 **in_progress**）→ **全部 `cmp` 相同，无丢失**。  
`cargo test --lib engines:: lsp::`：工具链重装后缺系统 `c++`（无 root 装 g++），**未能复跑**；因源文件与已封包内容一致，判定 WP-074/079 改动已在树中。

## 原始数据路径
- `.tmp/wp102/a1_mem.csv`、`a2_wall.txt`、`a2_progress.txt`、`run_limited.sh`、`mem_sample.py`
- `.tmp/wp102/session/out/granite_vs_cl100k.json`
- `/workspace/litecode.partial/.tmp/wp102/`（崩溃前完整副本，含 `target-rel` 二进制）
- 对标源码：`/workspace/litecode.partial/.tmp/ref/{continue,tabby,qdrant,lancedb}`

## 清理
交付后删除独立 `target-rel` 大目录、模型副本（若有）、`.tmp/ref` 克隆；**保留脚本与 CSV**。不杀他人进程。
