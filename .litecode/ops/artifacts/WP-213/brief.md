# WP-213 run_code 提示词改造 — brief

分支：`feat/tool-contract-ptc`（**不合 main**）。来源：Andy 产品决策（WP-212 之后实测反馈）。**提示词改造包；产品原则已钉死，无开放问题。** 不得触碰 `desktop/build/icon.ico`。补测包仍后置。

## 问题

Code 面描述把运行时契约讲了多遍：`PRELUDE` + 各工具 md + 参数说明 + TS 类型，SDK 快照约 14k token，几乎全是边界 / `reason` 枚举 / 结果头复述。类型、错误文案、结果头已经说清楚的，散文再堆是冗余。真正缺的是「怎么写这段程序」的工作哲学——而 `run_code` 是多 agent 共享面，**不得污染 agent 系统提示词**。

现状对照（`src/tool/code_manifest.rs` 的 `PRELUDE`）：约十几段密散文，把 8k 截断规则、ok/reason 枚举、run 字段、LSP、read 分页、undefined 容忍、结果头格式等全部塞进英文散文；工具 md 亦有同类重复。

## Andy 钉死的原则（必须守）

1. **代码 / 类型 / schema 已经编码的不说。** TS 声明、参数类型、返回形状、`reason` 联合类型在 SDK 里可见的，PRELUDE / 工具 md 不再复述枚举清单。
2. **运行时结果头 / 错误文案已经指引的不说。** `run_code ok|failed`、`program:`、`tool calls:`、截断注记、`[warn]`、ToolError message 等运行时会打出来的，提示词不重复讲解。
3. **补上「怎么干活」的哲学，不是 per-tool 教程。** 内容落在 **Code 面 PRELUDE / `code` 工具描述面**，**不是** agent system prompt，**不是**每个工具写长文。哲学要点（可压缩成几句，不必逐条标题化）：
   - 何时开一段 `run_code`（一轮能串完搜→读→改→验时开；瞄一眼单文件不必硬开大程序）
   - explore → edit → verify 闭环怎么在一个 program 里写
   - 何时 `Promise.all` / `allSettled` 并行，何时 `await` 串行（有依赖才串）
   - `return` 摘要（counts / paths / 关键几行），不要 dump 原文
4. **PRELUDE 压到约 15–25 行，只留不变量。** 砍掉 reason 枚举 dump、三遍重复的契约散文。不变量示例级（由 plan 定稿，brief 只定方向）：摘要优先、预算/截断存在、`ok` 语义一句、libs 只读、未知参数整调用失败、无定时器——每条最多一句，能靠类型/运行时说清的删掉。
5. **工具 md 保持瘦：** 需要时最多 **一个短 example + 3–5 行语义**；能靠类型说清的段落删掉。

## 已定范围

**A. 压缩 `PRELUDE`（`code_manifest.rs`）**  
重写为 ~15–25 行：工作哲学 + 少量不变量。删除与 TS / 结果头 / 错误文案重复的段落（含 reason 枚举罗列、截断算法细节、结果头格式说明、逐工具字段复述等）。

**B. 瘦身工具 md / code 参数描述**  
扫 Code 面挂载的工具说明与 `code` 工具自身 description：删掉与 PRELUDE / 类型 / 运行时重复的契约散文；保留必要的短 example + 极短语义。

**C. 刷 SDK 快照**  
PRELUDE / 工具文案变动后同步 SDK fixture；token 前后对比写进 artifacts（cl100k）。

**D. 验收材料**  
`examples.md`：改造前后 PRELUDE（或整段 code 描述）对照 + token 数；`acceptance.md` 自检；`cargo test -j 2` 绿（文案变更可能碰 snapshot / protected 字符串断言，按需更新，不堆新测）。

## 非目标（硬边界）

- **不改** agent / session **系统提示词**（harness `tools.md`、agent system prompt 等）。本包只动 Code 面组装出来的描述（PRELUDE + 工具 md + code 工具描述）。
- **不写** per-tool 长教程、不按单个 agent 口味定制教法。
- **不改** 运行时行为（截断、ok 语义、kill、rejection warn、shell 等）——上一包已定；本包只改「说什么」。
- 不加定时器、不加 rm/mv、不扩大工具面、不合 main。
- 补测包仍后置。

## 验收

1. PRELUDE ≤ ~25 行（或等价 token 量级明显下降；具体阈值以 plan 钉死，目标相对现状大幅缩），且含工作哲学、不含 reason 枚举 dump。
2. 工具 md 符合「短 example + ≤5 行语义」；无与类型/运行时三遍重复的契约段。
3. **系统提示词未改**（diff 不得碰 agent system prompt / 非 Code 面 harness 教法文件，除非 plan 经 GM 书面扩 scope——默认禁止）。
4. SDK 快照同步；`cargo test -j 2` 绿（跑前空闲内存 ≥2GiB）；可附 `cargo-full.txt`。
5. artifacts 给出 before/after + token 对比。

## 风险提示（供 plan）

- 压缩过度可能丢掉仍有用、且类型/运行时**说不清**的不变量——plan 应用清单列出「留下的每一句」及其不可替代理由。
- 若干 protected / snapshot 断言绑定旧文案，落地时逐条报备改断言原因。
