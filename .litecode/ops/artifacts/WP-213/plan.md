# WP-213 plan — run_code 提示词改造
基线 `8d0be10`（feat/tool-contract-ptc，不合 main）。不碰 `desktop/build/icon.ico`、`tool_traces.jsonl`。**提示词包：只改「说什么」，不改运行时行为。**

## Andy 已定（原样，不改范围）
1. **代码 / 类型 / schema 已编码的不说**（TS 声明、参数类型、返回形状、`reason` 联合类型等）。
2. **运行时结果头 / 错误文案已指引的不说**（`run_code ok|failed`、`program:`、`tool calls:`、截断注记、`[warn]`、ToolError message 等）。
3. **补「怎么干活」哲学**，落在 **Code 面 PRELUDE / `code` 工具描述面**；**禁止**污染 agent 系统提示词；**禁止** per-tool 长教程。要点：何时开 `run_code`；explore→edit→verify；并行 vs `await`；`return` 摘要。
4. **PRELUDE ≈15–25 行不变量**；砍 reason 枚举 dump、截断算法细节、结果头格式、逐工具字段复述。
5. **工具 md 瘦身**：最多 **一个短 example + 3–5 行语义**；能靠类型说清的删。
6. 刷 SDK 快照；artifacts 给 before/after + token 对比（cl100k）。

## 定位（只读 @8d0be10）与硬边界
| 面 | 路径 | 本包 |
|---|---|---|
| **PRELUDE** | `src/tool/code_manifest.rs:16-32` `const PRELUDE`（约 16 段密散文 / ~5410 字符；装进 `render_code_tool_description` → CodeTool.description = 整段 SDK） | **改** |
| **LIBS_SDK / TS 投影** | 同文件 `LIBS_SDK` + per-tool type alias（类型面） | **不改逻辑**；仅随 PRELUDE 重写后刷 fixture |
| **`code` 参数 description** | `src/tools/code.rs` `CodeTool::schema` 里 `properties.code.description`（长段复述 ok/ToolError/undefined/fs/libs） | **瘦**（属 code 工具描述面） |
| **工具 md** | `src/tools/descriptions/{read,glob,grep,edit,apply_edits,write,run,wait,kill,webfetch,websearch,diagnostics,document_symbol,find_references,go_to_definition,go_to_implementation,hover,workspace_symbol}.md`（经 `description_text(include_str!)` → Chat/Direct **与** Code SDK 注释共用） | **瘦 Code 面挂载的这些** |
| **SDK fixture** | `src/tool/fixtures/run_code_sdk.txt` + `.sha256`；测 `src/tool/sdk_snapshot.rs` | **刷** |
| **Code 准入列表** | `src/tool/registry.rs` `builtin_code_surface_tools`（read/glob/grep/edit/apply_edits/write/run/wait/kill/webfetch/websearch + 七 LSP） | **不改名单** |
| **系统提示词 / harness** | `src/config/global_db/prompts/harness/system.md`、`tools.md`（已有一行 run_code 绑定说明）、`builtin_prompts.rs`、各 agent pack | **禁止动**（diff 零触碰） |
| **运行时** | `src/tools/code/render.rs`、`src/ptc/engine.rs`、ok/kill/rejection 等 | **禁止动** |

**Chat 面副作用（须知）**：多数 Code 工具 md 与 Direct/Chat **共用同一文件**。瘦身会同时缩短 Chat 描述。本包不另开 Chat 专用教法、不做双轨。

**当前 PRELUDE 冗余地图（砍）**
- L1 截断算法 / 表格式细节 → 运行时超限注记已说
- L2 logs 200/2k 细节 → 运行时 `logs omitted` / 失败 logs 已说
- L5 `reason` 枚举全表 + ToolError code 全表 → 类型 `ToolFailure` / `ToolError` 已编码
- L6–L8 / L10–L11 逐工具字段与 read/grep 契约 → 各工具 TS + md example
- L12 结果头格式全文 → 每次运行时已打印
- L14 undefined 容忍细则 → 失败头已列 dropped paths（见 Q5）

**保留理由清单（落地时每一句必须能对上）** — 见下方「文案草案」。

## 分域与串并行
- **kind=direct**，域 **Tools**（PRELUDE、`code` schema 文案、工具 md、SDK 刷）。
- **Runtime 不参与**（无行为改动）。
- 阶段：① 重写 PRELUDE + 瘦 `code` schema description → ② 按清单瘦工具 md → ③ 刷 SDK fixture/sha → ④ examples.md（前后对照 + cl100k）+ acceptance + `cargo test -j 2` 全绿。
- cargo：一次一个重任务，`-j 2`，跑前 `free` available ≥2GiB。顺序建议：`cargo test -j 2 --lib tool::sdk_snapshot tool::code_manifest` → 相关 tools 描述/快照测 → `cargo test -j 2 --tests --no-run` → tip 全量 → `artifacts/WP-213/cargo-full.txt`。

## 文案草案（供 GM 批；英语，与现 PRELUDE 模型面语气一致）

### A. 新 `PRELUDE`（目标 18 行正文 + 空行 + \`\`\`ts；约 ≤900 字符）

> **完整 PRELUDE 字符串（落地原样写入 `const PRELUDE`，末行保留开启 TS fence 的 ` ```ts `）**
>
> Open `run_code` when one program can finish explore → edit → verify (search/read, then write/edit, then run/check). Do not open a long program just to peek one file—use a direct tool or a tiny program.
> Inside the program: explore first, then edit, then verify. `await` when a later call needs an earlier result; otherwise prefer `Promise.all` / `Promise.allSettled` for independent work.
> `return` a short summary (counts, paths, the few lines that matter)—not raw dumps. Only the returned value reaches you; oversized results are truncated (a note at the start says so).
> `code` is plain JavaScript. The TypeScript declarations below are documentation only; do not paste type annotations into `code`.
> Call bundled tools only through `tools.name(args)` or `tools["name"](args)` (one argument object each). They are not separately callable. Other LiteCode tools may remain directly callable outside this SDK.
> Every call resolves an object with boolean `ok` (did the tool get the job done). Soft failures resolve `ok: false` with `reason`/`message` so one bad item does not end a batch. System failures reject `ToolError`. Unknown tool/parameter or illegal args throw `TypeError` and fail the whole call—unknown fields are never silently dropped. Cancellation, the `run_code` budget, and hard limits end the program and cannot be caught.
> `libs` are deterministic pure-JS helpers (no filesystem, network, or process). `fs.ls` / `fs.stat` / `fs.exists` are synchronous, read-only workspace probes—change files with `tools.write` / `tools.edit`. The sandbox has no `setTimeout` / `setInterval`.
> Unhandled Promise rejections do not end the program; a `[warn]` notes them after settle.
>
> \`\`\`ts

**留下的每一句 ↔ 不可替代理由**

| 句 | 为何不能只靠类型/运行时 |
|---|---|
| 何时开 `run_code` / 勿为瞄一眼硬开 | 工作哲学；类型/结果头都不教策略 |
| explore→edit→verify + 并行 vs await | 同上 |
| `return` 摘要；超限会截断（一句） | 策略 + 存在性；**不**讲算法（运行时注记负责细节） |
| `code` 是 JS；TS 仅文档 | 模型易把声明贴进 `code`；类型本身不会说「别抄」 |
| 只经 `tools.*` 调用 | 准入边界；防止当 Direct 工具乱调 |
| `ok` 一句 + soft resolve vs ToolError vs TypeError（含 unknown param 整调用失败） | 类型有联合，但「批内不中断 / 未知参数不静默丢 / 整调用失败」是行为不变量 |
| libs/fs 只读；改文件走 write/edit；无定时器 | fs/libs 形状在 TS，但「无写/无 timer」是沙箱不变量 |
| unhandled rejection → `[warn]` 不结束 | 与 `allSettled` 哲学配套；一行 |

**明确不进 PRELUDE**：reason/ToolError 枚举表、截断/表格算法、结果头字段说明、logs 200/2k、逐工具（edit/run/read/LSP/web*）字段、undefined 细则（见 Q5）。

### B. `code` 参数 schema description（草案；取代现长段）

```text
Async JavaScript function body for the Code SDK below. Prefer explore → edit → verify in one program; return a short summary. Call tools as `await tools.name(args)` and branch on `ok`.
```

（`timeout_ms` 描述已短，可不动。）

### C. 工具 md 瘦身口径（示例；落地按同标准扫完全表）

**原则**：删 Result contract / reason 枚举复述 / 「不 reject ToolError」长段（PRELUDE + TS 已覆盖）；保留 **≤5 行语义**（仅该工具特有、类型说不清的）+ **一个短 example**。

**`apply_edits.md`（现 30 行 → 目标约）**
```text
Apply several independent `{path, old_string, new_string}` replacements in one call. Runs serially in array order; later items on the same path see earlier successful writes. Failed items are skipped; the call still resolves `{ results }` (same length as `edits`)—filter `results` yourself. Exact-match / unique-hit / CRLF / BOM rules match `edit`. Empty `new_string` deletes that item's `old_string`.
Example:
const batch = await tools.apply_edits({
  edits: [
    { path: "src/a.rs", old_string: "fn start()", new_string: "fn main()" },
    { path: "src/b.rs", old_string: "old_api(", new_string: "new_api(" },
  ],
});
if (batch.results.some((r) => !r.ok)) return { ok: false, results: batch.results };
return { ok: true, results: batch.results };
```

**`run.md`（现 25 行 → 目标约）**
```text
Run one shell command. Resolves `id`, `exit_code` (int or `null` if still running), `output` (stdout), `stderr`, `shell`. Foreground ~30s then backgrounds with `exit_code: null`—continue with `tools.wait` / `tools.kill`. cwd is always workspace root; chain with `&&` when needed. Non-zero `exit_code` still resolves—branch in-program. Parallel `run` via `Promise.all` is truly concurrent.
Example:
const test = await tools.run({ command: "cargo test -q --lib a" });
const done = test.exit_code === null ? await tools.wait({ id: test.id, sec: 60 }) : test;
return { exit_code: done.exit_code, output: done.output };
```

**`edit.md` / `read.md` / `grep.md` / `glob.md` / `wait.md` / `kill.md` / `webfetch.md` / `websearch.md` / LSP 七份**：同口径压缩；已很短者（如 `write.md` 单段）只删 reason 枚举堆砌、保留特有语义（`expected_hash`/`create_only`/`conflict`/`stale`）+ 可选极短 example。
**非 Code 面 md**（`ask_user`、`subagent_*`、`todo`、`plan`、`knowledge`、`session_search`、`code_search`、`litecode_workspace` 等）：**本包不动**。

## 测试与验收
- **不堆新测**。必做：刷 `run_code_sdk.txt`/`.sha256`；`sdk_snapshot` / `code_manifest` 既有测绿。
- 若有 protected / 字符串断言绑旧 PRELUDE 文案：落地时 **逐条报备**改断言原因（预期几乎只有 SDK 快照）。
- artifacts：`examples.md`（改造前后 PRELUDE 全文 + 整段 SDK 或 PRELUDE 的 cl100k token；工具 md 各举 1–2 个前后对照）；`acceptance.md`；`cargo-full.txt`。
- 验收阈值（plan 钉死）：PRELUDE **≤25 行**且散文 **≥60% 字符下降**（相对现状 ~5410）；无 reason 枚举 dump；含工作哲学四句要点；**diff 零触碰** harness/system prompt；全量 `cargo test -j 2` 绿。

## Q 列表（附推荐）
Q1 Chat/Direct 与 Code **共用**工具 md，瘦身会缩短 Chat 描述——本包接受、不做双轨？推荐：**是**（Andy 要去冗余；Chat 仍有 schema）。
Q2 非 Code 面 md（subagent/todo/…）本包不动？推荐：**不动**。
Q3 PRELUDE 用上方英文 8 段草案（哲学 + 不变量），不写中文？推荐：**英文**（与现模型面一致）。
Q4 删 PRELUDE 内截断算法 / logs 配额 / 结果头格式 / 逐工具段 / reason·ToolError 枚举表？推荐：**全删**。
Q5 undefined 容忍细则：从 PRELUDE 删除（失败头已列）？推荐：**删**。
Q6 `code` schema `properties.code.description` 换成上方两句短描述？推荐：**是**。
Q7 工具 md 目标「≤5 行语义 + 1 example」；`apply_edits`/`run` 等超长文件整篇重写，已短文件只删契约复述？推荐：**是**。
Q8 harness `tools.md` / `system.md` / agent pack：**零 diff**？推荐：**零 diff**（硬边界）。
Q9 token 验收：PRELUDE 字符降 ≥60%，并用现有 `count_text_tokens`（cl100k）写入 examples.md？推荐：**是**。
Q10 本包不改运行时、不加测数量、不合 main？推荐：**是**。

## 风险
- 压缩过度丢掉「类型/运行时说不清」的不变量 → 用上方保留清单逐句对账；GM 可点名加回一行。
- 共用 md 瘦身后 Chat 面变短 → Q1；若回归可下包双轨（本包不做）。
- SDK fixture ~55KB，文案大变必刷；sha 测会红到刷完为止。
- 若干旧字符串断言若绑 PRELUDE 片段 → deliver 时逐条报备。
- 工作区若出现他人未提交脏文件 / `tool_traces.jsonl`：提交只带 WP-213 显式路径。
