# WP-193 dig-mid-notes — 中层证据摘录（只读）

**分支：** `feat/tool-contract-ptc` @ `bd71f8c`  
**说明：** 为 review.md 聚合用；细节见同目录 `dig-runtime.md` / `dig-tools.md`。本文件按六项给「现状一句 + 证据」。

---

## 1. Surface / 投影 / 路由拒绝 / run_code 自动出现

**现状（agent）：** 五文件工具声明 `Code`，不进 direct 列表；误 direct 调会看到「only available as tools.X inside run_code」；有任一已准入 Code 工具时才挂上 `run_code`。

**证据：**
- `src/tool/trait_.rs:14-21` — `ToolSurface::{Direct,Code}`；Code = `tools["name"]` inside `run_code`
- `src/tools/{read,glob,grep,edit,write}.rs` — 各自 `fn surface() -> Code`（如 `read.rs:51-52`）
- `src/tool/project.rs:22-24,38-47` — 按 surface 分面；`direct` 不含 `run_code`
- `src/tool/registry.rs:314-330` — `code_tools` 非空 + host → `CodeTool::from_surface` 追加为 `run_code`
- `src/tool/executor.rs:866-871` — `` `{name}` is only available as tools.{name} inside run_code ``
- `src/tool/executor.rs:853-854` — Code 内调 Direct：`` is a direct tool and is not available inside run_code ``
- `src/tool/availability.rs:184-186,176` — 设置目录 `omit_code_transport`（`run_code` 非开关）

---

## 2. run_code 描述与 PRELUDE / SDK

**现状（agent）：** `run_code.description` = 生成的 SDK 全文；PRELUDE 写清 resolve data / reject ToolError / TypeError / 不可捕获封口 / 约 64KB 视口。

**证据：**
- `src/tool/code_manifest.rs:16-27` — PRELUDE（契约 + `tools.name(args)` + 64KB）
- `src/tool/code_manifest.rs:215` — `out.push_str(PRELUDE)`
- `src/tools/code.rs:1090-1122` — schema：`code` 描述与 PRELUDE 同口径；`description()` 返回 `sdk` 或 admission 失败句
- `src/tools/code.rs:1096` — `` await tools.name(args) resolves to that tool's data ``

---

## 3. 模型可见 render / 64KB / skip spill

**现状（agent）：** 外层文本自渲染 ≤65536 字节（头→失败→错误明细→诊断→账本→logs→result），`is_code_transport` 绕过 emergency/spill。

**证据：**
- `src/tools/code/render.rs:16-17` — `RUN_CODE_VIEW_MAX_BYTES = 65_536`
- `src/tools/code/render.rs:77-110,113-118` — `render_model_view`；固定段顺序 header→failure→errors→diagnostics→ledger
- `src/tools/code/render.rs:206-218` — 头行如 `run_code failed: uncaught ToolError…` / `run_code ok · N calls`
- `src/tool/executor.rs:760-771` — `tool.is_code_transport()`：不 `finalize_tool_call_result`（不 spill）
- `src/tools/code.rs:1113-1157` — `is_code_transport()=true`；`max_result_size=usize::MAX`（外层不 spill）

---

## 4. ToolError / resolve data 投影

**现状（agent）：** 嵌套成功 → JS 拿到裸 `data`；领域失败 → catchable `ToolError{code,message,details?,retryable?,tool,seq}`；协议失败 → `TypeError`。

**证据：**
- `src/tools/code.rs:674` — `outcome::program_result(&result)`
- `src/tool/outcome.rs:195-223` — 成功 `Ok(result.data)`；失败 `HostError::tool(code,…)`；`retryable` 仅 `conflict|timeout`
- `src/ptc/engine.rs:843-878` — `raise_host_error` → `throw_tool_error` 构造 JS `ToolError`
- `src/tools/code.rs:627-639` — 多参数 / 非法值 → `HostError::type_error`（不进工具管线）

---

## 5. 五文件工具 descriptions 首段

**现状（agent）：** 已按 code 契约写（`data.*` / reject ToolError / 无 token 预算）；**文案仍写 `data.file` 前缀**，与 A1「resolve 裸 data」不一致，易写成 `r.data.file`。

**证据（首段）：**
- `src/tools/descriptions/read.md:1` — structured data；`unsupported` ToolError
- `src/tools/descriptions/edit.md:1` — `data.file/blocks/applied/failed`；部分失败 resolve
- `src/tools/descriptions/write.md:1` — `data.file` + `data.created`；`stale`/`conflict`
- `src/tools/descriptions/glob.md:1` — `data.base/paths/page`
- `src/tools/descriptions/grep.md:1` — `data.matches/page`；`invalid_regex`

---

## 6. 系统提示 code 模式指导

**现状（agent）：** harness 仅一句：有 `run_code` 时绑定工具只能 `tools.<name>(...)`，勿直接调。无更深编排指导。

**证据：**
- `src/config/global_db/prompts/harness/tools.md:1-4` — `# Using your tools` + run_code 边界句
- `src/context_pipeline/system.rs:30-39` — 拼入 `HARNESS_TOOLS`

---

## 仍像 direct 冗余（3–8）

1. descriptions / SDK 注释统一写 `data.xxx`，程序侧是裸 data（`r.file` 才对）— dig-tools 已点。
2. edit SDK 仍可能提「read 行号前缀」类 direct 习惯（见 dig-tools / matcher `read_line_prefix`）。
3. `AGENT_FILE_PATH_HINT` 仍塞 Windows/Git-for-Windows 说明书腔（`workspace/tool_path.rs:16`）。
4. Direct 分支仍保留「Retry after… / use bash run_in_background」教模型句；Code 路径已收短，但共享 executor 残留（`executor.rs:639-720`）。
5. `attach_unknown_param_warning` 对 direct/`run_code` 外层仍可挂「ignored unknown parameter」警告文案（`executor.rs:220-226`）；五工具 schema 已 `additionalProperties:false`。
6. 无 host 退化时 code 工具会落回 direct（`registry.rs:318-321`）— 与「勿直接调」提示矛盾。

## 已对齐 remediation（3–8）

1. 五工具 `surface()=Code` + 路由拒绝文案（I2 / D1）。
2. `run_code` 仅 code 面非空时自动出现；设置目录省略（D9）。
3. PRELUDE：resolve data / ToolError / TypeError / 封口不可捕获（I12）。
4. 自渲染 64KB + `is_code_transport` 跳过 spill（I10 / V-1 整改）。
5. `program_result` → 裸 data / `HostError::tool`；`retryable` 收窄到 conflict|timeout（A1/A4）。
6. descriptions 已去 markdown 行号、`grep.view`、fuzzy、token 分页等 direct 教法。
7. 渲染顺序改为失败/结果优先于大账本尾部丢失（header→failure→…→logs→result）。
8. harness 补了一句 code 边界（charter「补 code 模式指导」最小落地）。
