# WP-193 dig-runtime — agent 面审阅（只读）

**分支 / HEAD：** `feat/tool-contract-ptc` @ `bd71f8c`  
**站位：** 模型 agent 看到什么、如何纠错；少写工程债。  
**对照：** `docs/ptc/charter.md` I6–I12；`docs/ptc/run-code-remediation-plan.md`（已实施）；现行 `src/ptc/**`、`src/tools/code.rs`、`src/tools/code/render.rs`、`src/tool/{project,executor,code_manifest,outcome,registry}.rs`。

---

## 总评

**档位：B+（可用且可纠错；边界与契约大体对齐章程，剩余坑集中在「旧习惯 / 并发写 / 视口裁剪」三类模型误用上。）**

一句话：启用任一 Code 工具时 agent 只看到 `run_code`（外加 bash 等 Direct），文件工具经 `tools.*` 进程序；错面调用有明确路由文案；外层结果按「头行→失败→错误明细→诊断→账本→logs→result」自渲染 ≤64KB 且跳过 spill；JS 侧 resolve data / reject `ToolError` / 抛 `TypeError` / 封口不可捕获与 I12 一致。agent 仍易栽在：沿用 `.status` 信封习惯、用 `Promise.all` 批量写、把空 `lines`/`reason` 当空文件、以及未读外层 diagnostics 里的 LSP。

---

## §1 PTC / code 边界

### 1.1 `run_code` 何时出现

- 投影：`project_tools` 按 `Tool::surface()` 分 Direct / Code；Code 经准入后进入 `code_tools`（`src/tool/project.rs:38–119`）。
- 装配：`assemble_projected_tools` **仅当** `code_tools` 非空且有 `CodeToolHost` 时，把 `CodeTool`（名 `run_code`）追加进 direct 列表（`src/tool/registry.rs:301–331`，D9）。
- 首期 Code 集：`read` / `glob` / `grep` / `edit` / `write`（`builtin_code_surface_tools`，`registry.rs:356–363`；各工具 `surface() -> Code`）。
- **agent 体感：** 工具表里不再有独立的 `read`/`edit`…；有 `run_code` 则说明本 turn 至少绑了一个已准入的 Code 工具。准入失败工具从两面移除（只打 warn，不进 SDK）；SDK 渲染失败时 `run_code` 描述变成「unavailable…admission」（`code.rs:1117–1121`）。

**缺口（纠错难度：中）：** 无 host 时会把 code 工具 **落回 direct**（`registry.rs:318–321`，error 日志）。正常产品路径有 host；若退化发生，agent 会突然又能「直接」调 `read`，与系统提示「do not call them directly」矛盾。

### 1.2 Direct vs Code 路由拒绝文案

agent 读到的错话（`src/tool/executor.rs:837–872`）：

| 误用 | 可见文案 |
|------|----------|
| Direct 调 Code 工具（如 `read`） | `` `read` is only available as tools.read inside run_code ``（非标识符名用 `tools["…"]`） |
| Code 内调 Direct 工具 | `` `{name}` is a direct tool and is not available inside run_code `` |
| 真不存在 | `tool '{name}' not found` |

与 charter **I2** 示例一致。系统提示也补了一句：有 `run_code` 时绑定工具只能 `tools.<name>(...)`，勿直接调（`src/config/global_db/prompts/harness/tools.md:4`）。

### 1.3 投影是否让 agent 少猜

- **少猜点：** 错面调用直接告诉正确 API；`run_code` 的 `description` = 整页 SDK（PRELUDE + 各工具 TS 声明 + libs），契约写在 description 里而非分散猜。
- **schema 一句：** `code` 字段说明 resolve data / reject ToolError / throw TypeError（`code.rs:1094–1096`）。
- **仍需猜：** 准入拒绝只在日志，agent 看不到「为什么少了某个工具」；custom/MCP 恒 Direct（D3/D4），agent 须从工具表本身判断，不能假设「凡文件类都进 code」。

---

## §2 `run_code` 管线可预测性

### 2.1 预算 / 超时 / 取消 / 封口

| 机制 | agent 侧语义 | 证据 |
|------|--------------|------|
| 程序预算 | 默认 120s，clamp ≤600s；审批等待 **不计** 入预算（schema + PausingSink） | `code.rs:36–38,1098–1101`；charter D7 |
| 超时 | 不可 catch；头行 `timeout after {ms}ms · finished/total, unsettled` | `render.rs:252,237`；`code.rs:826` |
| 用户取消 | 不可 catch；可出现 `cancelled` / `cancelled after the program returned`（后者可保留 result） | `render.rs:253–256`；`code.rs:827–830` |
| 封口 I7 | 终止时立即取消在途嵌套；宽限 **3s**；仍未结束 → `unsettled` + 警示「side effects may still land. Re-read…」 | `code.rs:56,601`；`render.rs:180–183` |
| 输出硬顶 | 执行期 `output_bytes` = 1MB（console+返回值）；触顶不可 catch | `code.rs:40,596–598`；`engine.rs:655–658` |
| 内存/栈 | 不可 catch；头行 `out of memory` / `stack overflow` | `render.rs:258–261` |

### 2.2 失败种类 → 模型可见文本

结构固定（`render.rs:113–128`，对齐 D8）：

1. **头行**（± unsettled 警示）  
2. **失败段**（ToolError 的 `code: message [details]`；或 syntax 行号/源码/`^`；exception 的 ≤8 帧 stack）  
3. **错误明细**（未捕获那次 + 全部 `permission_denied`，最多 8 条）  
4. **diagnostics:**（lsp / edit_block_failed / path 风险等）  
5. **calls (N of T shown):** 账本行 `#seq tool summary state`  
6. **logs:**（优先保留尾部）  
7. **result:**（成功或「取消晚于 return」时）

头行样例（`render.rs:204–237`）：

- 成功：`run_code ok · N calls (read×… ) · K rejected (…), handled by the program`
- 未捕获 ToolError：`run_code failed: uncaught ToolError {code} from call #{seq} {tool} {summary}`
- 其它失败：`run_code failed: {syntax error|exception|timeout after …|cancelled|output limit|…} · finished/total, unsettled`
- 未捕获 TypeError：标签为 `uncaught TypeError`（`render.rs:230–231`）

| 种类 | 程序内 | 外层头行/失败段要点 |
|------|--------|---------------------|
| syntax | — | `syntax error` + line/col + 源码行 |
| exception | — | `exception` + message + stack（截帧） |
| timeout（预算） | 不可 catch | `timeout after {ms}ms` |
| ToolError 未 catch | 结束程序 | `uncaught ToolError …`；失败段保留 code/message |
| ToolError 已 catch | 程序可继续 | 头行计入 `rejected … handled by the program`；明细一般不列（permission_denied 例外） |
| TypeError 协议 | throw | 未 catch → `uncaught TypeError` |
| cancelled | 不可 catch | 见上 |
| illegal output | — | `illegal output` / `program result is not JSON` |
| unsettled | — | 头行计数 + 警示重读文件 |

### 2.3 64KB render vs spill 免除

- **落地：** `RUN_CODE_VIEW_MAX_BYTES = 65_536`；分段裁剪并写明 `… [N bytes omitted of T]`（`render.rs:16–17,77–110`）。
- **免除 spill：** `CodeTool::max_result_size = usize::MAX`（注释「外层不 spill」，`code.rs:1155–1158`）；executor 对 `is_code_transport()` **跳过** `finalize_tool_call_result` / emergency cap / spill（`executor.rs:764–771`）。仅 warn 若 content 仍 >64KB。
- **嵌套调用** 向 executor 传 `spill_threshold=0`（`code.rs:133–134`），嵌套正文不进模型，由外层 render 吃。
- **agent 体感：** 大结果不再变成 `[blob:…]` 半截；但恒 ≈64KB 上限，PRELUDE 已要求「Return a summary, not a raw bulk」（`code_manifest.rs:25`）。超大返回值在 result 段被省略标注——纠错时应缩小 return，而非再调一次指望全文。

与 I10 / remediation P1 **一致**（实施记录仅确认 direct 路径仍保留 spill 给其它工具，属预期偏离 #2）。

---

## §3 JS 契约（对照 I6–I12 / remediation）

| 契约点 | 章程 / 计划 | 现行 | 一致？ |
|--------|-------------|------|--------|
| I6 runtime 工具无感 | 输入程序+绑定+预算；输出日志/值/失败种类 | `ptc::execute`；host 经 `HostFn` | ✅ |
| I7 取消封口 | 立即取消 + grace + unsettled | engine + 3s grace | ✅ |
| I8 权限只收紧 | 不超过绑定白名单 | whitelist 绑定；多余名 TypeError | ✅ |
| I9 单 Item | 无第二份结构化副本 | 无 `@@run_code_data`；content-only | ✅ |
| I10 正交失败 + 64KB 视口 + 无 spill | 见上 | render + transport 分支 | ✅ |
| I11 前缀稳定 | description 纯函数，无 cwd/perm/depth | `description_context()` 空 cwd（`code_manifest.rs:88–95`）；SDK 快照测试存在 | ✅ |
| I12 resolve data | 成功 → data | `outcome::program_result` Ok(data)（`outcome.rs:195–198`） | ✅ |
| I12 reject ToolError | `{code,message,details?}` + tool/seq | `throw_tool_error`（`engine.rs:855–897`） | ✅ |
| I12 TypeError 协议 | 多参/非法值/过深/… | `HostError::type_error` → `throw_type`（`code.rs:631–639`；`engine.rs:845`） | ✅ |
| I12 封口不可捕获 | cancel/budget/output/mem/stack | `boundary::uncatchable`（`engine.rs:818–828`） | ✅ |
| retryable 仅 conflict/timeout | I12 / A1 | `program_result` 门控（`outcome.rs:213–217`） | ✅ |
| 诊断不进 data | I12 | diagnostics 外层文本；LSP 在 diagnostics 段 | ✅ |
| edit 部分成功 resolve | remediation | PRELUDE + edit.md：检查 `failed`/`blocks` | ✅ |
| 无 status 信封 | A1 | PRELUDE 无 `ToolResult`/`status` | ✅（旧会话习惯见 §4） |

PRELUDE 全文已写清：resolve / ToolError codes 列表 / TypeError 场景 / 不可捕获 / `allSettled` / LSP 仅外层 / `in_context_window` / glob reason / 64KB（`code_manifest.rs:16–27`）。

---

## §4 对 agent 的坑（纠错难度）

1. **旧信封习惯（高）**  
   检查 `.status` / `.data`：成功路径直接是 data，取 `.status` 得 `undefined` 或抛错；remediation 明确「不做兼容层」。PRELUDE 首句已声明新契约，但历史 transcript 里的旧示例会带偏模型。

2. **`Promise.all` 批量写（高）**  
   一次未捕获 reject → 整段程序结束并取消其它在途调用 → 部分写 `cancelled`/`unsettled`。PRELUDE 要求 `allSettled`；若忽略，外层有 unsettled 警示，但 agent 可能仍盲目重试写（副作用可能已落地）。

3. **空结果语义（中）**  
   - session 行已在上下文：`lines=[]` + `reason:"in_context_window"`（不是文件空）。  
   - glob 无匹配 / 被过滤：`no_match` / `path_excluded`。  
   易被当成「读失败」或「目录空」而改用 bash（bash 仍是 Direct，会绕过 code 账本）。

4. **LSP / edit 诊断只在外层（中）**  
   程序里 `await tools.edit(...)` 成功 resolve 时 data **无** LSP；必须读本次 `run_code` content 的 `diagnostics:`。模型若只看 return value 会漏诊断。

5. **返回 `undefined`（中）**  
   非法输出 → 外层 `illegal output`；schema/PRELUDE 已说用 `null`。常见于「忘记 return」。

6. **Direct 调文件工具（低，可自愈）**  
   路由文案明确导向 `tools.x inside run_code`；系统提示也强调。一次失败即可学会。

7. **64KB 裁剪（低–中）**  
   账本/logs/result 都会省略；纠错应依赖头行计数 + 优先保留的 error/unsettled 行，而不是假设全文在场。把大文件内容 `return` 出去会浪费视口。

8. **静默面：** 准入失败 / SDK 渲染失败导致工具从两面消失——agent **看不到诊断**，只见工具表变短或 `run_code` unavailable。属「不可从结果文案纠错」、需人类/日志。

---

## 证据索引

| 主题 | 路径 |
|------|------|
| 章程 I6–I12 / D8–D9 | `docs/ptc/charter.md` §2–§3 |
| 整改目标与已实施 | `docs/ptc/run-code-remediation-plan.md` §文首、§3、§11 |
| 投影 / run_code 注入 | `src/tool/project.rs`；`src/tool/registry.rs:301–331` |
| 路由文案 | `src/tool/executor.rs:837–872` |
| spill 免除 | `src/tool/executor.rs:764–771`；`src/tools/code.rs:1155–1158` |
| 模型视图结构 / 64KB | `src/tools/code/render.rs` |
| 执行预算与绑定 | `src/tools/code.rs:568–606,1090–1122` |
| ToolError / 封口 | `src/ptc/engine.rs:595–621,818–897` |
| program_result | `src/tool/outcome.rs:195–223` |
| SDK PRELUDE | `src/tool/code_manifest.rs:16–27` |
| 系统提示 code 指引 | `src/config/global_db/prompts/harness/tools.md` |
| 工具语义说明 | `src/tools/descriptions/{read,edit,glob,grep,write}.md` |

---

## 给中层的优先关注（不改码；标纠错难度）

1. **高 · 并发写习惯：** 真实模型是否遵循 `allSettled`；若不，unsettled 警示是否足够阻止盲目重试。  
2. **高 · 旧 `.status` 习惯：** 首轮真实会话是否仍写信封；若是，是否要在系统提示再加一句硬否定（产品取舍，本 dig 不主张改码）。  
3. **中 · LSP 可见性：** agent 是否读取外层 `diagnostics:`；失败时是否误以为 edit「完全成功」。  
4. **中 · `in_context_window` / glob reason：** 是否被误读为空文件并改走 bash。  
5. **低 · host 缺失落回 Direct：** 产品路径应不可达；若运维/测试能触发，agent 会收到与提示矛盾的工具表。

---

*本产物只读审阅，未改 `src/**` / `docs/**`，未切分支、未跑 cargo、未 commit/push。*
