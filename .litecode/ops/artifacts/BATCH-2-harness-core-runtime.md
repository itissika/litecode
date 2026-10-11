# Batch 2 · harness-core（Runtime 切片）目录清单

- 负责人：Runtime
- 基线 HEAD：`075308b`（`git rev-parse --short HEAD`）
- 作用域：`src/runtime`、`src/agent`、`src/types`、`src/authority.rs`、`src/platform_knobs.rs`
- **不占有**：`src/reminder` / `src/session` / `src/context_pipeline`（上下文/Session 域）；subagent 工具入口归 Tools，本切片只挂 runtime 投影与 L0 循环

## 1. 拟新增节点

| key | 路径 | 上层 | A# | 一句话承诺 |
|---|---|---|---|---|
| item-authority | knowledge/contracts/item-authority.md | agent-harness-philosophy | A7 | Responses Item/StreamEvents 是唯一内核原子；禁止平行 Message/ContentBlock 方言 |
| agent-loop | knowledge/contracts/agent-loop.md | turn-lifecycle | A1+A2 | L0 循环冻结：取消>步数>模型停止；先 persist 再执行；只经 AgentDeps 注入 |
| stream-projection | knowledge/contracts/stream-projection.md | turn-lifecycle | A3 | 一流一投影：开行即得 seq；seal 用返回副本；流从不二次 append |
| turn-llm-binding | knowledge/contracts/turn-llm-binding.md | turn-lifecycle | A3+A5 | turn 模型绑定唯一解析；能力硬失败；投影只裁请求副本 |
| platform-knobs | knowledge/contracts/platform-knobs.md | agent-harness-philosophy | A7 | ThinkingSpec/ContextMode 是平台意图；厂商字面量只在目录 tiers 映射 |

均为 `status: pending`，`domain: harness-core`，含符号证据与 `基线：075308b`。

## 2. 拟保留 / 轻改的存量节点

**保留（不改主张）：**
- design/agent-design、harness/agent-harness-philosophy
- framework/session、framework/context
- contracts/{turn-lifecycle,session-model,context-view,subagent-lifecycle}

**本轮轻改（仅补向下链接，意图不变；已 append_log）：**
- `turn-lifecycle`：契约展开 → agent-loop、stream-projection、turn-llm-binding
- `agent-harness-philosophy`：契约展开 → item-authority、platform-knobs

**不新建（避免与他域争主节点）：**
- reminder → 上下文域
- session-model / context-view 正文 → Session/上下文；本切片用 相关/上层 挂接
- subagent-lifecycle → 已有；Tools 持有 hub/turn 入口证据

## 3. 疑似主节点重复

| 对 | 共享证据 | 判定 |
|---|---|---|
| agent-loop ↔ tool-call-lifecycle / tool-result-delivery | `fn run`（src/agent/core.rs） | **非重复主节点**。tools 子契约引用 `run` 证明「不完整 FunctionCall 不执行」；agent-loop 主张的是 L0 控制流与注入边界。保留双方证据；Batch 3 可让 tools 侧改用 `fn function_call_must_not_execute` 减轻启发式噪音 |

kb_check 对本切片无其它 dup；无 filename_mismatch / missing_symbol。

## 4. Issue 草稿（本域，Batch 3 再落盘）

| 草稿 id | grade | layer | core | basis | title | 备注 |
|---|---|---|---|---|---|---|
| （沿用）ISSUE-003 | drift | product | 是（session-model） | A3 | DATA.md 缺 SessionEvent 词汇 | Session 域主责；Runtime 侧 Batch 3 核对 turn/observer 是否依赖该词汇 |
| （沿用）ISSUE-006 | drift | gate | 否 | A7 | SQL 死亡清单误报 dense_parity 文件级测试模块 | 非本切片源码；知晓即可 |
| DRAFT-HC-R1 | suspect | gate | 否 | A7 | platform_knobs 与 provider_catalog.tiers 映射是否仍有产品路径写死厂商 thinking 字符串 | Batch 3 用 death_list + rg 扫 `src/`（豁免 codec/catalog）核实 |

无 arch；不触达 B1–B5。

## 5. kb_check（本域）

- 命令：`node .litecode/ops/tools/kb_check.mjs`（全库）
- 退出码：**0**；errors=0
- 本切片节点：5 pending 文件均过解析；仅 agent-loop 的 dup_primary_suspect（见上）
- 父节点 inactive_target（enabled→pending）属预期警告

## 6. 节点计数（本切片新增）

harness-core 新增 pending：**5**（item-authority, agent-loop, stream-projection, turn-llm-binding, platform-knobs）
