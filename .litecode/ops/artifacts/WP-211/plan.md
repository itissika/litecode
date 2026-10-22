# WP-211 plan — run_code 体验残留三刀
基线 83229d7（feat/tool-contract-ptc，不合 main）。不碰 desktop/build/icon.ico、tool_traces.jsonl。范围仅 brief 三刀，不扩。

## Andy 已定（原样）
1. write conflict/stale → 软失败：`soft_failure_reason` 纳入 `conflict`|`stale`；程序侧 resolve `{ok:false, reason, message}`；timeout/cancelled/unavailable/internal/unsupported 等仍 throw。
2. MSYS pathconv 假成功：`msys_path_hint` 命中时返回 `ok:false`、`exit_code: null`，保留 `message`+`shell`（stdout/stderr 可留作诊断）；禁止再挂 `exit_code: 0`；不加 `timeout_ms`/`background`。
3. PRELUDE 删掉硬编码 LSP 七名；描述只经 admitted tool blocks 组装；`desired=false`（或未准入）时 `render_code_tool_description` 与 projected whitelist 均不含 `LSP_CODE_TOOL_NAMES` 任一名字。不从 `builtin_code_surface_tools` 移除七工具；不削弱 `build_tool_list` 的 Ready+bind 门禁。

## 定位（只读 @83229d7）与分域
| 刀 | 位置 | 域 |
|---|---|---|
| ① soft_failure | `src/tool/outcome.rs:201-232` `soft_failure_reason`（现不含 conflict/stale；注释仍写「仍 reject」）；`:291-320` protected 单测把 stale 当系统失败；write 产出点 `src/tools/write.rs:338-378` `write_precondition`（conflict/stale 已是 coded failure，桥接层会改语义） | Tools（outcome + write 文档） |
| ① 文档/类型 | `src/tools/descriptions/write.md:1`（仍写 rejects ToolError stale/conflict）；`src/tool/code_manifest.rs:20` PRELUDE 契约段（conflict/stale 列在「Only system failures reject」）；`:100-104` `ToolFailure.reason` 联合类型缺 conflict/stale；`src/tool/fixtures/run_code_sdk.txt` 同步 | Tools |
| ① protected 意图 | `tests/run_code_model_view.rs:132-146` `uncaught_tool_error_names_the_call`（create_only conflict 断言仍抛） | Tools（改断言，acceptance 写明意图） |
| ② MSYS | 提示：`src/terminal/decode.rs:146-161` `msys_path_hint`；挂载：`src/tools/run.rs:162-169` `command_message`、`:118` 传入、`:79-106` `command_result` → `present_command`；呈现：`src/tools/command_job.rs:227-246`（现 `ToolOutcome::ok`，hint 时仍可带 `exit_code:0`） | Tools（run/command_job；decode 提示函数不改语义，仅被调用方改呈现） |
| ③ LSP 门禁 | PRELUDE 硬编码：`src/tool/code_manifest.rs:26`（七名那句）；组装：`:182+` `render_code_tool_description` 只渲染传入 tools；准入：`src/tool/project.rs:50-119` admitted→sdk/whitelist；名表：`src/tools/lsp.rs:37-45` `LSP_CODE_TOOL_NAMES`；`builtin_code_surface_tools`/`build_tool_list` **不动** | Tools |

分域说明：三刀代码均落在 Tools 既有文件（outcome/write/run/command_job/code_manifest）。MSYS 呈现改动在 run/command_job，与 WP-210 F 同域；不单开 Runtime（decode.rs 只读复用）。包 kind=direct，我统筹提交。

## 串并行
- 阶段1（Tools 一域）：① outcome + write.md + PRELUDE/ToolFailure 文案；② present_command/run 在 hint 命中时 `ok:false`+`exit_code:null`；③ 删 PRELUDE L26 硬编码句。
- 阶段2：刷 `run_code_sdk.txt` / `.sha256`（builtin 仍含 LSP 七工具，仅 PRELUDE 与 ToolFailure/write 文案变）。
- 阶段3：我写 acceptance（含 protected 断言变更报备）、cargo-full、提交推送。

## cargo
一次只一个重任务，`-j 2`，每次前 `free` 确认 available ≥2GiB。顺序：
1. `cargo test -j 2 --lib tool::outcome tools::write tools::run tools::command_job tool::code_manifest`
2. `cargo test -j 2 --lib sdk_snapshot`
3. `cargo test -j 2 --tests --no-run`
4. tip 全量 `cargo test -j 2` → `artifacts/WP-211/cargo-full.txt`

## 测试（仅三刀核心，入 protected.txt）
新增约 3–4 条：
1. ① `conflict`/`stale` → `program_result` resolve `{ok:false, reason, message}`；`timeout`/`unavailable`/`internal` 仍 reject（改现有 `domain_failures_resolve_ok_false_and_system_failures_reject`）。
2. ① 集成：create_only conflict 不再 uncaught；断言 `ok:false`+`reason=conflict`（改 `uncaught_tool_error_names_the_call`，acceptance 写明「原抛错意图改为软失败」）；另用仍会 throw 的用例保住「系统失败命名 call」——推荐 `unavailable` 或参数层无法轻易构造的 timeout，见 Q1。
3. ② hint 命中：呈现 `ok:false`、`exit_code` JSON null、保留 `message`+`shell`；不得出现数字 `0` 的 exit_code（单测直接打 `present_command`/辅助函数，不依赖 MINGW，见 Q4）。
4. ③ 传入不含 LSP 七名的 tool 列表时，`render_code_tool_description` 文本与推导 whitelist 均不含 `LSP_CODE_TOOL_NAMES` 任一名字（模拟 desired=false/未准入）。

不堆量；SDK 快照测试随文案变绿。

## 验收
brief 三条 + cargo 全绿 + SDK/文档同步 + protected 变更逐条报备。push 实验分支，不合 main。

## Q 列表（附推荐）
Q1 `uncaught_tool_error_names_the_call`：改断言 conflict→`ok:false`，并另留/另加一条仍 throw 的 call 命名用例（如强制 `ToolError unavailable` 的夹具）？推荐：是（软失败改原测；throw 意图另测一条，保持「uncaught 命名 call」覆盖）。
Q2 MSYS：先照常 spawn，再在呈现层强制 `ok:false`+`exit_code:null`（保留 stdout/stderr 诊断），而非跳过执行？推荐：是。
Q3 MSYS 结果要不要 `reason`？推荐：要，`reason: "invalid_args"`（命令形态在 MSYS 下不安全；不扩枚举）。数据仍带 id/output/stderr/shell/message。
Q4 Linux CI 无 MINGW：单测不依赖 `shell_identity`，直接测「message=Some(hint) ⇒ 呈现分支」？推荐：是。
Q5 `stale_cursor` 仍 throw（仅 `conflict`|`stale` 入 soft）？推荐：是。
Q6 CommandData：pathconv 时在 data 内显式带 `ok:false`（利用 `with_ok_flag`「已有 ok 则保留」），成功路径仍靠 with_ok_flag 补 `ok:true`？推荐：是（最小改 schema；pathconv 不走 `ToolOutcome::failure`，以免丢掉流字段）。
Q7 ③ 回归测挂在 `code_manifest` 单测（手工构造无 LSP 的 tool 列表），不改 registry/project 生产门禁？推荐：是。
Q8 write 的 `Promise<WriteData | ToolFailure>` 签名保留，只扩 `ToolFailure.reason` 与文档？推荐：是。

## 风险
- ① 语义面：任何依赖 conflict/stale throw 的调用方/测试会回归；`--tests --no-run` + 全量必跑；protected 断言变更逐条报备。
- ② pathconv 在 Linux CI 只能测呈现分支；Windows 真机仍靠 Andy（与 WP-210 F 同）。
- ② 若 Q3 要新 reason 字面量，需同步 PRELUDE/ToolFailure/SDK。
- ③ 删 PRELUDE 硬编码后，全量 SDK fixture 仍含七工具（builtin 未删）——验收看「未准入列表」回归，勿误判 fixture 有名即失败。
- 工作区 `tool_traces.jsonl` 有本地脏行，提交不碰。
