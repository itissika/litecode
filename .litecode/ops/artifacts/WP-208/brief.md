# WP-208 brief — 测试大幅瘦身：默认删除，只守关键模块核心链路

- 类型：implement；owner：backend_mid；approver：gm
- 基线：`feat/tool-contract-ptc` @ `3acfe31`（WP-207 后：全量 2714，lib 2212，tests/ 59 文件）；**不合 main**
- Andy 方向：「2000多条太离谱了，守住的是关键模块的核心链路，不是堆测试量」——**激进删除**。

## 原则
**默认删除。** 一条测试只有满足以下之一才保留：
1. 守护某关键模块**核心链路**上的行为（核心链路清单见下，plan 中按模块细化到具体行为）；
2. 是一个**过去真实 bug 的回归测**（需引用 commit/WP/ISSUE）。

不满足即删，不"以防万一"保留；不用 `#[ignore]` 代替删除。

## 核心链路清单（plan 中逐模块细化）
- run_code 管线 / ptc 运行时与 boundary（序列化、undefined/bigint、预算/取消/超时、栈与内存封口、render 预算）
- 工具契约：read / grep / glob / write / edit / apply_edits / run / wait / kill / lsp（七工具）/ webfetch / websearch
- session 持久化与 compaction
- provider 流式：每个 codec 各自的核心流式路径
- context pipeline 预算（估算、阈值、keep_recent）
- 权限与路径安全（preset、floor、越界/敏感路径）
- 锁与并发（resource_gate、write_lock、run 锁只到 spawn）
- SDK fixture 快照（描述/PRELUDE 唯一文本守卫）

## 删除类别（必须覆盖）
1. **遗留/退役方言"不存在"测试**（名含 legacy/retired/removed/old/dialect/no_longer 等，约 67 条）——方言已无回潮路径即删。
2. **描述/提示词子串断言**（约 60 处）——提示面由 SDK 快照覆盖；快照未覆盖的描述（registry/subagent/knowledge 等）亦删断言，必要时把对应文本纳入快照而非子串测。
3. **death_list 守卫**（types/death_list_gate 13 + tests/*_death_list 18）——仅保留"一行式、确有现役回潮路径"的架构守卫，且**合并到一个文件**；其余删。
4. **参数化重复变体**——同一行为的多条近似测折叠为表驱动单测。
5. **琐碎测**：getter、serde 往返、默认值、Display/Debug、常量自证等。
6. **私有 helper 测**——已被公共路径测覆盖的删。
7. **tests/ 全量审查**：59 个集成测文件逐个判定（含 3 个 *_contract.rs：subagent_child_turn_contract / terminal_pty_contract / workspace_process_contract），与 lib 单测重复者删。

## 目标
- 全套显著下降，**指导线 ≤ ~1000 条**。plan 需给出按区域（tools / engines / session / llm / workspace / tool / config / lsp / runtime / client_protocol / context_pipeline / ptc / terminal / 其他 / tests/）的删前→删后目标；某区高于目标须写理由。

## 验收标准
1. **核心链路覆盖地图**：`artifacts/WP-208/coverage-map.md`，删前/删后对照；每条核心链路删后仍至少 1 条有意义的测试（列测试名）。可选：若可用 `cargo llvm-cov`，对核心模块做行覆盖抽查并附结果。
2. **删除清单**：`artifacts/WP-208/deletions.md`，按类别分组给计数；批量类别不要求逐条说明，但须**按文件**列出删除数量；保留的"真实 bug 回归测"注明引用。
3. 最终 tip 上**全量 `cargo test`（lib + tests/）全绿**，日志入 artifacts；给出删前/删后总数与按区域计数。
4. **无产品代码改动**（只动测试代码与测试 fixture；若删测导致 dead test-helper，一并删除）。
5. **无 `#[ignore]`** 新增。
6. push 实验分支；**不合 main**。

## 约束
- 分支 `feat/tool-contract-ptc`；不合 main。
- 跑 cargo 前确认空闲内存 ≥ 2GiB；建议 `-j 2`。
- SDK fixture 快照测必须保留且绿（它是提示面唯一守卫）。
- 不碰 `desktop/build/icon.ico`；不改 web/。
