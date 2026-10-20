# Code 模式工具改造计划（可审计）

> **执行约束（必读，不可忽略）**
>
> - 本文件是本次 code 工具改造的**唯一执行规范**。
> - **不要阅读或引用其他文档**（含 charter、README、ADR、AGENTS.md、CLAUDE.md、历史计划）。它们很可能过时。
> - 冲突时以本文件为准。代码现状只用来定位落地点，不能用来否定本文已定决策。
> - 已定决策（§6）不得擅自改写。D-07 / D-10 / D-11 仍待定：不阻塞 Phase 0/1；遇到必须选边的点停下来报告，不要默认拍板。计划写明「按推荐处理」的，Phase 0 不要提前实现它们。
>
> 状态：执行中 v0.11（D-03/04/05/05a/06/09/14/17/20 已定；本批不做 apply_changes，多文件由 run_code 程序循环）
> 日期：2026-10-20
> 分支：`feat/tool-contract-ptc`
> 范围：`read`、`glob`、`grep`、`edit`、`write`、`run_code`
> 执行已开始。必须按 Phase 0→1→2→3→4→5 顺序；阶段门禁未过不得进入下一阶段。Phase 0–5 均已独立审核通过，产物未提交。不得推倒前序阶段。
> 权威关系：本文件是本次改造的唯一规范。不要去看其他文档。
> 变更规则：已定决策仍只读。允许更新执行约束、§8 阶段状态、§9 变更记录。不得擅自改写已定决策。

## 0. 摘要

- **D-03 已定**：五个工具彻底放弃 markdown 输出；`data` 是唯一真相，且必须结构化、带类型、完全契合 code 形态。
- **D-04 已定**：图片与媒体整体砍掉；`read` 只读文本，遇到图片/二进制返回 `unsupported`；后续需要时另开独立工具，不在本轮。
- **D-05 已定**：fuzzy 整条链砍掉；但 `edit` 保留“部分应用”——一个块失败不连坐其他块，`edit` 的目标是省 token，不是 all-or-none。
- **D-05a 已定**：`edit` 部分成功用 `status:"partial"`。已取代：已被 run-code-remediation-plan 取代（A3）。
- **D-06 已定**：本批不做多文件 `apply_changes`；多文件由 `run_code` 程序循环 `edit`/`write`，不做跨文件回滚。
- **D-20 已定**：`edit` 形态保持 `{path, edits[]}`；单次调用单文件，多文件多块由程序循环。
- **D-09 已定（本轮做）**：删除 per-call `options`；工具内部兜底超时；只保留 `run_code.timeout_ms` 作为整批预算。
- **D-14 已定**：`grep` 不做正则自动修复；非法 regex 硬失败 `invalid_regex`，`details` 带 pattern/regex_error/position/suggested_literal/suggested_pattern；需要字面量用 `literal:true`。其中 `suggested_literal` / `suggested_pattern` 与 `retryable:true` 已取代：已被 run-code-remediation-plan 取代（A1/A4）。
- **D-17 已定**：保留工作区外路径；`FileRef.abs` / `base:"absolute"` 是正式契约。
- **执行已开始**：必须按 Phase 0→1→2→3→4→5 顺序推进；阶段门禁未过不得进入下一阶段。Phase 0–5 均已独立审核通过，产物未提交。本文件仍负责把契约、边界、门禁定清楚；已定决策不改。
- **剩余待确认**：D-07/D-10/D-11 为待定，不阻塞 Phase 0/1；D-06a 撤销（不适用）。D-15/D-16 已按设计裁剪砍掉，可恢复。

## 1. 背景

### 1.1 已上线的形态

- `read/glob/grep/edit/write` 五个工具的 `Tool::surface()` 都是 `Code`，不再出现在 direct 工具列表，direct 调用会被执行器拒绝。
- `run_code` 是唯一传输：每次执行一个全新的 QuickJS runtime，程序通过生成的 `tools.*` SDK 调用上述五个工具。
- 嵌套调用仍然走同一条 `execute_tool_call` 管线（校验、授权、写锁、审计、取消、结果终结化）。
- 工具结果通过 `tool_result_value` 变成 `{ status, data, error }` 交给程序。
- 外层 `run_code` 的 `function_call_output` 文本由“嵌套调用账本 + 程序日志 + 最终返回值”拼成；`data` 只有 `{ logs: string[], result? }`。
- UI 的 `RunCodeToolView` 只渲染程序源码；账本、日志、结果仍走默认文本渲染器。
- session search 会解析账本文本，识别“程序里读过/搜过 session 历史”。

这套已经实测可用，问题是它仍然是 **direct 时代的工具，外面套了一层 code 传输**。

### 1.2 一条链上有三套投影，没有一套是完整的

| 投影 | 谁消费 | 现状 |
|---|---|---|
| `content` + `hint` / `warning_status` / `appendix` | direct 面的模型 | markdown 是事实来源；但五个工具已经不再 direct |
| `data` | code 程序、契约测试 | 手写 schema；只有 code 面校验；很多语义不在里面 |
| ledger 文本 | 外层模型、UI、session search | `transcript_file` 反向解析这段文本 |
| `Outcome.calls` / `LogEntry.level` | 无人消费 | runtime 已经产出结构化调用记录，transport 直接丢弃 |

最能说明问题的是最后一行：`ptc::execute` 返回的 `Outcome` 里本来就有 `calls: Vec<CallRecord>`（含 `seq/name/start/end/state`）和带 level 的 `logs`，但 `code.rs` 的 `render_run_output` 只用了 `logs/value/failure`，`calls` 没进 `data`，`LogEntry.level` 也被拍平成字符串。随后 `transcript_file` 又去解析 `1. read path=... ok` 这种文本行。

**结构化的数据被丢掉，文本反而成了契约。** 这是 direct 模式最典型的遗留：所有信息先写成人能读的文本，结构是事后补的。

### 1.3 问题清单（审计用）

| ID | 现象 | 证据 | 影响 | 本轮处理 |
|---|---|---|---|---|
| F-01 | runtime 已产出的 `calls` / `logs.level` 被 transport 丢弃，外层只保留文本账本 | `src/ptc/mod.rs:284-290`；`src/tools/code.rs:916-982`；`src/session/transcript_file.rs:336` | UI、检索、审计只能解析文本 | Phase 1 修 |
| F-02 | `hint` / `appendix` / `warning_status` 不进 `data`，code 面只剩 `status:"warning"` | `src/types/result.rs:66-72`；`src/tools/code.rs:895-914`；`src/tools/lsp_feedback.rs:47` | LSP 诊断、路径风险、参数警告静默消失 | Phase 1 修 |
| F-03 | `read` 图片返回 `parts`，但 code 面只回 `data` | `src/tools/read.rs:340-356`；`src/tools/code.rs:895` | 模型看不到图；但 D-04 决定砍媒体 | D-04 砍掉 |
| F-04 | `edit` 接受块写入、拒绝块跳过，`data` 只有 `{path, applied}` | `src/tools/edit.rs:290-317`；`src/tools/edit/feedback.rs:48-61`；`src/tools/edit.rs:65-68` | 一个调用可能只改一半；模型不知道哪块成功 | D-05 保留部分应用，补逐块 data |
| F-05 | 路径有三套方言：read 工作区相对，glob 相对 `path`，edit/write 绝对 | `src/tools/read.rs:391-396`；`src/tools/glob.rs:430-435`；`src/tools/edit.rs:252`；`src/tools/write.rs:48-52` | `glob({path:"src"})` 得到 `main.rs`，`read("main.rs")` 会读根目录 | Phase 2 修 |
| F-06 | 错误码基本未使用；`src/tools` 有 166 处 `ToolCallResult::error(...)`，默认全是 `Internal` | `src/types/result.rs:161-163`；`src/tools/read.rs:305-315`；`src/tools/grep.rs:513-515` | 程序无法按 `not_found` / `invalid_input` 分支 | Phase 1 修 |
| F-07 | 分页三套方言：read.next_start_line、grep.next_cursor、glob 无下一页 | `src/tools/read.rs:576-577`；`src/tools/grep.rs:274-277`；`src/tools/glob.rs:13,271` | 无法写统一分页；glob 静默丢 | Phase 2 修 |
| F-08 | `grep.cursor` 只是 base64 的 `{offset}`，不绑定查询 | `src/tools/grep.rs:464-475` | 换查询复用 cursor 会错位；翻页漂移 | Phase 2 修 |
| F-09 | timeout 两套单位、两种传法 | `src/tool/trait_.rs:135`；`src/tools/code.rs:1055-1074`；`src/tool/code_manifest.rs:225-229` | SDK 形状不统一 | D-09 本轮做 |
| F-10 | 工具描述还是 direct 文案：`grep.view`、read 的“line-numbered text”在 code 面不存在 | `src/tools/descriptions/grep.md`；`src/tools/descriptions/read.md` | 模型被描述误导 | Phase 1/2 重写 |
| F-11 | `PathGate` 按工具名硬编码 read/edit/write，授权前占锁 | `src/tools/code.rs:692-711,550-566`；`src/tool/executor.rs:96-101` | 审批阻塞；新工具漏锁；三套 key | Phase 4 修 |
| F-12 | 取消后 `CancelledUnfinished` 的 future 仍在跑，账本却渲染成 `cancelled` | `src/ptc/calls.rs:116-135`；`src/tools/code.rs:328` | 外层写“取消”，文件之后仍可能改 | Phase 4 修 |
| F-13 | `data_schema` 手写，且只在 code 面校验；validator 只实现子集 | `src/tool/executor.rs:736-771`；`src/tool/schema_validate.rs:294-300` | schema 与 Rust 类型漂移 | Phase 1/5 修 |
| F-14 | `edit` 精确失败后自动应用唯一 fuzzy；code 面没有信号 | `src/tools/edit/feedback.rs:54-58,209` | 不可预测 | D-05 砍掉 |
| F-15 | `read.truncated` 不是“后面还有” | `src/tools/read.rs:576-577` | 循环误判读完 | Phase 2 修 |
| F-16 | 多文件工具的授权模型不存在：`authorize` 一次 `evaluate_tool`，matcher 只读单个命名字段 | `src/tool/authorize.rs:76`；`src/permission/matchers.rs:37-40` | `apply_changes` 可能绕过路径规则 | 本批不做 apply_changes，F-16 不触发 |
| F-17 | `read` 用模型 token 预算（8000 cl100k）决定分页，code 面不可预测 | `src/tools/read.rs:16,536-560` | 页大小由模型概念决定，不是行/字节 | Phase 2 砍 |
| F-18 | `grep` 对非法正则做 BraceText / Literal 自动修复，静默改变语义 | `src/tools/grep.rs:112-165,368` | 程序以为在搜 regex，实际在搜字面量 | D-14 改为硬失败 + 建议 |
| F-19 | `edit` 的精确匹配把 smart quotes / dashes 等排版字符视为等价 | `src/tools/descriptions/edit.md`；`src/tools/edit/matcher.rs` | “看起来不同”的文本仍会命中 | D-15 砍 |
| F-20 | `glob` 自动剥离 path/pattern 重复前缀，静默改写语义 | `src/tools/glob.rs:339-363` | 传入 pattern 与执行 pattern 不一致 | D-16 砍 |

### 1.4 为什么现在改

1. **五个工具已经 code-only**：direct 文本不再被模型看到，继续维护是纯成本；文本里的事实（LSP、fuzzy、失败块）在 code 面根本不可见。
2. **D-03/D-05 已经把方向定死**：data-only、砍 fuzzy、保留部分应用；剩下的是把契约做干净。
3. **契约是后续能力的前置**：多文件编排、未来的 bash（含 `kill_shell` / `wait_shell`）code 化，都要求先有“结构化结果 + 文件版本 + 确定并发”。
4. **“能用”不等于“可编排”**：当前程序无法可靠地根据结果分支、续页、重试、判断部分成功。

---

## 2. 目标

### 2.1 总目标

把五个工具改造成 **code 原生工具**：

- 只产出结构化、带类型的 `data`；不产出 markdown，不维护 direct 文本。
- 所有非致命信号进入 `diagnostics`；LSP、路径风险、参数警告、行截断都不丢失。
- 分页统一为 `Page { has_more, next, total? }`；任何上限都可续或显式诊断。
- 文件结果带 `FileRef`（path/abs/hash/bytes）；所有写入接受 `expected_hash`。
- `edit` 砍掉 fuzzy，保留逐块部分应用；多文件由程序循环 `edit`/`write`；本批不做跨文件事务。
- 资源访问由统一 `Effect` 推导；锁在授权后获取；同 key FIFO；取消封口。
- `run_code` 的结构化账本（calls/logs/diagnostics）成为外层文本与 UI 的唯一来源。
- 图片/媒体不在本轮；`read` 遇到图片/二进制返回 `unsupported`；后续独立工具。

### 2.2 可验收目标

| ID | 目标 | 验收方式 |
|---|---|---|
| G-01 | 五个工具的 `data` 是唯一结果契约；`content/hint/appendix/warning_status` 不再被这些工具使用 | 源码扫描 + 契约测试；`rg` 门禁 |
| G-02 | LSP、路径风险、参数警告、行截断、部分失败全部以 `diagnostics` / 逐块结果到达程序 | 定向测试；`run_code` 快照 |
| G-03 | 所有分页统一为 `Page`；任何截断都有续页或诊断 | 契约矩阵 |
| G-04 | 所有路径有稳定 base；所有文件结果带 `FileRef`；read→edit→write 可链式组合 | 组合测试；SDK 快照 |
| G-05 | `edit` 的 1 个失败块不连坐其他块；逐块结果可审计；fuzzy 不存在 | 部分失败测试；fuzzy 删除扫描 |
| G-06 | 多文件由程序循环，本批不提供跨文件事务；权限按每次工具调用评估 | 程序循环/权限测试 |
| G-07 | 同一资源并发顺序确定；审批等待不占锁；取消后不产生事后写入 | 并发/审批/取消测试 |
| G-08 | 五个工具的 `data` schema 与真实输出一致，由测试矩阵强制 | 契约矩阵；schema 快照 |
| G-09 | direct 遗留代码被删除，而不是被条件分支保留 | `rg` 门禁；删除清单 |
| G-10 | 外层 `run_code` 文本由结构化 `data` 渲染，session search 继续可用 | 账本快照；检索测试 |
| G-11 | 图片/媒体不再是五个工具的能力；`read` 对图片/二进制稳定返回 `unsupported` | 图片/二进制测试 |

### 2.3 设计原则（评审 checklist）

- **P-01 单一真相**：`data` 是唯一事实。文本可以省略、可以概括，但不能包含 `data` 里没有的事实。
- **P-02 一名字一含义**：`path`、`base`、`limit`、`page`、`error.code` 在所有工具里同义。
- **P-03 限制可见且可续**：任何上限都必须有 `has_more/next` 或 `diagnostic`；禁止静默截断。
- **P-04 变更可前置，但不连坐**：写操作接受 `expected_hash`；`edit` 逐块部分应用；多文件由程序循环，本批不做跨文件事务。
- **P-05 只有一条执行路径**：嵌套与 direct 仍走同一个 `execute_tool_call`；不为 code 另开权限、锁、取消路径。
- **P-06 权限只收紧**：每个工具调用的路径都走同一权限引擎；未来若加多文件工具，必须逐 change 授权。
- **P-07 不做静默修复**：非法 regex、重复前缀、排版差异都不自动改写；`grep` 直接硬失败并给修复建议，不替模型做决定。

### 2.4 目标契约（讨论基线）

> 这里定义形状，不定义最终 Rust 类型名。Phase 1 的实现评审可以调整命名，但不能减少字段语义。
> D-07/D-10/D-11 未确认前，相关实现按“推荐”处理；D-14/D-17 已定。

#### 2.4.1 结果封套

> **已取代：** 本节信封契约（含 `ToolResult` 与 `status:"partial"`）已被 run-code-remediation-plan 取代（A1）。

```ts
type ToolResult<T> =
  | { status: "ok"; data: T; diagnostics: Diagnostic[]; page?: Page; error: null }
  | { status: "partial"; data: T; diagnostics: Diagnostic[]; page?: Page; error: null }
  | { status: "error"; data: null; diagnostics: Diagnostic[]; page?: Page; error: ToolError };

type Diagnostic = {
  code: string;                 // "lsp_error" / "path_outside_workspace" / "line_truncated" / "unknown_parameter" / "edit_block_failed"
  severity: "info" | "warning";
  message: string;
  details?: unknown;
};

type ToolError = {
  code: ToolErrorCode;
  message: string;
  details?: unknown;
  retryable: boolean;
};

type Page = {
  has_more: boolean;
  next: string | null;          // 不透明；read 用下一行号，glob/grep 用查询绑定 cursor
  total?: number;
};
```

- `ok`：完整成功。
- `partial`：已提交部分变更，但有块失败；由 `edit` 返回（D-05a 已定）。已取代：已被 run-code-remediation-plan 取代（A3）。
- `error`：没有提交任何变更，或调用失败；`details` 里可带 `blocks` / `current_hash` 等。
- 这五个工具取消 `warning` 作为独立 status（direct 工具不变）；非致命信号一律进 `diagnostics`。

#### 2.4.2 文件身份与路径

```ts
type FileRef = {
  path: string;                 // 工作区内相对；工作区外原路径
  abs?: string;                 // 仅工作区外
  hash: string;                 // sha256
  bytes: number;
};

type Base = "workspace" | "absolute";
```

- 工作区内：`path` 为工作区相对；工作区外：`path` 为原路径，`abs` 为绝对路径。
- `glob` / `grep` 结果带 `base`；`base:"workspace"` 时 paths 工作区相对，`base:"absolute"` 时 paths 为绝对路径。
- `mtime` / `eol` / `encoding` 不进 `FileRef`；`edit` 内部继续处理 BOM/EOL，但不把它们变成契约（D-18）。
- 所有写入接受 `expected_hash?: string`；不匹配返回 `stale` + `details.current_hash`。
- 工作区外路径保留；`FileRef.abs` / `base:"absolute"` 是正式契约（D-17）。

#### 2.4.3 read / glob / grep

```ts
read({
  path: string;
  start_line?: number;          // 1-based；与 end_line/limit 配合
  end_line?: number;
  limit?: number;               // 每页行数，默认 2000，最大 5000
}) => {
  file: FileRef;
  lines: { number: number; text: string; line_truncated: boolean }[];
  page: Page;                    // next 为下一行号字符串
}

glob({
  pattern: string;
  path?: string;
  include_ignored?: boolean;
  limit?: number;               // 默认 1000，最大 5000
  cursor?: string;
}) => {
  base: Base;
  paths: string[];
  page: Page;
}

grep({
  pattern: string;
  path?: string;
  glob?: string;
  case_sensitive?: boolean;
  include_ignored?: boolean;
  literal?: boolean;            // true 时按字面量搜索；默认 false
  limit?: number;               // 默认 20，最大 100
  cursor?: string;
}) => {
  base: Base;
  matches: {
    path: string;
    start_line: number;
    end_line: number;
    line_text: string;
    line_truncated: boolean;
  }[];
  page: Page;
  searched: { files: number; excluded: boolean };
  reason?: "no_match" | "corpus_empty" | "glob_empty" | "path_excluded" | "binary_file" | "file_not_searched";
}
```

- `read` 不再有 `cursor` 参数；分页只用 `start_line/end_line/limit` + `page.next`（D-08）。
- `read` 的页大小不再由模型 token 预算决定；砍掉 `INTERNAL_TOKEN_CAP`，改为行数上限 + 单行字符上限（D-19）。
- `end_line` 是显式上界；`limit` 是页大小上限；实际结束行 = `min(end_line, start_line + limit - 1)`。
- `read` 对图片/二进制返回 `unsupported`，`details` 带 `path/mime/bytes`。
- `grep` 不再有 `view`、`context_lines`；上下文由程序用 `read` 取。
- `grep` 不做正则自动修复；非法 regex 硬失败 `invalid_regex`，`details` 带 `pattern` / `regex_error` / `position`（若可用）/ `suggested_literal` / `suggested_pattern`；`retryable:true`（D-14）。其中 `suggested_literal` / `suggested_pattern` 与 `retryable:true` 已取代：已被 run-code-remediation-plan 取代（A1/A4）。
- 需要按字面量搜索时显式 `literal:true`；错误里的建议只是建议，不自动应用。
- `glob` 不再自动剥离 path/pattern 重复前缀；按传入 pattern 匹配（D-16）。
- `read` 的 `page.has_more` 必须回答“后面还有没有”，显式窗口也一样。

#### 2.4.4 edit / write

```ts
edit({
  path: string;
  expected_hash?: string;
  edits: {
    old_string: string;
    new_string: string;
    replace_all?: boolean;
  }[];
}) => {
  file: FileRef;                 // 更新后的 hash
  blocks: {
    index: number;
    status: "applied" | "failed";
    ranges: { start_line: number; end_line: number }[];
    reason?: { code: string; message: string; details?: unknown };
  }[];
  applied: number;
  failed: number;
}

write({
  path: string;
  content: string;
  expected_hash?: string;
  create_only?: boolean;
}) => {
  file: FileRef;
  created: boolean;
}
```

- `edit` 只做精确匹配（保留 CRLF/LF、BOM 归一）；fuzzy 整条链砍掉；没有 `match` 参数（D-05）。
- `edit` 默认逐块部分应用：所有块对“原始文件”匹配；成功的块写入；失败的块不连坐。状态：全成功 `ok`，部分成功 `partial`，全失败 `error`。其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
- `edit` 的失败块必须带 `reason.code`（如 `no_match` / `multiple_matches` / `empty_old_string` / `no_op` / `chained` / `overlap` / `whitespace_only`）。
- `edit` 的失败块同时产出 `diagnostic`（`edit_block_failed`，details 带 index/reason），由 `run_code` 聚合；这样即使程序忽略返回值，外层模型仍能看到哪些块失败。
- `edit` 的 `expected_hash` 可选；省略时不做版本校验；不匹配时整次调用 `stale`，不写入。
- `replace_all` 只对精确匹配生效；fuzzy 不存在，因此不存在“模糊 replace_all”。
- `write` 的 `create_only` 与 `expected_hash` 同时给且文件不存在时，返回 `invalid_input`。
- `edit` 是单文件、多块；多文件由 `run_code` 程序循环调用 `edit`/`write`，本批不做跨文件事务（D-06/D-20）。
- `edit` 部分成功返回 `partial`（D-05a）。已取代：已被 run-code-remediation-plan 取代（A3）。

#### 2.4.5 run_code 传输

> **已取代：** 本节 run_code data 已被 run-code-remediation-plan 取代（E-8）。

```ts
run_code({ code: string; timeout_ms?: number }) => {
  result?: unknown;
  calls: {
    seq: number;
    tool: string;
    args_summary: string;        // 仅展示/审计，不是程序契约
    outcome: "ok" | "partial" | "error" | "cancelled" | "unsettled";
    error_code?: string;
    started_ms: number;
    ended_ms?: number;
  }[];
  calls_omitted: number;         // 超过上限的调用数，不静默丢
  logs: { level: "log" | "info" | "warn" | "error" | "debug"; message: string }[];
  diagnostics: Diagnostic[];     // 嵌套诊断聚合（去重、限量）
  diagnostics_omitted: number;
}
```

- 外层文本由这份 `data` 渲染；不再有第二份账本。
- `calls` 合并 runtime 的 `CallRecord` 与 host 的 `args_summary/outcome`。
- 聚合嵌套诊断；edit 的 LSP 诊断即使程序忽略返回值，外层也能看到。
- 媒体不聚合（D-04）。
- 删除 per-call `options`；`run_code.timeout_ms` 作为整批预算保留；工具内部兜底超时（D-09）。
- 每个工具保留内部默认超时（如 glob/grep），只作为安全网；模型不能按调用覆盖，只能控制 `run_code.timeout_ms`。

#### 2.4.6 契约不变量

| ID | 不变量 |
|---|---|
| C-01 | 五个工具的 `data` 是唯一结果契约；`content` 不再承载事实 |
| C-02 | `ok` / `partial` 必有 `data` 且通过 schema；`error` 必无 `data`，必有 `error`。其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A1/A3） |
| C-03 | 任何截断/上限都必须有 `page.has_more`、`page.next` 或 `diagnostic` |
| C-04 | 所有路径有稳定 `base`；工作区内相对，工作区外 `abs` |
| C-05 | 所有写结果返回新的 `FileRef`；所有写接受 `expected_hash` |
| C-06 | `edit` 无 fuzzy；逐块部分应用；失败块不连坐成功块（D-05a） |
| C-07 | 资源访问由统一 `Effect` 推导；锁在授权后获取；同 key FIFO |
| C-08 | 取消必须封口：mutating call 要么在返回前结束，要么明确 `unsettled`；不得事后静默落盘 |
| C-09 | 嵌套与 direct 共用 `execute_tool_call`；不得新增第二套权限/锁/取消路径 |
| C-10 | 多文件由程序循环；未来若加多文件工具，必须逐 change 授权 |
| C-11 | `data` schema 由契约测试强制；真实输出的每个分支都必须过 schema |
| C-12 | 文本是 `data` 的纯渲染；不得包含 `data` 之外的事实 |
| C-13 | 五个工具不产出/不聚合媒体；图片/二进制是 `unsupported` |
| C-14 | 不做静默语义修复：非法 regex 直接报错并给建议；重复前缀、排版差异不静默改写 |

---

## 3. 阶段计划

> 每个阶段都以“可独立验收、可独立回滚”为准。阶段门禁不过，不进入下一阶段。
> 阶段顺序可调整；Phase 3 只做单文件写入，多文件由程序循环，不引入事务工具。

### Phase 0：基线与防回退

**目标**：在改行为之前，固定“现在是什么样”的证据，防止改造中悄悄回退。

**落地细则**

- **P0-01 现状快照**：为五个工具各建 fixture，覆盖成功、空结果、截断、非法输入、权限拒绝、取消、部分失败；记录 `data/error`（`content` 只作历史对照，不再作为断言目标）。
- **P0-02 SDK 快照**：记录当前 `run_code` 生成的 SDK 全文与哈希。
- **P0-03 契约矩阵骨架**：建立“工具 × 场景 → 期望 status/data/page/diagnostics/error.code”的表驱动骨架。
- **P0-04 删除清单**：把 §5.3 的 direct 遗留逐项列成 `rg` 门禁（允许暂时失败，Phase 5 全绿）。
- **P0-05 旧 transcript 回放确认**：确认旧会话里 direct `read/edit` 的文本仍能正常显示/检索；本计划不重写历史。
- **P0-06 构建基线**：`cargo check`、相关测试、`npx tsc -b`（若改 web）全部通过。

**验收门禁**

- [x] `cargo check` 通过。
- [x] 现有相关测试通过。
- [ ] SDK 快照与 fixture 已提交（本地分支）。
- [x] 契约矩阵骨架能跑（允许空断言）。
- [x] `rg` 删除清单已建立，但不要求绿。
- [x] 旧 transcript 回放测试通过。

**回退**：Phase 0 只加测试与文档；回退即删除新增 fixture/快照。

---

### Phase 1：单一结果契约与诊断

**目标**：五个工具只产出 `data`；所有非致命信号进入 `diagnostics`；`run_code` 结构化账本成为外层文本唯一来源。

**落地细则**

- **P1-01 定义结果类型**：在 `src/tool/` 新增 `ToolOutcome<T>`、`ToolError`、`Diagnostic`、`Page`、`FileRef`（或等价命名）。
- **P1-02 管线适配**：`execute_tool_call` 对 `ToolCaller::Code` 走 data-only 终结：跳过 `wire_result`、`apply_emergency_cap`、`spill`、`truncated_tool_result`；只做 schema 校验与错误映射。direct 工具路径不变。
- **P1-03 五工具迁移**：`read/glob/grep/edit/write` 的 `execute` 返回 `ToolOutcome`；删除对 `hint/appendix/warning_status/level` 的写入。
- **P1-04 诊断迁移**：LSP、路径风险、unknown-arg、read/grep 行截断、grep 非法 regex、edit 失败块，全部改为 `Diagnostic` 或错误码。
- **P1-05 `run_code` 账本结构化**：`data` 增加 `calls`、`calls_omitted`、`logs[{level,message}]`、`diagnostics`、`diagnostics_omitted`；文本从这份 data 渲染。
- **P1-06 媒体砍除**：删除 `read` 的图片检测、`media parts`、`MAX_MEDIA_BLOB_SIZE` 分支；图片/二进制返回 `unsupported`（`details.path/mime/bytes`）；`run_code` 不聚合媒体。
- **P1-07 测试改造**：所有依赖 `content` 的旧测试改为断言 `data`；新增 LSP/路径风险/unknown-arg/图片不复存在 的测试。
- **P1-08 SDK 文案**：重写 `code_manifest` 的 PRELUDE 和五个工具描述为 code 语义；删除 warning/text-view 语言；删除 per-call options 文案，保留 `run_code.timeout_ms`；工具默认超时内部化（D-09）。

**验收门禁**

- [x] `cargo check` 通过。
- [x] LSP 诊断测试：`edit` 后错误诊断出现在 `ToolOutcome.diagnostics`，并出现在 `run_code.data.diagnostics`。
- [x] 路径风险测试：工作区外写入产生 `path_outside_workspace` 诊断。
- [x] unknown-arg 测试：多传参数产生诊断。
- [x] 图片测试：`read` 图片返回 `unsupported`，没有 `parts`。
- [x] `run_code.data.calls` 含 seq/tool/outcome/state；文本账本与 calls 一致。
- [x] `run_code.data.logs` 保留 level。
- [x] 五个工具不再写 `hint/warning_status/appendix`；`rg` 门禁绿。
- [x] SDK 快照更新且评审通过。

**回退**：本阶段不改工具语义，只改返回通道；失败则回退到 Phase 0 的旧 `ToolCallResult` 适配层。

---

### Phase 2：文件身份与分页

**目标**：路径统一、文件可版本化、分页统一且可续；消除 read/glob/grep 的静默截断、token 预算和三种方言。

**落地细则**

- **P2-01 FileRef/Base**：实现 hash（`sha2`）、bytes、工作区相对/绝对 base；`mtime/eol/encoding` 不进入契约（D-18）。
- **P2-02 `read`**：返回 `{file, lines, page}`；`page.has_more` 总是回答“后面还有没有”；显式窗口也返回 `next`；砍掉 `cursor` 参数与 `INTERNAL_TOKEN_CAP`，改为 `limit` 行数 + 单行字符上限；删除行号文本渲染。
- **P2-03 `glob`**：返回 `{base, paths, page}`；路径按 base；`limit/cursor` 分页；cursor 绑定 `(pattern, path, filters)`，换查询返回 `stale_cursor`；排序写进 schema（depth → parent → name）；不再自动剥离 path/pattern 前缀（D-16）。
- **P2-04 `grep`**：返回 `{base, matches, page, searched, reason?}`；cursor 绑定 `(pattern, path, glob, filters, snapshot/version)`；换查询复用 cursor 返回 `stale_cursor`；删除 `view`、`context_lines`、文本渲染与正则自动修复；非法 regex 直接 `invalid_regex`，details 给 literal/转义建议；新增显式 `literal`（D-14）。
- **P2-05 零命中语义**：`reason` 区分 `no_match/corpus_empty/glob_empty/path_excluded/binary_file/file_not_searched`。
- **P2-06 组合测试**：`glob → read`、`grep → read`、`read → edit(expected_hash)` 三条链必须有测试。
- **P2-07 预算**：为 grep 全量扫描加总量上限（或分页索引），避免 `max_matches: usize::MAX` 的内存/时间风险。

**验收门禁**

- [x] `cargo check` 通过。
- [x] `read` 显式窗口在文件未读完时 `page.has_more=true` 且 `next != null`。
- [x] `read` 不再受 token 预算影响；页大小只由 `limit` 与行字符上限决定。
- [x] `glob` 超过 limit 时 `has_more=true`，路径可直接传给 `read`。
- [x] `grep` 换 pattern 复用 cursor 返回 `stale_cursor`；同一查询翻页稳定。
- [x] 零命中五种原因各有测试。
- [x] 非法 regex 返回 `invalid_regex`，details 含 pattern/regex_error/position/suggested_literal/suggested_pattern；`literal:true` 正常。其中 `suggested_literal` / `suggested_pattern` 已取代：已被 run-code-remediation-plan 取代（A1/A4）。
- [x] 工作区外路径：`base:"absolute"`，`read` 可消费。
- [x] SDK 快照中不再出现 `view`、`file_path`、`context_lines`。
- [x] 组合链测试通过。

**回退**：按工具拆小提交，任一工具失败可单独回退。

---

### Phase 3：写入原子性与前置条件

**目标**：写入可前置、可预测、不连坐；单文件多块由 `edit` 承担，多文件由程序循环，本批不做跨文件事务。

**落地细则**

- **P3-01 `write`**：接受 `expected_hash`；返回 `FileRef`；`create_only` 保留；删除文本状态，改为 `created` + `file`。
- **P3-02 `edit`**：保留逐块部分应用；返回 `blocks[]`（status/ranges/reason）+ `file` + `applied/failed`；全成功 `ok`，部分成功 `partial`，全失败 `error`；不连坐。失败块同时产出 `edit_block_failed` 诊断，供 `run_code` 聚合。其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
- **P3-03 fuzzy 整链删除**：删除 `match` 参数、fuzzy matcher 分支、`MatchKind::Fuzzy`、fuzzy 提示与测试；同时删除 typography 等价（D-15）；`edit` 只做精确匹配（保留 CRLF/LF、BOM 归一）。
- **P3-07 LSP 诊断**：编辑后的 LSP 诊断进入 `diagnostics`，由 `run_code` 聚合；即使程序忽略返回值，外层也能看到。

**验收门禁**

- [x] `cargo check` 通过。
- [x] `expected_hash` 不匹配返回 `stale`，文件未修改。
- [x] `edit` 10 块中 1 块失败：其余 9 块写入，`status:"partial"`，`blocks` 能指出失败块与原因，`run_code.data.diagnostics` 有 `edit_block_failed`。其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
- [x] `edit` 全部失败：`status:"error"`，文件未修改，`error.details.blocks` 可读。
- [x] fuzzy 与 typography 等价代码不存在；`rg` 无残留。
- [x] LSP 诊断在 `run_code.data.diagnostics` 可见。

**回退**：本批不做 `apply_changes`；只保留 write/edit 的前置条件与逐块部分应用。

---

### Phase 4：并发、锁与取消

**目标**：资源访问由一套模型推导；同 key 顺序确定；审批不占锁；取消不产生事后写入。

**落地细则**

- **P4-01 Effect 模型**：工具契约增加 `effects(input) -> { reads: ResourceKey[], writes: ResourceKey[] }`；五个工具实现；`resource_keys` 与 `is_concurrency_safe` 由它派生或删除。
- **P4-02 统一锁管理器**：用同一 `ResourceKey` 函数（大小写、`..`、分隔符与 `resolve_agent` 一致）替换 `PathGate` 与 `path_gate_key`；同 key FIFO；多 key 按排序后的 key 顺序获取，避免死锁。
- **P4-03 授权后取锁**：锁必须在 `authorize` 之后获取；审批等待期间不持有资源锁。
- **P4-04 取消封口**：mutating call 在返回前必须 settle；宽限期结束仍未结束时记录 `unsettled`，不得渲染成 `cancelled`；外层明确“可能已提交/未提交”。
- **P4-05 账本诚实**：`run_code.data.calls[].outcome` 区分 `ok/partial/error/cancelled/unsettled`；文本账本同步。其中 run_code data 已取代：已被 run-code-remediation-plan 取代（E-8）。
- **P4-06 timeout 收口（D-09）**：删除 per-call `options`、`split_invocation` 双参数、`parse_call_timeout_ms`；所有 binding 只接受一个参数；工具内部以 `Duration`/毫秒兜底；`run_code.timeout_ms` 保留为整批预算。

**验收门禁**

- [x] `cargo check` 通过。
- [x] 审批挂起测试：审批期间其他同路径 read/write 不被阻塞（或按 FIFO 排队，而不是被预占）。
- [x] 同 key 两个 write 的完成顺序等于提交顺序。
- [x] 取消测试：取消后不再发生写入；若无法保证，返回 `unsettled` 而不是 `cancelled`。
- [x] `PathGate`/`path_gate_key` 删除，`rg` 无残留。
- [x] 所有 binding 只接受一个参数；传第二个参数返回 `invalid_input`。
- [x] SDK 不再生成 `options`；工具默认超时内部化为 `Duration`/毫秒。

**回退**：锁管理器可按工具灰度；先 read（共享）后 write/edit（独占）。

---

### Phase 5：清理、文档与契约自动化

**目标**：删除 direct 遗留；把契约固定为自动测试；让 UI/检索消费结构化数据；交付可审计的完成证据。

**落地细则**

- **P5-01 删除清单执行**：按 §5.3 删除 direct 遗留；删除文本渲染器、参数别名、告警字段、fuzzy matcher、媒体分支、token 预算、锁代码。
- **P5-02 schema 单一来源**：在“手写 + 契约矩阵”与“从 Rust 类型生成”之间按 D-10 决策；无论哪种，真实输出的每个分支都必须过 schema。
- **P5-03 契约矩阵填满**：工具 × 场景全覆盖（成功/空/截断/非法/过期/拒绝/取消/部分失败）。
- **P5-04 UI/检索迁移**：`RunCodeToolView` 可读取 `data.calls/logs/diagnostics` 做结构化展示；session search 优先读结构化账本，保留文本回退。
- **P5-05 文档**：更新 `docs/ptc/charter.md` 或新增 ADR（需产品负责人确认）；更新工具描述为 code 语义。
- **P5-06 最终门禁**：`cargo check`、相关测试、`npx tsc -b`、`rg` 删除清单全绿；手工验收程序通过。

**验收门禁**

- [x] `cargo check` 通过。
- [x] `cargo test` 相关模块全绿。
- [x] `npx tsc -b` 通过（若改 web）。
- [ ] `rg` 删除清单全绿。除 spill-emergency 外为绿；该条保留是因为共享函数仍服务 direct 工具。
- [x] 契约矩阵全绿。
- [x] SDK 快照稳定：同一工具集、不同 cwd/权限下哈希一致。
- [x] 手工验收程序：read→grep→edit(partial)→write→取消→审批，全部符合预期。其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
- [x] 旧 transcript 回放与 session search 测试通过。

**回退**：Phase 5 删除项按文件拆提交；任一删除导致回归，单独恢复该文件，不影响前四阶段契约。

---

## 4. 整批做什么

### 4.1 契约与结果

- 五个工具改为 data-only：`data` + `diagnostics` + `page` + `error`。
- 删除 `warning` 作为独立 status；非致命信号进 `diagnostics`；`edit` 部分成功用 `partial`。其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
- 建立稳定错误码与 `details`；不再默认 `Internal`。
- 五个工具的 `data_schema` 与真实输出由契约矩阵强制一致。

### 4.2 文件与分页

- 统一 `FileRef`（path/abs/hash/bytes）。
- 统一 `Base`：工作区内相对，工作区外绝对。
- 统一 `Page`；read/glob/grep 全部支持续页。
- 删除 glob 1000 条静默截断；删除 `read.truncated` 的歧义。
- `grep.cursor` 绑定查询；换查询返回 `stale_cursor`。
- 删除 read token 预算；页大小由 limit 与单行上限决定。

### 4.3 写入与原子性

- `write/edit` 接受 `expected_hash`；返回新的 `FileRef`。
- `edit` 砍 fuzzy，保留逐块部分应用；返回逐块结果；失败不连坐。
- 多文件由程序循环 `edit`/`write`；本批不做跨文件事务或回滚。

### 4.4 并发与取消

- 统一 `Effect` 模型；锁由资源 key 推导；同 key FIFO。
- 授权后取锁；审批不占锁。
- 取消对 mutating call 封口；`unsettled` 诚实上报。
- 删除 per-call options；工具内部兜底；`run_code.timeout_ms` 保留为整批预算（D-09）。

### 4.5 传输与外围

- `run_code.data` 增加 `calls/logs(level)/diagnostics`；文本由 data 渲染。
- 媒体不聚合（D-04）。
- UI/session search 优先消费结构化账本。
- 删除 direct 遗留代码与文本渲染器。
- 建立契约矩阵、SDK 快照、`rg` 删除门禁。

### 4.6 工作项索引（审计表）

| ID | 阶段 | 工作项 | 主要文件 | 验收 |
|---|---|---|---|---|
| P0-01 | 0 | 五工具 fixture（data/error） | `src/tools/*/tests` | fixture 评审 |
| P0-02 | 0 | SDK 快照 | `src/tool/code_manifest.rs` 测试 | 哈希记录 |
| P0-03 | 0 | 契约矩阵骨架 | `src/tool/` tests | 可运行 |
| P0-04 | 0 | 删除清单 `rg` 门禁 | `scripts/` 或测试 | 清单建立 |
| P0-05 | 0 | 旧 transcript 回放 | `src/session/` tests | 通过 |
| P1-01 | 1 | 结果类型 | `src/tool/outcome.rs` | `cargo check` |
| P1-02 | 1 | 管线 data-only 终结 | `src/tool/executor.rs` | 定向测试 |
| P1-03 | 1 | 五工具迁移 | `src/tools/*.rs` | 契约测试 |
| P1-04 | 1 | 诊断迁移 | LSP/path/file_path | 定向测试 |
| P1-05 | 1 | 结构化账本 | `src/tools/code.rs` | 快照 |
| P1-06 | 1 | 媒体砍除 | `src/tools/read.rs` | 图片测试 |
| P1-07 | 1 | 测试改造 | tests | 全绿 |
| P1-08 | 1 | SDK 文案 | manifest/descriptions | 快照 |
| P2-01 | 2 | FileRef/Base | `src/tool/` | 单测 |
| P2-02 | 2 | read 分页/去 token | `src/tools/read.rs` | 分页测试 |
| P2-03 | 2 | glob 分页/路径 | `src/tools/glob.rs` | 分页测试 |
| P2-04 | 2 | grep 分页/cursor/regex 诊断 | `src/tools/grep.rs` | stale/修复诊断 |
| P2-05 | 2 | 零命中 reason | `src/tools/grep.rs` | 五原因测试 |
| P2-06 | 2 | 组合链 | tests | 通过 |
| P3-01 | 3 | write 前置条件 | `src/tools/write.rs` | stale 测试 |
| P3-02 | 3 | edit 部分应用/逐块 | `src/tools/edit*` | 部分失败测试 |
| P3-03 | 3 | fuzzy 整链删除 | `src/tools/edit/matcher.rs` | `rg` 无残留 |
| P4-01 | 4 | Effect 模型 | `src/tool/trait_.rs` | 单测 |
| P4-02 | 4 | 统一锁 | `src/tools/code.rs` | FIFO 测试 |
| P4-03 | 4 | 授权后取锁 | `src/tools/code.rs` | 审批测试 |
| P4-04 | 4 | 取消封口 | `src/tools/code.rs` | 取消测试 |
| P4-05 | 4 | 账本诚实 | `src/tools/code.rs` | 快照 |
| P4-06 | 4 | timeout 收口 | `src/tools/code.rs` / manifest | 单参数/单位测试 |
| P5-01 | 5 | 删除遗留 | 多文件 | `rg` 绿 |
| P5-02 | 5 | schema 单一来源 | 多文件 | 契约矩阵 |
| P5-03 | 5 | 矩阵填满 | tests | 全绿 |
| P5-04 | 5 | UI/检索迁移 | web/session | `tsc` + 测试 |
| P5-05 | 5 | 文档 | docs | 评审 |
| P5-06 | 5 | 最终门禁 | — | 全部通过 |

---

## 5. 整批不做什么

### 5.1 不保留的 direct 兼容

- 不为五个工具保留 direct 文本输出、`content` 事实、`hint/appendix/warning_status`。
- 不保留 `file_path` 历史别名；不保留 `accepts_historical_file_path` / `prepare_input` / `schema_arguments` 的别名逻辑。
- 不保留 `grep.view`、`context_lines`、“文本分页”概念。
- 不保留 `PathGate`/`path_gate_key`；不保留按工具名硬编码的锁。
- 不保留 fuzzy 匹配；typography 等价与 path/pattern 自动剥离已砍（D-15/D-16）。
- 不保留 `read` 的图片/媒体、token 预算、行号文本。
- 不重写旧 transcript；旧会话按原样回放，不迁移到新 schema。

### 5.2 不在本批范围

- `bash` / `kill_shell` / `wait_shell` 的 code 化（D-11；需要独立项目：jobs、PTY、流式、取消）。
- `subagent`、`todo/plan`、`knowledge`、`litecode_workspace`、`session_search`、`code_search`、`lsp`、`webfetch`、`websearch`、`ask_user`、custom、MCP 的 code 化。
- 持久 REPL、跨调用状态、TypeScript 运行时、任意 `import/require`、网络。
- per-agent 的 code 开关或权限预设。
- UI 视觉重设计；只做必要的结构化数据消费。
- 版本号、CHANGELOG、tag、Release、commit/push（未经产品负责人明确许可）。
- 图片/媒体工具（本轮砍掉；后续另开 `read_image` / `media` 工具）。
- `apply_changes` / 跨文件事务：本批不做；多文件由程序循环，后续确有需要再开独立工具。
- 图片/媒体的 base64 塞进 data。

### 5.3 明确删除清单（direct 遗留）

- `ToolCallResult` 在五个工具中的 `content` 事实地位。
- `hint` / `warning_status` / `appendix` 在五个工具中的使用。
- `warning` 作为独立结果 status（保留 `partial` 作为变更结果，不保留 warning）。其中保留 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
- `file_path` 别名与历史兼容分支。
- `grep.view`、`grep.context_lines`、所有展示型参数。
- read 的行号文本渲染/pagination footer、glob 的列表文本、grep 的文本视图、edit/write 的文本状态渲染。
- `PathGate` / `path_gate_key` / 按工具名的 path 提取。
- 五个工具的 `max_result_size` / spill / emergency cap 路径。
- 五个工具的 `is_concurrency_safe` 特判（由 `Effect` 取代）。
- fuzzy matcher 整条链（`MatchKind::Fuzzy`、fuzzy 提示、fuzzy 测试）。
- `read` 的图片检测、`media parts`、`MAX_MEDIA_BLOB_SIZE` 分支。
- `read` 的 `INTERNAL_TOKEN_CAP` 与按 token 决定页大小的逻辑。
- `grep` 的 BraceText/Literal 自动修复与文本提示（改为硬失败 + 建议）。
- per-call options / `split_invocation` 双参数（D-09）。
- `glob` 的 path/pattern 前缀自动剥离（D-16）。
- `edit` 的 typography 等价（D-15）。
- `edit` 的 `on_error` / `match` 参数（已无 fuzzy/all-or-none 分支）。
- `schema_validate` 中未被真正执行的装饰性约束，或补齐执行（按 D-10）。

### 5.4 疑似 direct 时代遗留审计

| ID | 遗留 | 处理 | 理由 |
|---|---|---|---|
| L-01 | `grep.view`（lines/context/expanded） | 砍 | 纯展示；code 只需 matches |
| L-02 | `grep` 正则自动修复（BraceText/Literal） | 砍（D-14） | 静默改变语义；改硬失败 + 显式 `literal` + 错误建议 |
| L-03 | `grep.context_lines` | 砍 | 程序可用 `read` 取上下文，避免第二套展示 |
| L-04 | `read` 的 token 预算分页 | 砍 | 模型上下文概念；改 `limit` 行数/字节 |
| L-05 | `read` 行号文本 + footer | 砍 | code 面只给 `lines[]` |
| L-06 | `read` 图片/媒体 | 砍（D-04） | 媒体独立工具后续 |
| L-07 | `edit` fuzzy 链 | 砍（D-05） | 实测令模型困惑 |
| L-08 | `edit` typography 等价 | 砍（D-15） | “看起来不同”仍会命中，可能同样困惑 |
| L-09 | `edit` `on_error` / `match` | 砍 | 只剩 exact + partial，无开关 |
| L-10 | `edit` 文本反馈 + `output_limit` | 砍 | 改 `blocks[]` |
| L-11 | `write` 文本状态 | 砍 | 改 `file/created` |
| L-12 | `glob` path/pattern 前缀自动剥离 | 砍（D-16） | 静默改写语义 |
| L-13 | `glob` 列表文本/warnings | 砍 | 改 `paths/page/diagnostics` |
| L-14 | 五个工具的 `max_result_size` / spill / emergency cap | 砍 | code 面 data-only，不再服务模型文本 |
| L-15 | `PathGate` / `path_gate_key` | 砍 | 改 `Effect` + 统一锁 |
| L-16 | `file_path` / `prepare_input` / `schema_arguments` | 砍 | direct 别名 |
| L-17 | `hint/warning_status/appendix/level`（五工具） | 砍 | 改 `diagnostics` |
| L-18 | `ToolCallResult.content` 的事实地位（五工具） | 砍 | `data` 唯一真相 |
| L-19 | `is_concurrency_safe` 特判（五工具） | 砍 | `Effect` 推导 |
| L-20 | `read` 图片检测 / `MAX_MEDIA_BLOB_SIZE` | 砍 | D-04 |
| L-21 | `grep` `GrepShape` / 文本渲染 | 砍 | 纯展示 |
| L-22 | `read` `cursor` + `start_line/end_line` 双分页 | 砍 `cursor`（D-08） | 一套分页即可 |
| L-23 | `FileRef` `mtime/eol/encoding` | 砍（D-18） | 内部处理，不暴露 |
| L-24 | `run_code` 媒体聚合 | 砍 | D-04 |
| L-25 | `run_code` 文本账本作为唯一账本 | 改为 data 优先，文本渲染 | F-01 |
| L-26 | `suggest_similar_files` 文本提示 | 保留能力，迁到 `error.details` | 已取代：已被 run-code-remediation-plan 取代（A4）。原文：有用，不是展示 |
| L-27 | `Tool::call` / `call_inner` 测试辅助 | 评估；不作为产品契约 | 测试内部 |
| L-28 | `edit` `SNAPSHOT_RULE` 文本 | 改写为部分应用语义 | 旧规则会被读成 all-or-none |
| L-29 | `grep` “more matches” 文本 footer | 砍 | 改 `page.next` |
| L-30 | `read` 的“line-numbered text”描述 | 改写 | code 面不存在 |
| L-31 | per-call options / `split_invocation` 双参数 | 砍（D-09） | 所有 binding 统一一个参数 |
| L-32 | `Tool::timeout` 秒单位 | 内部改 `Duration`/毫秒 | 模型只看到 `run_code.timeout_ms` |

### 5.5 禁止事项（AGENTS 约束）

- 不新增第二套 Message/Stream/Item 类型。
- 不让工具拥有 LSP/检索生命周期。
- 不放宽权限或敏感路径 floor。
- 不混入调试/测试残留、临时文件、本地配置。
- 未经确认不 commit/push/amend/force，不升版本号、不打 tag。

---

## 6. 决策登记

> 规则：只有产品负责人明确确认的结论才写入本节。未确认的一律“待定”，不得在实现中默认选择。

| ID | 决策 | 状态 | 备注 |
|---|---|---|---|
| D-01 | 五个已上线工具完全改造为 code 模式，不再维护 direct 行为 | 已定 | 产品负责人，2026-10-20 |
| D-02 | 允许砍掉不契合、不可预测、不稳定、多余的设计 | 已定 | 产品负责人，2026-10-20 |
| D-03 | 五个工具放弃 markdown/`content`，只产出结构化、带类型的 `data`，完全契合 code 形态 | 已定 | 产品负责人，2026-10-20 |
| D-04 | 图片与媒体整体砍掉；`read` 只读文本，图片/二进制返回 `unsupported`；后续另开工具 | 已定 | 产品负责人，2026-10-20 |
| D-05 | fuzzy 整链砍掉；`edit` 保留逐块部分应用，一个块失败不连坐其他块 | 已定 | 产品负责人，2026-10-20 |
| D-05a | `edit` 部分成功用 `status:"partial"`（三态） | 已定 | 已取代：已被 run-code-remediation-plan 取代（A3）。原文：与现有三态一致 |
| D-06 | 本批不做多文件 `apply_changes`；多文件由 `run_code` 程序循环 `edit`/`write` | 已定 | 不做跨文件回滚；后续确有需要再开独立工具 |
| D-06a | `apply_changes` 事务语义 | 撤销（不适用） | D-06 决定本批不做 |
| D-07 | grep/glob cursor 快照策略：查询指纹 + 版本校验；或全量快照 | 待定 | 推荐查询指纹 + 版本校验 |
| D-08 | `read` 只保留 `start_line/end_line/limit` + `page.next`，不引入不透明 cursor | 已定（设计裁剪） | 一套分页；如需要再恢复 |
| D-09 | 删除 per-call options；工具内部兜底；只保留 `run_code.timeout_ms` 整批预算 | 已定（本轮做） | 产品负责人 2026-10-20 |
| D-10 | schema 单一来源：手写 + 穷举契约测试；或从 Rust 类型生成 | 待定 | 推荐先手写 + 穷举测试，不新增依赖 |
| D-11 | 本批是否 code 化 `bash` / `kill_shell` / `wait_shell` | 待定 | 推荐“否”，独立项目；没有 `exec` 工具（`bash.exec` 是权限 rule id） |
| D-12 | 是否保留 `ToolSurface` 与 direct 分支 | 约束（charter D2） | 其他工具仍是 direct，只移除五工具遗留 |
| D-13 | 路径 base：工作区内相对，工作区外绝对；glob/grep 带 `base` | 已定（设计） | 与 `FileRef.abs` 一致 |
| D-14 | grep 不做正则自动修复；非法 regex 硬失败 `invalid_regex`，details 给 literal/转义建议；需要字面量用 `literal:true` | 已定 | 产品负责人 2026-10-20；可靠优先，不替模型做决定。其中 `suggested_*` 与 `retryable:true` 已取代：已被 run-code-remediation-plan 取代（A1/A4） |
| D-15 | edit 的 typography 等价（smart quotes/dashes）砍掉；只保留 CRLF/BOM 归一 | 已定（设计裁剪） | 与 fuzzy 同类：不做“看起来不同却命中”；可恢复 |
| D-16 | glob 的 path/pattern 前缀自动剥离砍掉；按传入 pattern 匹配 | 已定（设计裁剪） | 不做静默语义改写；可恢复 |
| D-17 | 保留工作区外路径；`FileRef.abs` / `base:"absolute"` | 已定 | 产品负责人 2026-10-20 |
| D-18 | `FileRef` 只保留 path/abs/hash/bytes；mtime/eol/encoding 不进契约 | 已定（设计裁剪） | edit 内部继续处理 BOM/EOL |
| D-19 | `read` 砍掉 token 预算，改 `limit` 行数 + 单行字符上限 | 已定（设计裁剪） | code 程序不受模型 token 预算约束 |
| D-20 | `edit` 形态保持 `{path, edits[]}`；单次调用单文件，多文件多块由 `run_code` 程序循环 | 已定 | 不改 edit 形态；`edits[]` 保证 9/10，多文件由程序编排 |

---

## 7. 验收与审计方法

### 7.1 通用门禁

- `cargo check` 通过。
- 本阶段新增/修改的测试与既有相关测试通过。
- 改了 `web/` 时 `npx tsc -b` 通过。
- 契约矩阵对应行全绿。
- SDK 快照与预期一致（同一工具集、不同 cwd/权限下哈希稳定）。
- `rg` 删除清单全绿（Phase 5）。

### 7.2 契约矩阵模板

> **已取代：** 表中 `partial` 状态已被 run-code-remediation-plan 取代（A3）；`run_code` 的 data 列已被取代（E-8）。

| 工具 | 场景 | status | data 关键字段 | page | diagnostics | error.code |
|---|---|---|---|---|---|---|
| read | 正常文本 | ok | file.hash/lines | has_more | [] | null |
| read | 空文件 | ok | lines=[] | 完成 | [] | null |
| read | 显式窗口未读完 | ok | lines | has_more=true,next | [] | null |
| read | 单行超长 | ok | lines[].line_truncated=true | — | line_truncated | null |
| read | 文件不存在 | error | null | — | [] | not_found |
| read | 图片/二进制 | error | null | — | [] | unsupported |
| glob | 正常 | ok | base/paths | next | [] | null |
| glob | 超 limit | ok | paths | has_more=true | [] | null |
| grep | 有命中 | ok | base/matches | next | [] | null |
| grep | 零命中 | ok | matches=[] | 完成 | [] | reason=no_match |
| grep | 非法 regex | error | null | — | [] | invalid_regex + suggestion |
| grep | literal 命中 | ok | matches | — | [] | null |
| grep | cursor 过期 | error | null | — | [] | stale_cursor |
| edit | 全部成功 | ok | file/blocks/applied | — | [] | null |
| edit | 10 块 1 失败 | partial | file/blocks/applied/failed | — | edit_block_failed | null |
| edit | 全部失败 | error | null | — | edit_block_failed | edit_failed + blocks |
| edit | hash 过期 | error | null | — | [] | stale |
| edit | LSP 错误 | ok/partial | file/blocks | — | lsp_error | null |
| write | 成功 | ok | file/created | — | [] | null |
| write | hash 过期 | error | null | — | [] | stale |
| write | create_only 冲突 | error | null | — | [] | conflict |
| run_code | 正常 | ok | calls/logs/result | — | 聚合 | null |
| run_code | 嵌套部分成功 | ok | calls[].outcome=partial | — | — | — |
| run_code | 嵌套取消 | ok/error | calls[].outcome=cancelled/unsettled | — | — | — |

### 7.3 证据规则

- 每个工作项在提交说明/评审记录里写：测试名、`文件:行号`、门禁结论。
- 不用“看起来对”作为证据；用测试输出、快照 diff、`rg` 结果。
- 阶段结束在 §8 的“阶段状态”表打勾；未打勾不得进入下一阶段。

### 7.4 手工验收程序

一个 `run_code` 程序覆盖：

1. `glob` 找文件 → `read` 读窗口 → `page.next` 续读；
2. `grep` 搜内容 → 用 `data.path` 链到 `read`；
3. `read` 拿 `file.hash` → `edit` 带 `expected_hash`；
4. 故意用旧 hash 触发 `stale`；
5. `edit` 10 块中 1 块失败，确认 9 块写入、`status:"partial"`、失败块可读；其中 `partial` 状态已取代：已被 run-code-remediation-plan 取代（A3）。
6. `write` 新文件 → `read` 校验 hash；
7. 触发一次审批，确认审批期间其他调用不被锁死；
8. 触发一次取消，确认 `calls[].outcome` 为 `cancelled/unsettled`，且没有事后写入。

---

## 8. 阶段状态与风险

### 8.1 阶段状态

| 阶段 | 状态 | 开始 | 完成 | 证据 |
|---|---|---|---|---|
| Phase 0 基线与防回退 | 审核通过（未提交） | 2026-10-20 | 2026-10-20 | 独立审核通过；产物未提交 |
| Phase 1 单一结果契约 | 审核通过（未提交） | 2026-10-20 | 2026-10-20 | 独立审核通过；产物未提交 |
| Phase 2 身份与分页 | 审核通过（未提交） | 2026-10-20 | 2026-10-20 | 独立审核通过；产物未提交 |
| Phase 3 写入原子性 | 审核通过（未提交） | 2026-10-20 | 2026-10-20 | 独立审核通过；产物未提交 |
| Phase 4 并发/锁/取消 | 审核通过（未提交） | 2026-10-20 | 2026-10-20 | 独立审核通过；产物未提交 |
| Phase 5 清理与自动化 | 审核通过（未提交） | 2026-10-20 | 2026-10-20 | 独立审核通过；产物未提交；spill-emergency 仍红，审核认定为 direct 工具仍需的共享函数，五个工具的 code 路径不进入，非阻断。 |

### 8.2 风险与回滚

| 风险 | 影响 | 缓解 |
|---|---|---|
| 旧 transcript / session search 依赖文本 | 回放/检索回归 | 不重写历史；保留文本账本渲染；先加结构化再切读取 |
| fuzzy 移除后编辑失败率上升 | 模型需要更精确 old_string | 逐块 reason + candidates；不做自动应用；测试覆盖 no_match/multiple_matches |
| 媒体能力移除 | 图片不可见 | 已接受（D-04）；后续独立工具 |
| 多文件无跨文件回滚 | 已接受 | 多文件由程序循环；后续确有需要再加独立事务工具 |
| 大量测试依赖 `content` | 改造面扩大 | Phase 0 先建 fixture；按工具拆提交 |
| hash 计算成本 | 大文件变慢 | 读取顺带 hash；写入 hash 内容；必要时 size+mtime 预筛（不暴露） |
| 并发顺序变化 | 行为回归 | 同 key FIFO；先 read 后 write/edit 灰度 |
| 删除 direct 代码导致旧测试失败 | 构建门禁不过 | 删除清单按文件拆；每步 `cargo check` |
| per-call options 移除 | 旧 SDK 调用第二个参数会 `invalid_input` | P4-06 门禁；SDK 快照；旧 transcript 不重新执行 |
| grep 硬失败 | 常见代码片段多一次重试 | 错误里给 `literal:true` / 转义 pattern 建议；SDK 描述明确；测试覆盖 |

**总体回滚策略**：按阶段、按工具拆成本地提交；任何阶段门禁失败，回退该阶段提交，不触碰前一阶段已验收的契约。不涉及数据库 schema 变更，不需要数据迁移。

---

## 9. 变更记录

| 版本 | 日期 | 变更 | 作者 |
|---|---|---|---|
| v0.1 | 2026-10-20 | 初稿：背景、目标、目标契约、6 个阶段（Phase 0–5）、做/不做、决策登记、验收与风险 | — |
| v0.2 | 2026-10-20 | 对齐 D-03/D-04/D-05/D-06/D-09；砍媒体、fuzzy、token 预算；保留 edit 部分应用；新增 D-05a/D-06a/D-13~D-19 与 direct 遗留审计；本轮不做实现 | — |
| v0.3 | 2026-10-20 | D-09 改为删除 per-call options；D-14 改为保留正则修复 + 诊断；D-17 保留工作区外；D-06 重新讨论、新增 D-20 edit 形态；仍未做实现 | — |
| v0.4 | 2026-10-20 | D-14 改为硬失败 + `literal` + 错误建议；D-05a=partial；D-06 不做 apply_changes；D-20 保持单文件 `edits[]`、多文件程序循环；仍未做实现 | — |
| v0.5 | 2026-10-20 | 执行约束改为本文唯一规范；开始 Phase 0。不改 D-01–D-20 的结论 | — |
| v0.6 | 2026-10-20 | Phase 0 独立审核通过（产物未提交）；开始 Phase 1。不改 D-01–D-20 的结论 | — |
| v0.7 | 2026-10-20 | Phase 1 独立审核通过（产物未提交）；开始 Phase 2。不改 D-01–D-20 的结论 | — |
| v0.8 | 2026-10-20 | Phase 2 独立审核通过（产物未提交）；开始 Phase 3。不改 D-01–D-20 的结论 | — |
| v0.9 | 2026-10-20 | Phase 3 独立审核通过（产物未提交）；开始 Phase 4。不改 D-01–D-20 的结论 | — |
| v0.10 | 2026-10-20 | Phase 4 独立审核通过（产物未提交）；开始 Phase 5。不改 D-01–D-20 的结论 | — |
| v0.11 | 2026-10-20 | Phase 5 独立审核通过；spill-emergency 按审核意见保留为非阻断。不改 D-01–D-20。 | — |

---

## 附录 A：证据索引

- F-01：`src/ptc/mod.rs:284-290`、`src/tools/code.rs:916-982`、`src/session/transcript_file.rs:336`
- F-02：`src/types/result.rs:66-72`、`src/tools/code.rs:895-914`、`src/tools/lsp_feedback.rs:47`
- F-03：`src/tools/read.rs:340-356`、`src/tools/code.rs:895`
- F-04：`src/tools/edit.rs:290-317`、`src/tools/edit/feedback.rs:48-61`、`src/tools/edit.rs:65-68`
- F-05：`src/tools/read.rs:391-396`、`src/tools/glob.rs:430-435`、`src/tools/edit.rs:252`、`src/tools/write.rs:48-52`
- F-06：`src/types/result.rs:161-163`、`src/tools/read.rs:305-315`、`src/tools/grep.rs:513-515`、`src/tools/glob.rs:186-197`
- F-07：`src/tools/read.rs:576-577`、`src/tools/grep.rs:274-277`、`src/tools/glob.rs:13,271`
- F-08：`src/tools/grep.rs:464-475`
- F-09：`src/tool/trait_.rs:135`、`src/tools/code.rs:1055-1074`、`src/tool/code_manifest.rs:225-229`
- F-10：`src/tools/descriptions/grep.md`、`src/tools/descriptions/read.md`、`src/tool/code_manifest.rs:169`
- F-11：`src/tools/code.rs:692-711,550-566`、`src/tool/executor.rs:96-101`、`src/tool/trait_.rs:150-157`
- F-12：`src/ptc/calls.rs:116-135`、`src/tools/code.rs:328`
- F-13：`src/tool/executor.rs:736-771`、`src/tool/schema_validate.rs:294-300`
- F-14：`src/tools/edit/feedback.rs:54-58,209`、`src/tools/descriptions/edit.md`
- F-15：`src/tools/read.rs:576-577`
- F-16：`src/tool/authorize.rs:76`、`src/permission/matchers.rs:37-40`
- F-17：`src/tools/read.rs:16,536-560`
- F-18：`src/tools/grep.rs:112-165,368`
- F-19：`src/tools/descriptions/edit.md`、`src/tools/edit/matcher.rs`
- F-20：`src/tools/glob.rs:339-363`

## 附录 B：目标 SDK 示例（讨论用）

> **已取代：** 本示例里的 `status` / `partial` 信封已被 run-code-remediation-plan 取代（A1/A3）。

```ts
// 只示意形状，最终以 Phase 1 实现评审为准
const g = await tools.glob({ pattern: "**/*.rs", path: "src", limit: 200 });
if (g.status !== "ok") throw new Error(g.error.code);

const r = await tools.read({ path: g.data.paths[0], limit: 200 });
if (r.status !== "ok") throw new Error(r.error.code);

const e = await tools.edit({
  path: r.data.file.path,
  expected_hash: r.data.file.hash,
  edits: [
    { old_string: "fn old()", new_string: "fn new()" },
    { old_string: "fn missing()", new_string: "fn found()" },
  ],
});
// e.status 可能是 "ok" | "partial" | "error"
if (e.status === "partial") {
  // 成功的块已写入；看 e.data.blocks 里 failed 的 reason
  console.log(e.data.blocks.filter((b) => b.status === "failed"));
}

return { files: e.data?.file, applied: e.data?.applied };
```
