# run_code 模型视图与 JS 契约整改计划

> **权威声明（执行前必读）**
>
> 执行本整改期间，**本文档是唯一执行依据**。`docs/ptc/charter.md`、`docs/ptc/code-tool-rework-plan.md` 及其他任何文档，与本文档冲突或含糊时一律以本文档为准。执行者**无需、也不应**去读它们来决定做法。执行所需的章程条款、约定和类型草案都已内联在本文档中（§4、§5、§3.3）。文中保留的少量外部文档名只作背景出处，**仅背景，非执行依据**。代码位置（`文件:行号`/函数名）是执行对象，不是参考文档，照常使用。
> 唯一例外是仓库根目录的 `AGENTS.md`：它是所有 agent 的工作契约，本文档 §6.0 已内联其中与本整改相关的条款（提交与构建门禁）。

> 状态：**已实施**（2026-10-21）。v1.1 定稿的产品决策、架构定论与 Phase 0–6 已全部落地。各阶段提交见 §6 标题下的完成标注与 §11。
> 范围：`run_code` 的模型可见文本、JS 侧结果/错误契约、封口语义、失败现场、账本；五个文件工具（read/glob/grep/edit/write）的 direct 时期冗余；与之相连的共享管线修补（UTF-8 安全切片、超时后结果）；`run_code` 人类视图删除。
> 不在范围：Session schema、ctx/提示词组装、MCP/custom、bash 系列的 code 化、新增工具能力。
> 方向原则：精力放在模型 agent 体验（模型看到的文本、SDK 契约、错误可读性）；人类 UI 最小化，按原始文本显示。

---

## 0. 结论

**模型看不到 `run_code` 结果的主因是外层 spill，不是「失败清空现场」。** `run_code` 是 direct 工具，它的 content 在 `execute_tool_call` 的 direct 分支里先过 400KB emergency cap，再过 32KB spill。超过 32KB 的内容被换成 `[blob:ID]` 加前 500 字节；blob 只在推给 UI 时还原，`llm/`、`context_pipeline/` 都不还原。渲染顺序是账本 → 诊断 → 日志 → 失败/结果 → `@@run_code_data` + 整份 data JSON，所以模型最常看到的是账本开头，失败句和返回值在尾部，被切掉了。

旧计划（v0.1）有四处根因判断需要纠正：

| 旧计划判断 | 实际 | 证据 |
|---|---|---|
| `Outcome::from_failure` 清空 logs/calls 导致失败现场为空 | `from_failure` 只用于程序还没跑起来的失败（无 tokio、校验失败、线程 panic、建 runtime 失败）。timeout/OOM/exception/cancel/syntax 走 `drive()`，logs 与 calls 都保留。现场是被 spill、渲染顺序和 UI 丢掉的 | `src/ptc/mod.rs:293-338`；`src/ptc/engine.rs:320-335`、`:1215-1221` |
| 在 render 里加 64KB 预算即可 | 64KB 大于 32KB spill 阈值，不绕过 spill 就到不了模型 | `src/tool/executor.rs:746-763`；`src/tool/output.rs:29`、`:225-239` |
| 账本聚合成一行 `read×466` | 违反 D8（逐条、有上限，条文见 §4）；会话检索回声检测依赖逐条路径 | `src/session/transcript_file.rs:336-387` |
| 保留旧 marker 的 UI 结构化解析 | 产品决策 P3：人类 UI 不做兼容，删除结构化解析 | 本计划 §2 |

另有五处硬伤，旧计划没有覆盖：

1. **UTF-8 切片 panic。** `apply_emergency_cap`（`output.rs:46`，`&content[..2000]`）、`spill_content_if_needed`（`output.rs:236`，`&body[..500]`）、`Session::truncated_tool_result`（`session.rs:2714`，`&content[..end]`）都按字节切。中文内容落在非字符边界会 panic；panic 发生在 `catch_unwind`（只包 `executor.rs:695` 的工具 future）之外。
2. **双 seq。** runtime 在 JS 线程按提交顺序发 seq（`calls.rs:66`）。host 的 `CallLedger` 在被 spawn 的 tokio 任务首次 poll 时另发一套（`code.rs:186`）。`Promise.all` 下两套错位，渲染按 seq 合并，终态和 `CancelledUnfinished` 会套到别的调用上。
3. **content 里有双份 JSON。** `render_text_from_data` 末尾追加 marker 加整份 data（`code.rs:873-877`），最坏约 2MB。
4. **成功值在收口时被丢。** 程序已返回，但 `settle` 发现预算耗尽就改判 Timeout（`engine.rs:491-493`）。`drive_queued` 在成功后报错同样改判（`engine.rs:296-299`）。之后 `value` 被设成 `None`。
5. **未捕获的错误被折叠。** 程序异常一律映射为 `Internal`（`code.rs:655`）。失败段没有 stack（`Failure.stack` 已采集但不渲染）；语法错误没有位置（`Failure::syntax` 丢 stack，`engine.rs:922`）。

JS 契约本身也偏离了 code 形态。五个文件工具已无 direct 调用方，但 `tools.x()` 仍 resolve 成 `{status, data, diagnostics, error, page}` 信封：程序不检查 `status` 就会带着 `data:null` 继续。参数改写、未知参数照跑、空 glob 放大范围、「Similar files」、「请用 bash」等 direct 时期的容错和教模型文案仍在。

**整改方向：**

- `run_code` 自己按 64KB 预算渲染模型视图，外层不再 spill 或二次截断。
- `tools.x()` 成功时 resolve 该工具的 `data`，领域失败时 reject `ToolError`，协议失败抛 `TypeError`，封口不可捕获。
- 砍掉 direct 冗余。
- 人类视图按原文显示。

---

## 1. 现状核实与证据

两份调研报告的结论已逐条对照代码核实。下表只列与计划相关的项。行号以当前工作区（未提交改动）为准，执行时允许漂移，以函数名定位。

### 1.1 模型视口与管线

| ID | 现象 | 根因 | 证据 |
|---|---|---|---|
| V-1 | 大于 32KB 的 `run_code` 结果，模型只见约 500 字节 | direct 分支对 `run_code` 执行 `finalize_tool_call_result`（emergency + spill）；LLM 路径不还原 `[blob:` | `executor.rs:746-763`；`output.rs:67-74`、`:225-239`；还原只在 `client_protocol/output.rs:12-20` |
| V-2 | 大于 400KB 时原文直接丢弃，只剩 2000 字节预览 | `apply_emergency_cap` 不落 blob | `output.rs:36-64` |
| V-3 | 中文内容触发 panic | 三处按字节切片（见 §0 第 1 条） | `output.rs:46`、`:236`；`session.rs:2714` |
| V-4 | content 带 marker 和整份 data JSON | `render_text_from_data` 追加 | `code.rs:873-877`；`outcome.rs:15` |
| V-5 | 失败句和返回值在尾部 | 渲染顺序：账本 → 诊断 → 日志 → 失败/结果 → JSON | `code.rs:784-877` |
| V-6 | SQLite 行级 spill（body JSON 超过 32KB 时写入 `body_ref`） | 读回时 `row_to_item` 透明还原，**不影响模型**。整改不要误改 | `session.rs:836-854`、`:885-897` |

### 1.2 runtime 与账本

| ID | 现象 | 证据 |
|---|---|---|
| R-1 | 双 seq：host 账本序号与 runtime 序号独立 | `calls.rs:60-84`；`engine.rs:703-723`；`code.rs:170-205`、`:432`、`:446` |
| R-2 | `LEDGER_CAP=64`，超出部分只计总数；没有按工具计数、错误码计数和 session 路径标记 | `code.rs:38`、`:186-204`、`:221-224` |
| R-3 | 超过 64 条的调用从回声检测里消失；spill 后前 64 条也只剩 500 字节 | `transcript_file.rs:336-387`；`echo.rs:111-122` |
| R-4 | 账本行只有 `error_code`，没有 message；文本账本连 code 都不印（测试锁死了这一点） | `code.rs:159-167`、`:691-694`、`:1145-1156` |
| R-5 | 语法错误没有位置；源码被包成 `(async () => {\n…\n})()`，行号偏移 1 | `engine.rs:767`、`:917-922`；`mod.rs:238-240` |
| R-6 | exception 和栈溢出的 stack 已采集但不渲染 | `engine.rs:812`、`:924`；`code.rs:620-629` |
| R-7 | 成功值被丢（见 §0 第 4 条） | `engine.rs:296-299`、`:485-493`、`:329-334` |
| R-8 | 未捕获异常折叠成 `Internal`；host 失败只能 `throw_message`（普通 Error，没有 code） | `code.rs:653-662`；`engine.rs:743`；`boundary.rs:660` |
| R-9 | runtime 日志超限：溢出的那条保留 UTF-8 前缀，随即 trip 并封口。此后程序已停止，不存在「更晚的尾部日志」。真正的问题是 render 没有说明截断 | `ptc/output.rs:31-53` |
| R-10 | console 打印对象：深度 4、每层 20 项，静默截断；Error 对象的 message 不可枚举，大概率打印成 `{}`（需探针确认） | `engine.rs:1145-1213` |
| R-11 | `failure_outcome` 与 `Outcome::from_failure` 重复 | `engine.rs:1215-1221`；`mod.rs:293-300` |
| R-12 | `CodeToolHost.spill_threshold` 传进内层，code 分支从不 spill | `code.rs:52`、`:123`；`executor.rs:746-748` |

### 1.3 JS 契约

| ID | 现象 | 证据 |
|---|---|---|
| C-1 | 所有工具失败都 resolve 信封 `{status:"error", data:null, error}`；成功也包信封 | `code.rs:610-613`；`outcome.rs:240-289` |
| C-2 | SDK 称「Runtime or protocol failures reject」，但多参数和参数嵌套过深这两种协议失败 resolve 成信封；非法 JS 值和 in-flight 超限已抛 `TypeError` | `code_manifest.rs:18`；`code.rs:411-430`；`engine.rs:677`、`:703-705` |
| C-3 | `retryable` 泛滥：`invalid_regex` true、`stale` true、`edit_failed` true | `grep.rs:368`；`edit.rs:360-363`、`:393-397` |
| C-4 | `stale` 经 `kind_for_code` 映射为 `Conflict`。若按 kind 推导 retryable，会误标为可重试 | `outcome.rs:213` |
| C-5 | `status:"partial"` 是第二个状态位，`failed` 已是 data 必填字段 | `edit.rs:442-446`、`:227`；`outcome.rs:115` |
| C-6 | 信封外层 `page` 与 `data.page` 重复 | `outcome.rs:268-270`；`read.rs:521-528` |
| C-7 | 部分只靠 diagnostics 携带的事实：glob 的 `path_excluded` 和 `empty_glob`、read 的 `in_context_window`（读当前会话的虚拟 transcript 时，已在窗口内的行返回空）。去掉信封的 diagnostics 后必须迁入 data，否则程序拿到空结果却不知原因 | `glob.rs:379-388`；`read.rs:427-439` |

### 1.4 direct 时期冗余（五个工具已无 direct 调用方）

| ID | 冗余 | 证据 |
|---|---|---|
| D-1 | 未知参数：加警告后照跑。read/grep/glob 的 schema 没有 `additionalProperties:false`（edit/write 已有） | `executor.rs:220-234`、`:745-747`；`write.rs:91`；`edit.rs:151` |
| D-2 | `start_line` 小于 1 或非整数时改从第 1 行读；`end_line` 非整数时改用默认窗口 | `read.rs:539-567`、`:594-598` |
| D-3 | 空白 `glob` 当作没传，放大搜索范围 | `grep.rs:260`、`:315-324` |
| D-4 | 含 `..` 的 glob：加警告后照跑，结果通常为空 | `glob.rs:370-378`、`:391-401` |
| D-5 | 找不到文件时扫描目录，把「Similar files」或「verify the path exists」拼进 message | `read.rs:272-283`、`:660-`；`file_path.rs:24-27` |
| D-6 | 教模型下一步的文案：目录错误「Use glob…」、工具超时「use bash run_in_background」、写锁冲突「Retry after…」 | `file_path.rs:18-21`；`executor.rs:711`、`:650` |
| D-7 | 非法正则带 `suggested_literal`/`suggested_pattern` | `grep.rs:358-371` |
| D-8 | `line_truncated` 既在字段上又推一条诊断；`read_window`、`scan_budget`、空结果原因诊断与 data 重复 | `read.rs:616-641`；`grep.rs:390-408`、`:913-916`、`:936-941` |
| D-9 | 摘要键偏好里仍有 `file_path` | `code.rs:558-566` |
| D-10 | 五个工具中有 38 处无类型的 `ToolCallResult::error(`，会被折叠成 `internal`（read 12、edit 10、write 8、glob 6、grep 2） | `rg "ToolCallResult::error\("` |
| D-11 | 过时注释「strip quietly」（行为已改为不剥） | `glob.rs:403-405` |

### 1.5 取消与超时

| ID | 现象 | 证据 |
|---|---|---|
| T-1 | 宽限 3 秒后，未结束的内层调用被标为 `CancelledUnfinished`，但 future 不中止，写仍可能落地；read/edit/write 没有 `timeout()` | `code.rs:42`；`calls.rs:112-135`；`grep.rs:230`、`glob.rs:139` 有超时，另外三个没有 |
| T-2 | **工具级超时后，即使写已提交也返回 `timeout`（retryable）。** 超时分支在 cancel 后 `let _ = tool_fut.await` 丢弃真实结果，与 `:722` 注释「tool future 返回的结果是真相」矛盾，会诱发重复写 | `executor.rs:700-717` |

### 1.6 人类 UI

| ID | 现象 | 证据 |
|---|---|---|
| U-1 | `RunCodeToolView` 解析 marker；解析成功时不显示 result 和失败句 | `web/src/components/toolviews/RunCodeToolView.tsx:36-137` |
| U-2 | 范围约定已写「不做 `run_code` 专门视图」（条文见 §4 S-1），仓库里却有 | `registry.tsx:45`、`:86` |
| U-3 | `RESOURCE_BUSY_RE` 未锚定。任意工具输出只要含 `resource busy`，整张卡片就被替换成琥珀色提示 | `web/src/components/ToolContentView.tsx:6`、`:51-65` |

---

## 2. 决策记录

> P、A、Q 三组是产品负责人的已定决策，执行中不得推翻。E-xx 是本计划在已定决策框架内做的工程裁定，可在评审中调整，但不得违背 P/A/Q 决策。本计划**没有待决项**。

### 2.1 产品决策

- **P1 模型可见预算。** `run_code` 的 content 跳过外层 32KB spill 和 400KB emergency cap，由 `run_code` 自己的 render 保证在 64KB 以内。I10 修订为「唯一硬顶 = 执行期 `output_bytes`（现 1MB）；模型视口由 render 预算保证」（最终条文见 §4）。
- **P2 被程序 catch 的内层错误。** 摘要显示计数；明细只列未捕获的那次，外加每一次 `permission_denied`。
- **P3 人类 UI。** `run_code` 优先模型体验，JS 运行时对人是黑箱，人类侧体验可忽略。删除 `@@run_code_data` marker 及 `RunCodeToolView` 的结构化解析。新旧记录一律按原始文本显示，不做旧会话兼容。可整个删掉 `RunCodeToolView`、回落通用视图时，优先删。

### 2.2 架构定论

- **A1 结果契约。** `tools.x(args)` 成功时 resolve 该工具的 `data`（分页、`line_truncated`、`file`、`applied`/`failed`、`reason` 都在 data 内）。去掉 `status`/`diagnostics` 信封、泛滥的 `error.retryable` 和外层 `page`。领域失败 reject `ToolError{code, message, details?}`。协议失败（多参数、非法 JS 值、嵌套过深、大于 2^53-1、in-flight 超限）抛 `TypeError`。`retryable` 只用于 `conflict` 和工具级 `timeout`，公共层不自动重试。PRELUDE、`code_manifest` 和生成的类型同步修改。
- **A2 封口（不可捕获）。** 用户取消、`run_code` 预算超时、输出上限、内存/栈，保持不可捕获。权限拒绝是可捕获的 `ToolError(permission_denied)`，账本必记。工具自身超时是可捕获的 `timeout`。
- **A3 部分成功的 edit。** resolve `data`，`failed` 为必填字段，不抛。
- **A4 砍掉 direct 时期冗余。** 以 §3.6 冗余审计表为准逐项写清。共享管线 `execute_tool_call` 不复制，只在 `ToolCaller::Code` 分叉上改投影。
- **A5 硬伤修复。** 包括：
  - `run_code` 跳过 spill 和 emergency cap；
  - 所有字节切片改为 UTF-8 边界安全；
  - host 账本复用 runtime 的 seq；
  - 删除 content 里的双份 JSON；
  - 失败段带 code 加 message（截断）和截断后的 stack 帧；
  - 语法错误给出 line:col（扣除包装偏移）和源码行；
  - settle 时已算出的成功值不丢；
  - 未捕获 ToolError 的 code/message 保留到外层。
- **A6 账本。** 遵守 D8：有上限地逐条列出（seq、工具、参数摘要、终态），按字节预算截断，并写明省略条数和按工具计数。上限之外仍要保留「是否读了 session 路径」，供会话检索的回声检测使用。
- **A7 diagnostics。** LSP 和 `edit_block_failed` 有界地保留在外层文本。
- **A8 宽限期后仍落地的写。** 维持 `unsettled`，文案写明副作用可能已落地、不要盲目重试；给 read/edit/write 补内部超时。
- **A9 其余中低优先级问题。** 按正确性处理，见 §6 各阶段。

### 2.3 原待决项（已定案，2026-10-20）

- **Q1 未捕获错误终止程序时的在途写。** 维持 I7：立即向在途调用发出取消，宽限 3 秒后记录终态（`cancelled` / `unsettled`），不等待它们自然结束。SDK 的 PRELUDE 明确提示：批量写用 `Promise.allSettled`，因为一次未捕获的 reject 会结束程序并取消其余在途调用。
- **Q2 code 模式读取当前会话 transcript 时，已在上下文窗口内的行。** 继续隐藏（防回声），`ReadData` 新增 `reason: "in_context_window"`，让程序知道 `lines` 为空的原因（E-6）。
- **Q3 LSP 报错不给程序。** `lsp_error` 不进入 edit/write 的 data，只出现在外层文本的诊断段（与 A7 一致），由模型在下一轮处理。不新增 `lsp_errors` 字段。

### 2.4 工程裁定

| ID | 裁定 | 理由 |
|---|---|---|
| E-1 | 跳过 spill 的判定用已有的 `Tool::is_code_transport()`，不新增 trait 方法 | 唯一实现者是 `CodeTool`（`code.rs:999`）；不按工具名分叉，符合 D3（§4） |
| E-2 | 统一一个 UTF-8 安全的前缀/后缀切片 helper（把 `ptc::output::prefix_utf8` 上提为共享函数并补 `suffix_utf8`），所有截断点复用 | 避免第二套截断实现 |
| E-3 | runtime 安装全局 `ToolError` 类（保留名，与 `libs`/`console` 同级）；`HostError` 结构化为三种 kind：普通 Error / TypeError / ToolError | runtime 仍对工具无感（I6）：它只认「host 失败带 code/details」，不认识具体工具 |
| E-4 | `ToolError` 实例额外带只读 `tool` 和 `seq` | 让模型把异常对上账本行；也是识别「未捕获的那一次」（P2）的唯一可靠来源 |
| E-5 | 去掉信封后，各 diagnostic 的去向按 §3.5：要么删除，要么迁入 data，要么只留在外层文本 | 不让任何事实只存在于被删的通道里 |
| E-6 | glob 的 data 新增 `reason?: "no_match" \| "path_excluded"`；read 的 data 新增 `reason?: "in_context_window"`（Q2） | 承接 C-7；与 grep 已有的 `reason` 同义 |
| E-7 | 工具级超时触发后，若 tool future 最终返回的不是取消结果，就返回该结果，不返回 `timeout` | 修 T-2；与 `executor.rs:722` 的既有原则一致 |
| E-8 | 删除 `run_code` 的 `data`（不再组装 JSON，不再声明 `data_schema`），渲染直接从内存中的执行报告生成 | `run_code` 是 direct 工具，Session 只存 content，data 无消费者（Phase 1 先用 rg 确认） |
| E-9 | 程序返回值已被接受后，只有用户取消会让 Outcome 带 failure，此时 value 仍保留。其他收口事件（预算恰好耗尽、排空队列时到点）降级为 warn 日志 | 修 R-7；程序已经 return，不应改判超时 |

---

## 3. 目标契约

### 3.1 分层职责

| 层 | 负责 | 不做 |
|---|---|---|
| runtime（`src/ptc/`） | 执行、预算、取消、限额；调用账本（唯一 seq）；`ToolError`/`TypeError`/不可捕获封口；失败分类，附带 stack、语法位置和未捕获 ToolError 的身份 | 不认识工具，不决定模型文本 |
| `run_code` host（`src/tools/code.rs`） | 绑定、内层调用投影（data / ToolError / TypeError）、账本元信息（摘要、终态、错误、诊断、计数、session 标记） | 不复制管线 |
| `run_code` render（新 `src/tools/code/render.rs`） | 把执行报告渲染成 64KB 以内的模型文本；分段预算、裁剪、写明省略 | 不输出 JSON 副本，不依赖外层兜底 |
| 共享管线（`executor.rs`/`output.rs`） | 校验、授权、锁、审计；direct 工具的 cap 和 spill；对 code transport 跳过 cap 和 spill | 不做 `run_code` 的语义裁剪 |
| Session / ctx | 只存外层一条 `function_call_output` 的 content | 不解析 `run_code` |
| UI | 原文显示 | 不解析结构 |

### 3.2 模型可见文本

**结构（固定顺序；没有内容的段省略）：**

1. **头行**（必有），一句话说明结局与规模：
   - 成功：`run_code ok · 466 calls (read×460, grep×6) · 2 rejected, handled by the program (not_found×2)`
   - 失败：`run_code failed: timeout after 120000ms · 412/466 calls finished, 3 unsettled`
   - 未捕获 ToolError：`run_code failed: uncaught ToolError not_found from call #12 read path=src/x.rs`
   - 失败时外层 `wire_result` 会自动加上 `Error: ` 前缀，头行措辞要与之衔接。
2. **失败段**（有失败时）：
   - message（≤1KB）；
   - 语法错误：`line L, column C` 加源码行（≤200 字符）和 caret；
   - exception 或栈溢出：≤8 帧 stack，`program.js:L` 的行号减 1；
   - 未捕获 ToolError：`code: message` 加 `details` 的单行 JSON（≤300B）。
3. **unsettled 警示**（存在 `unsettled` 调用时）：`3 calls were still running when the program stopped; their side effects may still land. Re-read the files before retrying any write.`
4. **错误明细**（P2）：只列未捕获的那次和每一次 `permission_denied`，每条 `#seq tool summary: code: message`（message ≤300B），最多 8 条，超出写省略数。被处理的拒绝只计入头行的计数。
5. **诊断段**（A7）：`diagnostics:` 后每条 `- code path: message`；`lsp_error` 附 `details.text`（≤1KB）。允许的 code 见 §3.5。最多 16 条，超出写省略数。
6. **调用账本**（A6/D8）：`calls (N of M shown):` 后每行 `{seq}. {tool} {summary} {state}`，state 不含空格：
   - `ok`；
   - `ok(9/10)`：edit 部分写入；
   - `error:{code}`；
   - `cancelled`；
   - `unsettled`。

   超出预算时末行写 `… K more calls not listed (read×400, grep×2)`。省略部分若有 session 路径读取，在末行追加固定标记 `· reads session store`（常量与回声检测共用）。
7. **logs 段**：`logs:` 后每行 `[level] message`；从尾部向前按条保留，超出时首行写 `… K earlier log lines (N bytes) omitted`。若 runtime 因输出上限截断了最后一条，在该条后写 `[output limit reached]`。
8. **result 段**：`result:` 后接内容。字符串原样输出，其他值输出 2 空格缩进的 JSON。超出预算时保留头部 2/3 和尾部 1/3，中间写 `… [N bytes omitted of M] …`。程序已返回但被用户取消时也显示 result（E-9）。

**禁止：** marker、data JSON 副本、timestamps、未说明的截断、教模型下一步的泛泛建议（unsettled 警示除外）。

**预算（常量放在 `render.rs`，可调）：**

| 段 | 上限 | 说明 |
|---|---|---|
| 总计 `RUN_CODE_VIEW_MAX_BYTES` | 65,536 | 硬上限，含所有省略说明 |
| 预留 | 512 | 省略说明和分隔符 |
| 头行 + unsettled 警示 | 1 KiB | |
| 失败段 | 4 KiB | message 1K、stack 1.5K、源码行 300B、details 300B |
| 错误明细 | 3 KiB | ≤8 条 × ≤300B message |
| 诊断段 | 4 KiB | ≤16 条 |
| 调用账本 | 8 KiB | 优先行：未捕获、拒绝、unsettled、`ok(x/y)` |
| logs | 12 KiB | 结果较短时，未用的预算让给 logs |
| result | 剩余部分 | 典型 ≥32 KiB；result 用不完时 logs 可扩展到剩余部分 |

**分配算法：** 先渲染固定段（头、失败、警示、错误、诊断、账本）。剩余预算 R = 总计 − 已用 − 预留。logs 上限 = max(12K, R − result 长度)，二者都按 UTF-8 边界切。最后断言总长 ≤ 65,536，超出就从 result 继续削减。

### 3.3 JS 侧契约

**成功：** `await tools.read(args)` 得到 `ReadData` 本身。分页（`page`）、`line_truncated`、`file`、`applied`/`failed`/`blocks`、`reason` 都在其中，没有 `status`/`error`/`diagnostics`/外层 `page`。

**领域失败（可捕获）：** reject `ToolError`。`ToolError` 是全局类，`e instanceof ToolError` 成立。字段：

- `name: "ToolError"`；
- `code`：稳定码，包括 `invalid_input`、`not_found`、`invalid_regex`、`stale`、`stale_cursor`、`conflict`、`permission_denied`、`timeout`、`io`、`unsupported`、`edit_failed`、`internal`；
- `message`：一句事实；
- `details?`：只放程序能据以分支的事实，如 `current_hash`、`position`、`regex_error`、`blocks`、`holder`；
- `retryable?: true`：只出现在 `conflict` 和工具级 `timeout` 上；
- `tool`、`seq`：只读。

**协议失败（可捕获）：** 多参数、非法 JS 值（`NaN`/`Infinity`/`BigInt`/循环引用；`undefined` 属性按 WP-206 #7 静默剥除、数组元素置 null）、参数嵌套过深、host 返回大于 2^53-1 的整数、in-flight 超限，一律抛 `TypeError`。

**封口（不可捕获）：** 用户取消、`run_code` 预算、输出上限、内存/栈。程序无法 catch，外层头行给出种类。

**权限拒绝：** 可捕获的 `ToolError(permission_denied)`；账本记 `error:permission_denied`，错误明细必列。

**部分成功的 edit：** resolve `data`。`failed > 0` 表示部分块未写入；SDK 明确要求检查 `failed`。全部失败时 reject `ToolError(edit_failed)`，`details.blocks` 给出逐块原因。

**SDK 类型草案**（替换 `code_manifest.rs:209-212` 的 `Diagnostic`/`ToolError`/`Page`/`ToolResult<T>`）：

```ts
declare class ToolError extends Error {
  readonly name: "ToolError";
  readonly code: string;
  readonly details?: unknown;
  readonly retryable?: true;
  readonly tool: string;
  readonly seq: number;
}
// 每个绑定：readonly ["read"]: (args: ReadInput) => Promise<ReadData>;
```

**PRELUDE 要点**（替换 `code_manifest.rs:18` 那一段）：

- 成功时 resolve 工具的 data；领域失败时 reject `ToolError`（列出 code）；协议错误抛 `TypeError`；取消、超时、输出上限、内存/栈会直接终止程序，不可捕获。
- 权限拒绝是可捕获的 `ToolError("permission_denied")`。
- edit 部分写入时 resolve，必须检查 `failed`。
- 批量写入用 `Promise.allSettled`：一次未捕获的 reject 会结束程序，并取消其余在途调用（Q1）。
- edit/write 的 LSP 报错不进入 data，只出现在本次运行的外层输出中（Q3）。
- read 对已在上下文窗口内的会话行返回空 `lines`，并带 `reason: "in_context_window"`（Q2）。
- 模型只看到约 64KB 的运行输出：返回摘要，不要返回原始大块数据。

### 3.4 封口语义表

| 事件 | 程序看到 | 外层头行 | 账本终态 |
|---|---|---|---|
| 工具成功 | resolve data | — | `ok` / `ok(x/y)` |
| 领域失败 | reject ToolError | 未捕获时：`uncaught ToolError {code}` | `error:{code}` |
| 权限拒绝 | reject ToolError(permission_denied) | 未捕获时同上；明细必列 | `error:permission_denied` |
| 工具级超时（工具响应了取消） | reject ToolError(timeout, retryable) | — | `error:timeout` |
| 工具级超时但工具已提交 | resolve data（E-7） | — | `ok` |
| 写锁冲突 | reject ToolError(conflict, retryable, details.holder) | — | `error:conflict` |
| 协议失败 | throw TypeError | 未捕获时：`uncaught TypeError` | 进入了 host 的：`error:invalid_input`；在 runtime 侧被拒的（非法值、in-flight 超限）：不入账本 |
| 用户取消 / 预算 / 输出上限 / 内存与栈 | 不可捕获 | `cancelled` / `timeout` / `output limit` / `out of memory`、`stack overflow` | 在途调用为 `cancelled` 或 `unsettled` |
| 宽限期后仍未结束 | — | unsettled 警示 | `unsettled` |
| 程序已返回，随后用户取消 | — | `cancelled after the program returned`，并显示 result | 同上 |

### 3.5 diagnostics 去向

| code | 来源 | 去向 |
|---|---|---|
| `unknown_parameter` | `executor.rs:220-234` | 删除；code 侧 schema 拒绝 → `ToolError(invalid_input)` |
| `read_window` | `read.rs:621` | 删除；非法窗口 → `invalid_input` |
| `line_truncated` | `read.rs:633-640`、`grep.rs:401-406` | 删除；字段保留 |
| `in_context_window` | `read.rs:435-438` | 迁入 `ReadData.reason`（E-6） |
| `scan_budget` | `grep.rs:913-916` | 删除；`page.has_more/next` 已表达 |
| grep 空结果原因 | `grep.rs:936-941` | 删除；`data.reason` 已有 |
| `pattern_parent` | `glob.rs:370-378` | 删除；`..` 模式 → `invalid_input` |
| `path_excluded`、`empty_glob` | `glob.rs:379-388` | 迁入 `GlobData.reason`（E-6） |
| `path_outside_workspace`、`sensitive_path` | `file_path.rs:30-64` | 只留在外层诊断段；JS 侧 `file.abs` 已表达工作区外 |
| `lsp_error` | `lsp_feedback.rs:41-44` | 只留在外层诊断段（A7） |
| `edit_block_failed` | `edit` 报告 | 只留在外层诊断段（A7）；JS 侧 `blocks[].reason` 已表达 |

外层诊断段只渲染 `lsp_error`、`edit_block_failed`、`path_outside_workspace`、`sensitive_path` 四种。`ToolCallResult.diagnostics` 保留为 host 内部通道。

### 3.6 冗余审计表（A4 删除清单）

| # | 项 | 位置 | 处理 |
|---|---|---|---|
| 1 | 信封 `status`/`diagnostics`/`page`/`error` | `outcome.rs::program_value`、`contract_error` | 删除，改为 `program_result() -> Result<Value, HostError>` |
| 2 | `status:"partial"` | `OutcomeStatus::Partial`、`ToolOutcome::partial`、`edit.rs:442` | 删除；edit 用 ok |
| 3 | `retryable` 泛滥 | `grep.rs:368`、`edit.rs:362`、`:396` | 改为 false；投影层按 code 字符串只放行 `conflict`/`timeout` |
| 4 | 多参数、嵌套过深 resolve 信封 | `code.rs:411-430` | 改抛 TypeError |
| 5 | 未知参数照跑 | `executor.rs:220-234`、`:746`；read/grep/glob schema | schema 补 `additionalProperties:false`；删除 `attach_unknown_param_diagnostic`；direct 侧的 warning 保留 |
| 6 | start_line/end_line 改写 | `read.rs::resolve_start_line`、`resolve_end_line` | 非法值返回 `invalid_input`，删除 warning 分支 |
| 7 | 空白 glob 放大范围 | `grep.rs::optional_string` | 空白值返回 `invalid_input` |
| 8 | `..` glob 只警告 | `glob.rs::parent_dir_pattern_warning` | 扫描前返回 `invalid_input` |
| 9 | Similar files 扫描 | `read.rs:272-283`、`suggest_similar_files`；`file_path::missing_file_hint` | 删除；message 改为 `file not found: {path}` |
| 10 | 教模型文案 | `file_path.rs:18-21`；`executor.rs:650`、`:711` | 目录错误改为一句事实；超时和冲突文案按 caller 分叉，Code 分支只给事实，冲突的 holder 放进 details；direct 原文不动 |
| 11 | 正则建议字段 | `grep.rs::invalid_regex_result` | 删除 `suggested_*`；保留 `pattern`/`regex_error`/`position` |
| 12 | 重复诊断 | §3.5 中标「删除」的各项 | 删除 |
| 13 | 摘要键 `file_path` | `code.rs::key_arg_summary` | 删除 |
| 14 | 无类型错误 | 五个工具中的 38 处 `ToolCallResult::error(` | 改为带 code 的错误（`io`/`not_found`/`invalid_input`/`unsupported`/`internal`） |
| 15 | 过时注释 | `glob.rs:403-405` | 改写 |
| 16 | run_code data JSON | `code.rs::assemble_run_data`、`CodeTool::data_schema`、`RUN_CODE_DATA_MARK` | 删除（E-8） |

---

## 4. 执行用条款终稿

> 下列条文就是本整改要满足的全部章程约束，执行与验收只对照这里。标「修订」「新增」的条文由 Phase 0 原样写入章程（§5）；标「不变」的条文与章程现文一致，这里照录，免得执行者去翻章程。每条后的「落实」说明本计划怎么满足它。

**D3（不变）custom tool 与内置同路径。** custom tool 首期保持 Direct，但架构必须朝支持它演进。投影、准入、SDK 生成、执行路径不得按工具来源（内置/custom）分支；一个非内置的测试用工具声明 `Code` 时，必须与内置工具走完全相同的路径。
- 落实：所有分叉都按 `ToolCaller::Code` 或 `Tool::is_code_transport()`，不按工具名或来源（E-1）。

**D7（修订）审批。** 与 direct 同一条权限管线，审批等待不计入程序预算。嵌套调用触发 Ask 时，经同一个 PermissionSink 弹出同一种卡片；等待期间暂停程序预算时钟；拒绝时该嵌套调用 reject 可捕获的 `ToolError(permission_denied)`，程序可 catch 后继续，外层账本和错误明细必记；等待期间用户取消，按 I7 封口。
- 落实：Phase 3 投影；Phase 2 错误明细必列 `permission_denied`。

**D8（修订）嵌套调用账本进入外层结果内容。** 外层 `run_code` 的 `function_call_output` content 依次为：头行、失败段、unsettled 警示、错误明细（未捕获的那次加全部权限拒绝）、诊断（LSP、edit 失败块、路径风险）、调用账本（按提交顺序逐条，每条含序号、工具名、关键参数摘要、终态，不含结果正文；按字节预算截断，写明省略条数和按工具计数，省略部分保留 session 读取标记）、程序日志、最终返回值。不含结构化副本。transcript、session search、compact、resume、UI 都只读这一个 Item，不另建第二份记录；审计日志只是派生物。
- 落实：§3.2；Phase 1–2。

**I5（不变）一条管线。** 嵌套调用与 direct 调用进入同一条管线：输入校验、授权、敏感路径 floor、审批、写锁、执行后的全部副作用（文件提醒、计划修订检测、LSP 反馈、检查点等，凡 direct 有的）、审计、取消、结果终结化。不存在第二条执行路径。
- 落实：只在 `ToolCaller::Code` 分叉和 `is_code_transport()` 处改投影，不新增执行路径（A4）。

**I6（不变）runtime 无感。** runtime 对工具与会话无感：输入是程序、命名绑定、预算、取消、限额；输出是日志、值、正交的失败种类、调用日志。换引擎或移到子进程不需要改工具侧。
- 落实：`ToolError`/`HostError` 是通用的「带 code 的 host 失败」，runtime 不认识工具名语义（E-3）。

**I7（修订）取消是封口。** 任何终止路径（完成、取消、超时、异常、限额）都先**立即**向所有在途嵌套调用发出取消，在宽限期内等待它们结束并记录各自终态，然后才结束程序；外层一定写出 `function_call_output`，账本中不丢条目；外层超时不覆盖已完成的内层结果。宽限期结束仍未结束的调用记为 `unsettled`，外层文本写明其副作用可能仍会落地；工具级超时后若工具已提交，以工具结果为准。
- 落实：未捕获错误也立即取消、不等在途写（Q1）；unsettled 警示（§3.2）；E-7；PRELUDE 提示 `Promise.allSettled`（§3.3）。

**I8（不变）权限只收紧。** `run_code` 不提供任何额外权限；程序可达的能力不超过绑定集合。
- 落实：所有改动都不放宽；未知参数从「照跑」变为「拒绝」。

**I10（修订）结果诚实。** 失败种类正交（语法错误、异常、超时、取消、限额、非法输出、内部错误）；跨边界的值必须可无损表示为 JSON（`NaN`、`Infinity`、`BigInt`、symbol、function、循环引用一律判非法输出，报错带路径如 `bigint at $.a.b`）。`undefined` 例外（WP-206 #7，Andy 拍板）：工具入参中值为 undefined 的属性静默剥除；返回值中 undefined 属性剥除、数组元素置 `null`、根值置 `null`，并在 head 记 `dropped undefined at a.b, c[2]`（封顶 10 条，超出写 `+N more`）；throw 的 details 仍按严格规则。执行期 `output_bytes`（现 1MB）是唯一硬顶。模型视口由 `run_code` 自己的 render 预算（64KB）保证，按段裁剪并写明省略；外层 spill 和 emergency cap 不作用于 `run_code`。未捕获的 `ToolError` 把 code 和 message 保留到外层。
- 落实：P1、A5；Phase 1–2。

**I11（不变）前缀稳定。** `run_code` 的描述（含 SDK）是该调用方 code 面的纯函数，不含权限、depth、cwd 等易变量，保证请求前缀稳定。
- 落实：SDK 改写后仍是纯函数；快照测试覆盖（Phase 3）。

**I12（新增）JS 契约。** `tools.*` 成功时 resolve 工具 data；领域失败 reject `ToolError{code, message, details?}`；协议失败抛 `TypeError`；取消、预算、输出上限、内存/栈不可捕获。`retryable` 只用于 `conflict` 与工具级 `timeout`。工具的诊断不进入 data；LSP 报错只出现在外层文本。
- 落实：§3.3；Phase 3；Q3。

**S-1（不变，范围约定）不做 `run_code` 专门视图。** 消息列表中的工具调用视图保持 raw 格式；不做 `run_code` 的专门视图。
- 落实：Phase 6 删除 `RunCodeToolView`（P3）。

---

## 5. 章程修订项（已获产品负责人批准，Phase 0 已执行）

Phase 0 把下列条文写入 `docs/ptc/charter.md`。**内容以本文档 §4 的内联条文为准**，原样替换或追加，不得另行改写；写入时若与章程其他措辞冲突，同样以本文档为准。

1. **I10**：整条替换为 §4 的 I10。
2. **D7**：整条替换为 §4 的 D7。
3. **D8**：整条替换为 §4 的 D8。
4. **I7**：整条替换为 §4 的 I7（相对现文：「发出取消」前加「立即」，末尾补 unsettled 与工具已提交两句）。
5. **I12**：作为新条目追加在 I11 之后，文本取 §4 的 I12。
6. **范围「不做」**：保持「消息列表中 `run_code` 的专门视图」一项不变（S-1），不需改字。

同时在 `docs/ptc/code-tool-rework-plan.md` 的对应位置标注「已被 run-code-remediation-plan 取代」：

- D-05a（partial 状态）被 A3 取代；
- D-14 的 `suggested_*` 和 `retryable:true` 被 A1/A4 取代；
- §2.4.1 信封契约被 A1 取代；
- §2.4.5 run_code data 被 E-8 取代；
- L-26（Similar files 迁入 details）被 A4 取代。

以上条目编号只用于 Phase 0 在旧文档里定位；条目内容**仅背景，非执行依据**。

---

## 6. 分阶段实施

> 每个阶段都能独立提交，能过 `cargo check`（改 `web/` 时还要过 `npx tsc -b`）。测试与 rg 门禁按阶段列出。vitest 不能替代构建检查。
> 顺序依据：Phase 1 解决「模型看不见」（收益最大，且后续所有阶段的验收都依赖模型能看见）；Phase 2 在 Phase 1 的 render 上补现场；Phase 3 改 JS 契约，依赖 Phase 2 的账本和失败结构；Phase 4 的冗余清理依赖 Phase 3 去掉信封后的 diagnostics 去向；Phase 5 与 2–4 互相独立，可并行；Phase 6 收尾。
> 下列提交信息仅为建议。

### 6.0 执行约定（摘自仓库 `AGENTS.md`，执行时照此办）

- **提交：** 未经确认，不得提交、不得推送远端；不得擅自 commit / push / force-push / amend。一次一事，描述用 `type(scope): 做什么`，单条信息内中英文一致。
- **提交内容：** 不混入调试代码、测试残留、临时文件、本地配置；不混入密钥、真实邮箱、内部地址、大文件、编译产物。
- **构建门禁：** 提交前和进入构建前必须过 `cargo check`；改了 `web/` 还要过 `npx tsc -b`。vitest 或单测不能替代构建检查；不得带着编译或类型错误提交。
- **架构红线：** 内核只有一套 OpenAI Responses `Item` 真相，不新增 Message/Stream 类型；权限与敏感路径只收紧不放宽；取消是封口，不是丢弃。
- **不改：** `AGENTS.md` 本身；本计划范围外的代码。

### 测试基建（Phase 1 先行）

- 把 `tests/turn_prefix_stability.rs` 里的 `RecordingProvider`、`capture_request` 和 `CapturedLlmRequest` 挪到 `tests/common/recording_provider.rs`，原测试改为引用。
- 新建 `tests/run_code_model_view.rs`：
  - 用 `build_runtime_with_provider` 建一个启用 read/grep/glob/edit/write 的 agent；
  - 脚本第 1 轮返回 `function_call run_code`，第 2 轮返回文本；
  - 断言 `captured[1].input` 中该 `call_id` 的 `function_call_output` 文本。**这是「LLM 请求侧可观察」的验收入口**，后续各阶段都往这个文件里加用例。

### Phase 0：章程与旧计划修订

**完成：** 已实施，`f59a378`。

- **前提：** 修订已获产品负责人批准，本阶段直接执行，不再另行确认。
- **改动：** 按 §5 修订 `docs/ptc/charter.md`，条文内容以本文档 §4 内联文本为准、原样写入；在 `docs/ptc/code-tool-rework-plan.md` 标注被取代的条目；把本计划状态改为「Phase 0 已执行」。
- **验收：** 章程中 I7、I10、D7、D8、I12 与本文档 §4 逐字一致；rework 计划中 `partial`、`ToolResult`、`suggested_pattern` 所在条目都带有「已取代」标注。
- **提交建议：** `docs(ptc): 修订章程 I7/I10/D7/D8、新增 I12 并标注被取代的旧契约`

### Phase 1：模型视口——run_code 自持 64KB 预算，绕过 spill，UTF-8 安全

**完成：** 已实施，`3b33184`。

**改动：**

1. **UTF-8 安全切片（E-2）**
   - 把 `src/ptc/output.rs::prefix_utf8` 上提为共享函数，放在现有共享位置（如 `src/types/` 下的文本工具；没有就新建一个模块），并新增 `suffix_utf8`；`ptc::output` 改为引用。
   - `src/tool/output.rs::apply_emergency_cap`（:46）和 `spill_content_if_needed`（:235-236）改用 `prefix_utf8`。
   - `src/session/data/sqlite/session.rs::truncated_tool_result`（:2708-2715）改用 `prefix_utf8`。
   - 全仓搜索文本字节切片：`rg "&\w+\[\.\.\w+\]" src/tool src/tools/code.rs src/session src/context_pipeline`，逐处确认按字节还是按字符；涉及用户文本的一律改用 helper。
2. **跳过 spill/emergency（E-1）**
   - `src/tool/executor.rs::execute_tool_call` 的 direct 分支（:749-763）：若 `tool.is_code_transport()`，只做 `attach_unknown_param_warning`（`run_code` 的 schema 已禁止额外字段，实际不触发）和 `wire_result`，不调用 `cap_direct_content` 与 `finalize_tool_call_result`。
   - 加一条 `tracing::warn!` 防线：content 超过 `RUN_CODE_VIEW_MAX_BYTES` 时告警（不截断）。
   - `CodeTool::max_result_size`（`code.rs:1041-1045`）的注释改为「render 保证预算，外层不 spill」。
3. **新 render（`src/tools/code/render.rs`；`code.rs` 改为 `src/tools/code/mod.rs` 目录模块，或在 `code.rs` 内建 `mod render`，二选一，保持最小改动）**
   - 定义 `RunReport`（logs、value、failure、账本快照、诊断）和 `render_model_view(&RunReport) -> String`，按 §3.2 的顺序、预算和格式实现。
   - 本阶段的失败段先用现有 `failure_text`；账本仍用现有 `LedgerLine`。
   - `render_run_output`（`code.rs:615-667`）改为构造 `RunReport` 后调用 render，不再 `.with_data`。
   - 删除 `render_text_from_data`、`assemble_run_data`、`CodeTool::data_schema`（先 `rg "\.data\b"` 确认 `run_code` 的 data 没有任何消费者，E-8）。
4. **marker 删除**
   - 删除 `src/tool/outcome.rs::RUN_CODE_DATA_MARK`。
   - 删除 `src/session/transcript_file.rs::structured_ledger_reads_sessions`（:351-370）及其在 `ledger_reads_sessions` 中的调用；文本行解析 `parse_ledger_line` 保留。账本行格式维持 `{seq}. {tool} {summary} {state}`，state 不含空格。
   - web 侧的 marker 解析暂不动：没有 marker 时自动回落为原文，Phase 6 再删。

**删除清单：** `render_text_from_data`、`assemble_run_data`、`CodeTool::data_schema`、`RUN_CODE_DATA_MARK`（Rust）、`structured_ledger_reads_sessions`。

**测试改动：**

- `src/tools/code.rs` 的 `outcome_tests`：去掉对 `render_text_from_data` 和 marker 的依赖（:1056、:1097、:1146-1155），改测 `render_model_view`；新增预算单测（各段上限、总长 ≤65,536、CJK 边界）。
- `src/tool/output.rs` 的 tests：新增 `emergency_cap_cjk_boundary_does_not_panic`、`spill_cjk_boundary_does_not_panic`（构造第 500/2000 字节落在多字节字符中间的输入）。
- `session.rs` 新增 `truncated_tool_result_cjk_boundary`。
- `src/session/transcript_file.rs` 中 marker 相关的测试（约 :770-790）删除或改写为纯文本账本。
- `src/tool/phase1_gate.rs::edit_lsp_error_reaches_outcome_and_run_code_data`：改为断言外层 content 含 `lsp_error`。
- `tests/run_code_model_view.rs` 新增（真实循环）：
  - `large_result_is_budgeted_in_the_request`：`return "A".repeat(200000) + "END_SENTINEL"`；
  - `cjk_result_does_not_panic`：`return "中".repeat(300000)`；
  - `small_result_is_verbatim`。

**验收（LLM 请求侧）：**

- [ ] 200KB 结果：`captured[1]` 中 `function_call_output` 字节数 ≤65,536；不以 `[blob:` 开头；含 `END_SENTINEL`（尾部保留）；含 `bytes omitted` 说明；头行以 `run_code ok` 开头。
- [ ] 300k 个「中」：不 panic；输出是合法 UTF-8 且 ≤65,536 字节。
- [ ] 小结果：result 原样出现，没有 JSON 副本，没有 `@@run_code_data`。
- [ ] direct 工具（如 bash）输出超过 32KB 仍然 spill，行为不变（既有 `output.rs` 测试保持绿）。
- [ ] 门禁：`cargo check`；`cargo test --lib tools::code tool::output session::transcript_file`；`cargo test --test run_code_model_view --test turn_prefix_stability --test code_tool_runtime`。
- [ ] 手工（真实模型）：用 `scripts/dev_win.ps1` 启动，让模型写一个返回 200KB 文本、末尾带哨兵串的程序，然后问它「返回值最后 20 个字符是什么、省略了多少字节」。模型能准确回答，说明尾部和省略说明都进入了上下文。

**提交建议：** `fix(code): run_code 结果按 64KB 自持预算渲染，绕过 spill 并修复 UTF-8 截断`

### Phase 2：失败现场与账本——单一 seq、有界逐条、诊断、语法位置、stack、成功值

**完成：** 已实施，`ec79e60`。

**改动：**

1. **单一 seq（A5）**
   - `src/ptc/calls.rs::Ticket` 增加 `seq: u64`。
   - `src/ptc/mod.rs::HostFn` 的签名在 `(Json, CancellationToken, BudgetPause)` 之外增加 `CallInfo { seq: u64 }`（或直接加 `u64`），`HostFn::new`/`call` 同步修改。
   - `src/ptc/engine.rs::invoke`（:703-723）把 `ticket.seq` 传给 `host.call`。
   - `src/tools/code.rs::CallLedger` 删除 `next: AtomicU64`；`begin(seq, tool, summary)` 使用传入的 seq；`invoke_binding` 的 `call_id` 用该 seq。
   - 所有 `HostFn::new` 的调用点（`src/ptc/tests.rs`、`tests/ptc_contract_harness.rs`、`tests/code_*`）机械修改。
2. **账本（A6）**
   - `CallLedger` 对**全部**调用维护 `per_tool: BTreeMap<String, usize>`、`per_error_code: BTreeMap<String, usize>`、`unsettled`/`rejected` 计数和 `session_read: bool`。`begin` 时用 `transcript_file::is_session_target`（改为 `pub(crate)`）检查 read/grep/glob 的摘要，`session_search` 恒为 true。
   - 逐条记录保留前 `LEDGER_KEEP = 256` 条，另设优先行池（错误、拒绝、unsettled、`ok(x/y)`，上限 64 条）。
   - `LedgerLine` 增加 `error_message: Option<String>`（≤300B，UTF-8 安全）和 `edit_counts: Option<(applied, total)>`（从 edit 的 data 读取）。
   - render 在 8KB 内先放优先行，再按 seq 顺序补头部，最后写 `… K more calls not listed (…)`；有 session 读取时追加标记。
   - 标记常量 `RUN_CODE_SESSION_NOTE` 定义在 `transcript_file.rs`，供 `ledger_reads_sessions` 匹配，code.rs 引用。
3. **诊断（A7）**：`aggregate_diagnostics`（`code.rs:726-753`）只收 §3.5 允许的四种 code，按 `code|path|message` 去重，最多 16 条；`lsp_error` 附 `details.text` 截断到 1KB。
4. **失败段（A5）**
   - `src/ptc/mod.rs::Failure` 增加 `location: Option<SourceLocation { line, column }>`。
   - `Failure::syntax(message, stack)` 保留 stack；`engine.rs::classify_value` 的语法分支从 stack 或 message 中解析 `program.js:L:C`，L 减 1（包装首行，`engine.rs:767`），并限制在源码行数内；超出时标注 `at end of input`。
   - render 用 `ExecuteRequest.source` 取源码行（host 侧持有 code 字符串，不必经 runtime），stack 取前 8 帧并把 `program.js:L` 的行号减 1。
   - **先加一个探针测试，确认 QuickJS-NG 的 SyntaxError/Error 的 stack 格式**（当前 `src/ptc/tests.rs:436-443` 只断言 message 非空），再写解析代码。
5. **成功值（E-9）**
   - `engine.rs::settle`（:485-494）：程序已 `Success` 时不再按预算改判 Timeout。
   - `engine.rs::drive`（:296-299）：成功后 `drive_queued` 返回的 failure 若是 Timeout/Internal，改为 push warn 日志；若是用户取消，生成 `Outcome { value: Some, failure: Cancelled }`。
   - 修改 `Outcome` 的文档注释（`mod.rs:280`）：「value 在程序已返回时保留；failure 只有 Cancelled 可与 value 并存」。
6. **unsettled 警示（A8 文案部分）**：存在 unsettled 时 render 输出 §3.2 第 3 段。
7. **重复清理（A9）**：删除 `engine.rs::failure_outcome`，改用 `Outcome::from_failure`（可见性改为 `pub(crate)`）。

**删除清单：** `CallLedger::next`、`engine.rs::failure_outcome`。

**测试改动：**

- `src/ptc/tests.rs`：
  - `syntax_error` 断言 `location == (1, 8)` 一类的精确值（按探针结果写）；
  - 新增 `exception_keeps_stack`、`success_survives_budget_expiry_at_return`；
  - 新增 `seq_is_shared_with_host`：`Promise.all` 发 10 个调用，host 按逆序延迟返回，断言 host 收到的 seq 与 runtime 的 `CallRecord.seq` 一一对应。
- `src/tools/code.rs` 的 tests：466 条账本的预算与计数；优先行在超出后仍可见；`ok(9/10)`；session 标记出现在省略行。
- `src/session/transcript_file.rs`：`ledger_reads_sessions` 能识别省略行标记。
- `tests/code_surface_phase2b.rs::cancellation_seals_in_flight_nested_calls` 断言新的 unsettled 文案。

**验收（LLM 请求侧，加到 `tests/run_code_model_view.rs`）：**

- [ ] 超时：`timeout_ms: 300` 的 `while(true){}` → 头行含 `timeout after 300ms`；有 `console.log` 时 logs 段保留尾部。
- [ ] 语法错误：`code: "const x = 1;\nreturn (;"` → 含 `line 2, column` 和该源码行。
- [ ] 异常：`throw new Error("boom")`（从第 3 行的函数里抛）→ 含 `boom` 和 `program.js:3`。
- [ ] 466 次 read：账本 ≤8KB，含 `of 466`、`read×466`；第 300 次读 `.litecode/sessions/...` 时省略行带 session 标记，且 `ledger_reads_sessions(content) == true`。
- [ ] 一次 edit 10 块失败 1 块：账本该行为 `ok(9/10)`；诊断段有 `edit_block_failed`。
- [ ] 门禁：`cargo check`；`cargo test --lib ptc tools::code session::transcript_file`；`cargo test --test run_code_model_view --test code_surface_phase2b --test ptc_contract_harness`。

**提交建议：** `fix(code): run_code 失败现场带位置与栈，账本共用 runtime 序号并按预算逐条截断`

### Phase 3：JS 结果与错误契约（A1/A2/A3/P2）

**完成：** 已实施，`1f946b5`。

**改动：**

1. **runtime 的 ToolError（E-3/E-4）**
   - `src/ptc/mod.rs::HostError` 改为 `{ kind: HostErrorKind, message }`，`HostErrorKind = Error | Type | Tool { code, details: Option<Value>, retryable: bool }`，并提供 `HostError::tool(..)`、`HostError::type_error(..)`。
   - `mod.rs::validate` 把 `ToolError` 加为保留名。
   - `engine.rs::install` 安装全局 `ToolError extends Error`（定义为只读常量，原型存进 `Protos`）。
   - `engine.rs::invoke`（:738-748）：`Err(Tool{..})` 时构造 `ToolError`，设 `code`、`details`、`retryable`（仅 true 时）、`tool`（绑定名）、`seq`（ticket.seq）后抛出；`Err(Type)` 时调用 `throw_type`。
   - `engine.rs::classify_value`：值是 `ToolError` 实例时，生成 `Failure::exception` 并附 `tool_error: Some(ToolErrorInfo { code, tool, seq, details })`（`Failure` 增加该字段）。
2. **console 格式（A9）**：`engine.rs::format_debug` 中，Error 实例输出 `{name}: {message}`，`ToolError` 追加 ` [{code}]`；数组和对象超过 20 项时追加 `… N more`；超过深度时输出 `[Object …]`。先写探针测试确认当前 `console.log(new Error("x"))` 的输出。
3. **host 投影**
   - `src/tools/code.rs::invoke_binding`：
     - 成功返回 `Ok(data)`；
     - 失败返回 `Err(HostError::tool(code, message, details, retryable))`；
     - `DecodeFail::ExtraArgument`/`Illegal` 返回 `Err(HostError::type_error(..))`，仍调用 `ledger.record`，终态记 `error:invalid_input`；
     - 绑定不存在返回 `Err(HostError::type_error(..))`。
   - `src/tool/outcome.rs`：
     - 删除 `program_value`、`contract_error`，新增 `program_result(&ToolCallResult) -> Result<Value, HostError>`；
     - code 取 `error_code`，没有时用 `error_kind_code(kind)`；
     - retryable 规则：`code ∈ {conflict, timeout} && error.retryable`，按 code 字符串判断，不按 kind（C-4）。
   - 删除 `OutcomeStatus::Partial` 和 `ToolOutcome::partial`；`edit.rs:442-446` 改为一律 `ok`。评估能否删除整个 `OutcomeStatus`（只剩 Ok/Error 时用 `error.is_some()` 代替），能删就删。
   - 删除信封 `page`：`ToolCallResult.page` 字段及其设置点（read/grep/glob 的 `ok_data(.., Some(page))`、`output.rs:96`）。先 rg 确认 direct 工具不消费它。
   - 源头的 retryable 修正：`grep.rs:368`、`edit.rs:362`、`:396` 改为 false。
4. **外层身份（A5）**：`render_run_output` 遇到 `failure.tool_error` 时，返回 `error_with_kind(kind_for_code(code), content, false)`，并设 `error_code = code`。头行为 `run_code failed: uncaught ToolError {code} from call #{seq} {tool} {summary}`，失败段写 `code: message` 和 details。普通未捕获异常维持 `Internal`，`error_code` 设为 `"exception"`。
5. **P2 渲染**：头行计数 `N rejected (code×k, …)`；未捕获时写 `1 uncaught`，否则写 `handled by the program`。错误明细只列未捕获的那次（按 seq 对账本行）和全部 `permission_denied`。
6. **SDK**
   - `src/tool/code_manifest.rs`：PRELUDE（:16-19）按 §3.3 的要点重写；类型块（:209-212）替换为 `declare class ToolError`；绑定签名（:241）改为 `Promise<{Data}>`。
   - `src/tools/descriptions/edit.md`、`grep.md`（以及 read.md、glob.md 中提到 status、partial、retryable、diagnostics 的句子）同步修改：edit 写明「部分失败会 resolve，检查 `failed`/`blocks`」，grep 写明「非法正则 reject `invalid_regex`；需要字面量时传 `literal:true`」。
   - 重新生成 `src/tool/fixtures/run_code_sdk.txt`。
   - `CodeTool::schema` 中 `code` 的描述句保持一致。

**删除清单：** `program_value`、`contract_error`、`OutcomeStatus::Partial`、`ToolOutcome::partial`、`ToolCallResult.page`、SDK 中的 `ToolResult`/`Diagnostic`/`Page` 类型、`grep` 的 `suggested_literal`/`suggested_pattern`（此项在 Phase 4 中删，也可在本阶段顺手删）。

**测试改动：**

- `src/tool/outcome.rs` 的 tests 重写为 `program_result`：成功、各错误码、retryable 规则、`stale` 不可重试。
- `src/tool/contract_matrix.rs`、`src/tool/phase1_gate.rs`、`src/tools/phase2_gate.rs`、`src/tools/phase3_gate.rs`：从断言信封改为断言 data 或 ToolError。
- `src/tools/baseline/fixtures/*.json` 中带 `retryable`/`status` 的条目按新契约重新生成，并核对差异。
- `src/ptc/tests.rs` 新增：
  - `tool_error_is_catchable_and_instanceof`；
  - `uncaught_tool_error_keeps_code`；
  - `second_argument_is_type_error`；
  - `type_error_from_host_is_catchable`；
  - `console_formats_errors`。
- `tests/code_tool_runtime.rs`、`tests/code_surface_phase1.rs` 至 `phase5.rs`、`tests/ptc_contract_harness.rs`：程序里对 `.status`/`.data` 的使用改为直接取值或 try/catch。
- `tests/turn_prefix_stability.rs`：SDK 断言改为检查 `declare class ToolError`，并确认不再出现 `ToolResult<`。

**验收（LLM 请求侧）：**

- [ ] `try { await tools.read({path:"missing.txt"}) } catch (e) { return [e instanceof ToolError, e.code, e.tool, typeof e.seq] }` → result 为 `[true,"not_found","read","number"]`；头行含 `1 rejected (not_found×1), handled by the program`；错误明细段不出现（P2）。
- [ ] 未捕获：`await tools.read({path:"missing.txt"})` → 头行含 `uncaught ToolError not_found from call #1 read path=missing.txt`；失败段含 message；外层 `ToolCallResult.error_code == "not_found"`（工具层断言）。
- [ ] 权限拒绝（测试 agent 配置 deny 规则）且程序 catch 后继续 → 程序正常返回；错误明细段列出该拒绝；账本 `error:permission_denied`。
- [ ] `await tools.read({path:"a.txt"}, {})` → `TypeError`，可被 catch；`e.name === "TypeError"`。
- [ ] edit 10 块失败 1 块 → resolve，`r.failed === 1`，`"status" in r === false`。
- [ ] SDK 文本（`captured[0].tools` 中 run_code 的 description）不含 `ToolResult<`、`status: "partial"`、`retryable: boolean`；含 `class ToolError`；不同 cwd 下快照哈希一致。
- [ ] 门禁：`cargo check`；`cargo test --lib`（受影响模块）；`cargo test --test run_code_model_view --test code_tool_runtime --test ptc_contract_harness --test turn_prefix_stability` 以及 `code_surface_phase*`。
- [ ] 手工（真实模型）：给模型新 SDK，让它批量读 5 个文件（其中 1 个不存在）并汇总。观察它用 try/catch 或 `allSettled`，没有去检查 `.status`。

**提交建议：** `feat(code): tools.* 成功时 resolve data，失败 reject ToolError，协议错误抛 TypeError`

### Phase 4：砍掉 direct 时期冗余（A4）

**完成：** 已实施，`60e4527`。

**改动**（按 §3.6，每项都写明位置）：

1. **未知参数**：`src/tools/read.rs`、`grep.rs`、`glob.rs` 的 `schema()` 补 `"additionalProperties": false`；删除 `executor.rs::attach_unknown_param_diagnostic`（:220-234）及其调用（:747）。确认 `check_tool_input`（:547）会拒绝，message 点名多余字段，例如 `unknown parameter 'file_path'`；不点名就补上。
2. **read 窗口**：`resolve_start_line` 和 `resolve_end_line` 遇到非法值返回 `Err(invalid_input)`；删除 `start_warning`/`window_warning` 的传递链和 `attach_read_signals` 的 `read_window` 分支。
3. **重复诊断**：删除 `read.rs::attach_read_signals` 的 `line_truncated` 分支（整个函数若只剩空壳就删）、`grep.rs::attach_line_truncation`、`grep.rs` 的 `scan_budget`（:913-916），以及 `empty_result` 中把 note 包成 diagnostic 的分支（:936-941）。
4. **空白参数**：`grep.rs::optional_string` 遇到空白字符串返回 `invalid_input`（`parameter 'glob' must not be blank`）；更新 grep 测试中「空白 glob 当没传」的用例（约 :1674）。
5. **glob**：
   - `parent_dir_pattern_warning` 改为扫描前的 `invalid_input`：`pattern '{p}' reaches outside the search directory`；
   - 删除 `pattern_parent` 诊断；
   - `path_excluded` 和 `empty_glob` 改为 `GlobData.reason`（E-6），同步 `data_schema`；
   - 不再使用 `empty_discovery_hint`（direct 侧若无其他调用方，一并删除）；
   - 改写 :403-405 的注释。
6. **read**：
   - 不存在的文件：message 改为 `file not found: {path}`，删除 `suggest_similar_files` 和其测试（:1059-）、`file_path::missing_file_hint`；
   - `directory_not_file_message` 改为 `{path} is a directory, not a file`；
   - `in_context_window` 改为 `ReadData.reason`（E-6），同步 `data_schema`。
7. **grep 正则**：`invalid_regex_result` 删除 `suggested_literal` 和 `suggested_pattern`。
8. **教模型文案按 caller 分叉**（`executor.rs`）：
   - 写锁冲突（:648-660）：Code 分支 message 为 `resource busy: held by session {holder}`，并设 `error_details = {holder}`；
   - 工具超时（:708-715）：Code 分支为 `tool '{name}' timed out after {budget}`；
   - direct 原文不动。
9. **摘要键**：`code.rs::key_arg_summary` 删除 `"file_path"`。
10. **无类型错误**：五个工具里的 38 处 `ToolCallResult::error(` 改为带 code 的错误：
    - IO 和读写失败用 `io`；
    - 参数问题用 `invalid_input`；
    - 越界（如 `start_line exceeds file length`）用 `invalid_input`；
    - 解码失败用 `unsupported`；
    - 其余用 `internal`。

**删除清单：** `attach_unknown_param_diagnostic`、`read_window` 链、`attach_line_truncation`、`scan_budget`、`parent_dir_pattern_warning` 的 warning 路径、`suggest_similar_files`、`missing_file_hint`、`suggested_*`、`key_arg_summary` 中的 `file_path`。

**rg 门禁**（Phase 4 结束时全为 0，测试代码除外）：

- `rg "ToolCallResult::error\(" src/tools/read.rs src/tools/grep.rs src/tools/glob.rs src/tools/edit.rs src/tools/write.rs`（只数非测试部分）
- `rg "suggest_similar_files|missing_file_hint|suggested_pattern|suggested_literal|attach_unknown_param_diagnostic|\"read_window\"|\"scan_budget\"|\"pattern_parent\"|\"empty_glob\"" src`
- `rg "Similar files|verify the path exists|Use glob to list" src/tools`

**测试改动：** read、grep、glob、edit、write 的单测按新行为更新；`contract_matrix.rs` 增加以下行：`start_line:0`、空白 glob、`..` glob、未知参数、glob 的 `path_excluded` reason、read 的 `in_context_window` reason。

**验收（LLM 请求侧）：**

- [ ] `tools.read({path:"a.txt", start_line:0})` → `invalid_input`，message 指出 `start_line`。
- [ ] `tools.read({path:"a.txt", file_path:"a.txt"})` → `invalid_input`，message 点名 `file_path`；文件没有被读取（账本为 `error:invalid_input`）。
- [ ] `tools.glob({pattern:"../*"})` → `invalid_input`；`tools.grep({pattern:"x", glob:"  "})` → `invalid_input`。
- [ ] `tools.read({path:"nope.rs"})` 的 message 中没有 `Similar files`，也没有 `verify`。
- [ ] 门禁：`cargo check`；`cargo test --lib tools`；`cargo test --test run_code_model_view` 以及 `code_surface_phase*`。

**提交建议：** `refactor(code): 删除五个文件工具的 direct 期容错与教模型文案`

### Phase 5：取消与超时诚实（A8，与 Phase 2–4 互相独立）

**完成：** 已实施，`65b795c`。

**改动：**

1. **超时后以工具结果为准（E-7）**：`executor.rs` 超时分支（:700-717）取 `let late = tool_fut.await`。若 `late` 是 `Ok(r)` 且 `r` 不是 `Cancelled` 错误，返回 `r`，可附 `tracing::warn!("completed after timeout")`；否则返回 `Timeout` 错误（retryable）。该修改对 direct 和 code 一致。
2. **补内部超时**：
   - `read.rs`、`edit.rs`、`write.rs` 实现 `fn timeout() -> Some(Duration::from_secs(30))`（常量各自定义，与 grep/glob 一致）；
   - 确认 write 在写入前检查 `execution.cancel`，没有就补；
   - read 在逐块读取大文件时检查 cancel；
   - 同步磁盘 IO 无法中断，这一点作为已知限制写进风险表。
3. **unsettled 文案**：若 Phase 2 已完成，确认警示文案与 §3.2 一致；否则在本阶段补上。

**测试改动：**

- `executor.rs` 新增 `committed_result_wins_over_tool_timeout`：测试工具忽略取消，超时后返回 ok → 断言结果为 ok，不是 Timeout。
- 新增 read/edit/write 声明超时的断言。
- `tests/code_surface_phase2b.rs` 用测试专用的 Code 工具（D3 允许）在宽限期外才完成 → 断言外层文本含 `may still land` 和 `unsettled`。

**验收：**

- [ ] 真实循环：慢写工具超过宽限期 → 请求侧文本含 unsettled 警示；账本该行 `unsettled`；没有 `retryable`。
- [ ] 超时后提交的写：程序拿到 data，而不是 `timeout`。
- [ ] 门禁：`cargo check`；`cargo test --lib tool::executor tools`；`cargo test --test code_surface_phase2b --test run_code_model_view`。

**提交建议：** `fix(tool): 工具超时后以已提交结果为准，并为 read/edit/write 补内部超时`

### Phase 6：人类 UI 最小化与收尾（P3、A9 余项）

**完成：** 已实施，`387ed9c`。

**改动：**

1. **web**
   - 删除 `web/src/components/toolviews/RunCodeToolView.tsx` 和 `RunCodeToolView.test.tsx`；
   - 从 `registry.tsx` 的 `TOOL_VIEWS`（:45）和 `OUTPUT_OWNED_BY_VIEW`（:86）中移除 `run_code`；
   - `paramMeta.ts:19` 的 `run_code` 项若再无引用就删除；
   - `ToolContentView.tsx`：`detectResourceBusy` 对 `name === "run_code"` 不生效（U-3）。

   删除后通用视图把 input 显示为 JSON 字符串，代码中的换行显示成 `\n`。按 P3 接受。
2. **Rust 收尾**
   - 删除 `CodeToolHost.spill_threshold`（`code.rs:52`、`:68`、`:78`），`run_nested` 向 `execute_tool_call` 传 0；同步修改 `src/runtime/mod.rs` 中构造 `CodeToolHost::new` 的调用点（约 :1008、:1046）。
   - 复查 §3.6 全表，以及 Phase 1–5 的删除清单是否有残留。

**最终 rg 门禁（全为 0）：**

- `rg "@@run_code_data|RUN_CODE_DATA_MARK|RunCodeToolView|parseRunCodeLedger" src web/src`
- `rg "program_value|OutcomeStatus::Partial|ToolOutcome::partial|ToolResult<|failure_outcome|structured_ledger_reads_sessions" src`
- `rg "&content\[\.\.EMERGENCY|&body\[\.\.preview_len\]|&content\[\.\.end\]" src`
- `rg "spill_threshold" src/tools/code.rs`

**验收：**

- [ ] `npx tsc -b`（`web/`）通过；`cargo check` 通过。
- [ ] 手工：新旧会话的 run_code 卡片都按原文显示，不崩溃；旧会话卡片里的 marker JSON 原样出现，按 P3 接受。
- [ ] 手工：run_code 输出中含 `resource busy` 时，卡片仍显示原文。

**提交建议：** `refactor(web): 删除 run_code 专门视图，按原文显示`

---

## 7. 验收矩阵（汇总）

| 场景 | 模型看到（请求侧） | 程序看到 | 阶段 |
|---|---|---|---|
| 小结果 | 头行加 result 原文；没有 JSON 副本 | — | 1 |
| 200KB 结果 | ≤64KB；保留头尾；写明省略字节；不出现 `[blob:` | — | 1 |
| CJK 超大结果 | 不 panic；合法 UTF-8 | — | 1 |
| 超时 | `failed: timeout`；完成数；logs 尾部 | 不可捕获 | 2 |
| 语法错误 | `line:col` 加源码行 | — | 2 |
| 异常 | message 加 ≤8 帧 stack | — | 2 |
| 466 次调用 | 逐条 ≤8KB；省略数；按工具计数；session 标记 | — | 2 |
| `Promise.all` 并发 | 账本 seq 与调用一一对应 | — | 2 |
| 返回后用户取消 | `cancelled after the program returned` 加 result | — | 2 |
| 内层失败被 catch | 头行计数，不列明细 | `ToolError`（code/tool/seq） | 3 |
| 内层失败未被 catch | 头行和失败段带 code/message；外层 `error_code` | — | 3 |
| 权限拒绝 | 明细必列 | 可捕获 `ToolError(permission_denied)` | 3 |
| 多参数 / 非法值 | 未捕获时为 `TypeError` | `TypeError` | 3 |
| edit 部分成功 | 账本 `ok(9/10)`；`edit_block_failed` 诊断 | resolve，`failed=1` | 2/3 |
| LSP 错误 | 诊断段 `lsp_error` 加正文 ≤1KB | data 中没有 | 2 |
| 未知参数 / `start_line:0` / 空白 glob / `..` glob | 账本 `error:invalid_input` | `ToolError(invalid_input)` | 4 |
| 宽限期后写落地 | unsettled 警示 | 不可捕获（程序已终止） | 2/5 |
| 工具超时但已提交 | 账本 `ok` | resolve data | 5 |
| 人类视图 | 原文 | — | 6 |

---

## 8. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 一次未捕获的 reject 结束程序，其余在途写被 I7 取消（`Promise.all` 批量写尤其常见） | 部分写为 `cancelled`/`unsettled` | SDK 明确要求批量写用 `allSettled`；账本和 unsettled 警示可见；I7 维持立即取消（Q1），不为此延迟封口 |
| 模型沿用旧 SDK 习惯检查 `.status`（历史会话里有旧写法） | 首次运行即抛错 | 预期行为，不做兼容层；PRELUDE 首句写明新契约 |
| 每次 `run_code` 最多 64KB（约 16k token）进上下文，旧行为（被截成 500B）虽然错误但更省 | 上下文增长更快 | 依赖既有压缩；PRELUDE 引导返回摘要；预算常量可调 |
| QuickJS-NG 的 stack 和语法错误格式未经实测 | 位置解析写错 | Phase 2、3 先写探针测试，再写解析代码 |
| `HostFn` 签名变更波及多处测试 | 改动面大 | 机械修改；Phase 2 一次完成 |
| 删除 `ToolCallResult.page` 或 `OutcomeStatus` 撞上未知消费者 | 编译失败 | 删除前先 rg；有 direct 消费者就只删 code 投影 |
| 同步磁盘 IO 无法被超时中断 | 极端情况下锁被长期持有 | 已知限制；unsettled 文案兜底 |
| 旧会话：marker JSON 原样显示；旧的 spill 记录回声检测仍不完整 | 人类观感差；旧数据检索有损 | P3 接受；新数据不再 spill |
| 去掉 `edit_failed` 和 `invalid_regex` 的 retryable | 依赖该标志的旧提示失效 | 公共层本来就不重试；模型改由 code 和 details 判断 |

---

## 9. 待决

无待决项。原三项已定案，见 §2.3 的 Q1–Q3。执行中若遇到本文档未覆盖的产品取舍，停下来向产品负责人确认，不得自行引入新契约字段或改动 §4 条文。

---

## 10. 变更记录

| 版本 | 日期 | 变更 |
|---|---|---|
| v0.1 | 2026-10-20 | 初稿：一个持久化权威；run_code 自渲染；分 Phase 0–3 |
| v1.0 | 2026-10-20 | 重写：纠正根因（外层 spill 是主因，`from_failure` 不是）；纳入产品决策 P1–P3 与架构定论 A1–A9；新增 UTF-8 切片、双 seq、成功值丢失、工具超时后结果、diagnostics 去向、`RESOURCE_BUSY_RE` 等核实结果；目标契约（模型文本结构与预算、JS 契约、封口表）；章程修订项；Phase 0–6 实施步骤与请求侧验收；待决 3 项 |
| v1.1 | 2026-10-20 | 定稿：文首加权威声明（本文档为唯一执行依据）；原待决三项定案为 Q1–Q3 并移入决策记录；§4 改为执行用条款终稿，内联 D3/D7/D8/I5–I8/I10–I12/S-1 全文；§5 标为已批准、待 Phase 0 执行，修订内容以 §4 为准；新增 §6.0 执行约定（提交与构建门禁）；去掉执行层面对外部文档的依赖 |
| v1.2 | 2026-10-21 | 标记已实施：各 Phase 完成状态与提交；文末实施记录记下审核确认的偏离 |

---

## 11. 实施记录

2026-10-21 独立审核确认的偏离（相对本文档字面，语义已接受）：

1. **5 处 `@@run_code_data` 否定断言保留。** rg 门禁仍命中 `src/tools/code.rs` 两处、`src/tools/code/render.rs`、`src/tool/contract_matrix.rs`、`src/tool/phase1_gate.rs`。这些断言锁定输出不含 marker。生产代码已无该常量、解析器与 `RunCodeToolView`。
2. **direct 路径保留 `spill_threshold`。** `RuntimeContext` 继续按 direct 阈值 spill。`CodeToolHost` 已删除该字段，嵌套调用向 `execute_tool_call` 传 0。计划里约 `runtime/mod.rs:1046` 是 direct 构造点。
3. **`present_returned_value` 改名。** 渲染辅助函数原名 `present_program_value` 含已删符号 `program_value` 的子串，改为 `present_returned_value`。该函数继续负责把程序返回值写入模型视图。
4. **SDK 否定断言收为 `ToolResult`。** 生成文本的断言由不含 `ToolResult<` 改为不含 `ToolResult`。该类型本来就不在生成文本里。
