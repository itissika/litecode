# WP-193 dig-tools — 文件工具 agent 面（read / glob / grep / edit / write）

**分支：** `feat/tool-contract-ptc`（`/workspace/litecode`）  
**角度：** 模型 agent 易理解 / 易用 / 易排错（少写工程细节）  
**对照：** `docs/ptc/code-tool-rework-plan.md`、`docs/ptc/run-code-remediation-plan.md`（A1/A4 已落地）、现行 `src/tools/{read,glob,grep,edit,write}*` + `descriptions/*.md` + SDK  
**性质：** 只读审阅；未改产品码、未 cargo、未提交

---

## 总评

五个文件工具在 **code 原生契约**上已大体到位：成功 resolve 裸 `data`、领域失败 reject `ToolError`、非法输入多数硬失败、分页/`FileRef`/`reason`/`edit.failed` 可编排。对 agent 仍有几处 **文案与运行时不一致** 和 **易漏检的半成功**，会教错用法或让静默半写入难发现。整体：**可用、可链式，但描述层仍带 direct 腔调**；**非硬 blocker**，建议作为文案/SDK 对齐小包跟进。

**档位：** B+（契约硬面好，文案投影有漂移）

---

## 1) 描述文案（descriptions + SDK）

**结论：** 五份 `descriptions/*.md` 与 SDK 绑定注释已按 remediation A1/A4 改成「reject `ToolError` / resolve 部分失败 / 无 token 预算 / 无 fuzzy」口径，**不再教** `grep.view`、markdown 带行号正文、静默修正则等 direct 用法；但仍有两处会 **教错程序写法**。

### 已对齐 code 形态（好）

- read：结构化 `file`/`lines`/`page`；非法窗口 `invalid_input`；图片/二进制 `unsupported`；续页用 `page.next`→`start_line`；无 cursor、无 token 预算。证据：`src/tools/descriptions/read.md:1-3`。
- glob/grep：统一 `base` + `page` + `reason`；非法 `..` / 空白 glob / 非法 regex 硬失败；cursor 绑定查询。证据：`descriptions/glob.md:1-3`、`descriptions/grep.md:1-5`。
- edit/write：精确匹配、部分失败 resolve 并检查 `failed`/`blocks`、全失败 `edit_failed`、`expected_hash`→`stale`。证据：`descriptions/edit.md:1-3`、`descriptions/write.md:1`。
- SDK PRELUDE 明确：成功 resolve 工具 data；领域失败 `ToolError`；协议失败 `TypeError`；edit 部分写入必须查 `failed`；批量用 `Promise.allSettled`。证据：`src/tool/fixtures/run_code_sdk.txt:3-10`。

### 仍像「信封时代」/ 教错用法（风险）

| 问题 | 为何伤 agent | 证据 |
|---|---|---|
| 五工具描述与 SDK 绑定注释一律写 `data.file` / `data.lines` / `data.page`… | A1 下程序拿到的是 **裸 data**（`const r = await tools.read(...)` 后应是 `r.file`）。写 `r.data.file` 会得到 `undefined`，难排错 | `descriptions/read.md:1`；`run_code_sdk.txt:278-301`；对照 `outcome.rs::program_result`（成功返回 `result.data` 本体，`:195-198`）与 PRELUDE「resolves to the tool's data」（`run_code_sdk.txt:3`） |
| edit SDK：`Do not include read's line-number prefix` | code 面 `lines[].text` **本无**行号前缀（`run_code_sdk.txt:237`）；这是 direct 时代习惯残留。会诱导模型以为 read 仍吐带前缀文本，或反过来在外层账本文稿里抄行号再 edit | `run_code_sdk.txt:85`；matcher 仍 reject `read_line_prefix`：`edit/matcher.rs:73-75,104,122-123` |
| 路径 schema 提示统一塞 Windows/Git-for-Windows 方言 | Unix agent 噪音大；易误以为必须写盘符。语义本身正确但偏人类/跨平台说明书，不像 code SDK 的最小事实 | `workspace/tool_path.rs:16`（`AGENT_FILE_PATH_HINT`）；落入 `run_code_sdk.txt:90,211,255` |
| write 的 `content` schema 描述仍是「Content to write to the file」 | 相对其它字段偏口语/direct，信息量低（小问题） | `run_code_sdk.txt:253` |

**相对计划：** rework F-10（描述仍像 direct）在能力面上已修；remediation Phase 3 要求 descriptions 同步 A1，但 **「data.」前缀** 与「resolve 裸 data」未对齐，属文案投影漂移。

---

## 2) 成功 / 失败契约

**结论：** 与 A1/A3/A4 目标契约基本一致；程序可按 `code` 分支；edit 半成功靠 data 内字段表达，不再有 `status:"partial"` 信封。

### 成功形状（程序侧）

- `await tools.x(args)` → 该工具的 data（无外层 `status` / `diagnostics` / `error` / 外层 `page`）。证据：`src/tool/outcome.rs:191-198`；SDK 签名 `Promise<ReadData>` 等：`run_code_sdk.txt:284-302`。
- **Page** 在 data 内：`{ has_more, next, total? }`。read 的 `next` 是下一行号字符串；glob/grep 的 `next` 是绑定查询的不透明 cursor。证据：`read.rs:521-528`；`glob.rs:370-384`；`descriptions/grep.md:5`。
- **FileRef**：`path` / `hash` / `bytes`，区外另有 `abs`。证据：`run_code_sdk.txt:131-137,224-229`；`file_identity.rs`（D-17）。
- **reason**：read `in_context_window`；glob `no_match`|`path_excluded`；grep 多枚空结果原因。证据：`read.rs:142-144,451`；`glob.rs:84-87,371-384`；`descriptions/grep.md:5`。
- **edit 部分成功**：resolve data，`applied`/`failed`/`blocks[]` 必填；全失败才 reject `edit_failed` + `details.blocks`。证据：`edit.rs:410-460`；`descriptions/edit.md:1`；A3（remediation §2.2）。

### 失败形状

- 领域失败：`ToolError{ code, message, details?, retryable?, tool, seq }`。稳定码含 `invalid_input` / `not_found` / `invalid_regex` / `stale` / `stale_cursor` / `conflict` / `permission_denied` / `timeout` / `io` / `unsupported` / `edit_failed` / `internal`。证据：`run_code_sdk.txt:3,15-21`。
- `retryable` 仅 `conflict`|`timeout`（投影层按 code 字符串放行）。证据：`outcome.rs:193-217`；门禁测试 `:267-279`。
- 非法 regex：**无** `suggested_literal`/`suggested_pattern`（A4 已砍）。证据：`grep.rs:1345-1346`；`contract_matrix.rs:506-507`。
- 与 tools.* 直出：host 用 `program_result` 把 `ToolCallResult` 投影为 data 或 `HostError`→JS `ToolError`，与描述中的 reject 语义一致。证据：`outcome.rs:195-223`。

### 残留不一致（小）

- 描述层仍说「The result is `data.xxx`」，与程序运行时字段路径不一致（见 §1）——契约硬面正确，**教法**偏信封。
- LSP / 区外路径风险：只进外层 run_code 文本诊断，不进 edit/write data（Q3/A7，有意为之）；程序若只看 resolve 值会「听不见」LSP。证据：`descriptions/write.md:1`；`run_code_sdk.txt:7`。

---

## 3) 与 run_code 配合

**结论：** read→edit 链式、分页续读、非法输入硬失败等主路径可预测；半成功与同批链式 edit 是主要坑。

### 链式 read → edit / write

1. `const r = await tools.read({ path })` → 用 `r.file.hash` 作 `expected_hash`，用 `r.lines[].text`（无行号前缀）拼 `old_string`。
2. `await tools.edit({ path, expected_hash, edits })`；hash 变则 `stale` + `details.current_hash`，不写盘。
3. 多文件 = 多次调用（D-06/D-20）；应用 `Promise.allSettled`，避免一次未捕获 reject 取消在途写（Q1 / PRELUDE）。

证据：`descriptions/read.md:3`；`descriptions/edit.md:1-3`；`edit/planner.rs:6`（`SNAPSHOT_RULE`）；`run_code_sdk.txt:5-6,281`。

### 分页续读

| 工具 | 续法 | 证据 |
|---|---|---|
| read | `page.has_more` 时把 `page.next` 原样作下次 `start_line`（行号字符串） | `descriptions/read.md:3`；`read.rs:521-522` |
| glob / grep | `page.next` 作下次 `cursor`；换查询或语料变 → `stale_cursor` | `descriptions/glob.md:3`；`descriptions/grep.md:5` |

**形状统一、消费路径不统一**：同一 `Page.next`，read 与 glob/grep 语义不同。Agent 若写通用 `continue(page)` 易把行号当 cursor 或反之。

### 非法输入：硬失败 vs 静默改写

A4 清单在现行代码上 **大体已落地**：

| 项 | 现行 | 证据 |
|---|---|---|
| 未知参数 | schema `additionalProperties:false`；校验阶段拒绝 | `read.rs:80`、`glob.rs:56`、`grep.rs:115`；`executor.rs:531` |
| start_line/end_line 非法 | `invalid_input`，不再改写到 1 | `read.rs:639-646,649-666` |
| 空白 glob | `invalid_input` | `grep.rs:1537-1544` |
| pattern 含 `..` | `invalid_input` | `glob.rs:737-744` |
| 非法 regex | `invalid_regex`，无自动 Literal/Brace 修复、无 suggested_* | D-14 被 A1/A4 取代；`grep` 测试 `:1345-1346` |
| 写锁 / 工具超时（Code 调用方） | 事实句，不教 bash | `executor.rs:634-641,716-721` |

**仍保留、但非静默成功的「软匹配」：** edit 精确匹配下 CRLF↔LF 等价、保留 BOM（有文档）。证据：`descriptions/edit.md:3`。这是声明过的归一，不是静默改写 pattern。

---

## 4) agent 坑（易理解 / 易排错）

按伤害优先级：

1. **`data.` 文案 vs 裸 resolve（高）**  
   模型按描述写 `r.data.lines` → `undefined`，以为工具坏了。PRELUDE 与类型签名是对的，**长描述段落是错的**。见 §1。

2. **edit 部分成功未查 `failed`（高）**  
   resolve 即「看起来成功」，文件已半改；外层账本有 `ok(x/y)`，但程序若 `return 'done'` 会丢块级原因。必须读 `failed`/`blocks[].reason`。证据：`edit.rs:457-460`；`run_code_sdk.txt:5`；remediation 账本态 `ok(x/y)`（§3.2）。

3. **同 call 内链式 old→new（中高）**  
   所有块对 **同一快照** 匹配；后块依赖前块 `new_string` 会 `chained` 失败。须拆成两次 edit。证据：`SNAPSHOT_RULE` `edit/planner.rs:6`；注入 `descriptions/edit.md:4`。

4. **分页 `next` 方言（中）**  
   read：`start_line`；glob/grep：`cursor`。统一 `Page` 名掩盖了不同消费。见 §3。

5. **路径方言残留（中低）**  
   - 工作区内 glob 结果已是 **workspace 相对且带 search 前缀**（`path:"src"` → `src/main.rs`），可直接 read——F-05「glob 相对 path、read 相对根」旧坑已修。证据：`glob.rs:501-524`；测试期望 `src/main.rs`（`:567-568`）。  
   - 仍须看 `base`：`absolute` 时路径是绝对路径，原样回传。  
   - schema 里 Windows 提示对非 Windows agent 干扰（§1）。

6. **read_line_prefix / 「不要带行号前缀」（中低）**  
   code read 已无前缀；若模型从 **外层账本/旧记忆** 抄带行号文本进 `old_string`，会硬失败 `read_line_prefix`。防护有价值，文案却像在暗示 read 仍产出前缀。证据：`matcher.rs:104,122-123`；`run_code_sdk.txt:85,237`。

7. **教 bash / 下一步文案（低，code 路径已清）**  
   Code 调用方超时/写锁不再追加「use bash run_in_background」「Retry after…」；direct 仍保留。五个文件工具走 Code surface，agent 在 run_code 内应只见事实句。证据：`executor.rs:634-641,716-721`。  
   A4 rg 门禁目标（Similar files / Use glob…）在 tools 侧已不见命中。

8. **空结果有 reason、仍易误判（低）**  
   `in_context_window` / `path_excluded` / `no_match` 已进 data；程序若只判断 `lines.length===0` 或 `paths.length===0` 而不读 `reason`，会空转重试。证据：`read.rs:451`；`glob.rs:371-384`。

9. **难纠错点**  
   - `stale` / `stale_cursor`：须重读 hash 或重开查询，不能盲重试同 cursor。  
   - `multiple_matches`：半成功时仅该块失败，其它块已落盘——纠错前应再 read。  
   - 未捕获 `ToolError` 会结束程序并取消在途写（宽限后可能 `unsettled`）；批量写必须 `allSettled` + 写前再读。

---

## 附录：对照计划速查

| 计划项 | agent 相关现状 |
|---|---|
| rework D-03 data-only | 是；程序见裸 data |
| rework D-05/D-05a 部分应用；A3 取代 partial status | resolve + `failed`；无 `status:"partial"` |
| rework D-14 硬失败 regex；A1/A4 砍 suggested_* / 泛滥 retryable | 已落地 |
| rework F-05 路径方言 | 工作区 glob→workspace 相对带前缀；可链式 |
| rework F-07/F-15 分页 | 统一 Page；read has_more=「后面还有」 |
| rework F-10 描述 | 能力面已 code 化；**`data.` 措辞未跟 A1** |
| remediation A1 | 已实施；SDK PRELUDE/类型正确 |
| remediation A4 | 静默修/教 bash（code）/Similar files 等已砍；executor 内 direct 分支仍有教模型句（文件工具不走） |

---

## 结论

- **可用：** 契约硬面（裸 data / ToolError / FileRef / Page / reason / edit 半成功字段 / 非法输入硬失败）与 run_code 编排目标一致，read→edit 与分页主路径可写。
- **主要风险（3）：** ① 描述/SDK 长文仍写 `data.*`；② edit 半成功易漏检 `failed`；③ 同 call 链式 edit / `page.next` 双方言。
- **是否 blocker：** **否**（不阻 agent 正确用法，但会提高试错成本）。建议后续小包：把五工具描述与 SDK 绑定注释中的 `data.` 改为裸字段名；弱化「line-number prefix」为「勿从带行号的外层文本粘贴」；可选在 PRELUDE 加一句「续页：read 用 start_line，glob/grep 用 cursor」。

**产物路径：** `.litecode/ops/artifacts/WP-193/dig-tools.md`
