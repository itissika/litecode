# WP-195 dig-mid 笔记 — bash 系列工具盘点（只读）

分支：`feat/tool-contract-ptc`  
约束：只读产品码；未改 descriptions / code_manifest / run_code_sdk。

---

## 1. bash / shell / 终端类工具清单

同族注册工具仅三者；均未覆盖 `surface()`，走 trait 默认 `ToolSurface::Direct`（`src/tool/trait_.rs:49-50`）。  
辅助模块 `bash_status` / `bash_safety` / `bash_jobs_contract` 非独立工具。

| 工具名 | surface() | 入口文件 |
|--------|-----------|----------|
| `bash` | Direct（默认） | `src/tools/bash.rs` |
| `kill_shell` | Direct（默认） | `src/tools/kill_shell.rs` |
| `wait_shell` | Direct（默认） | `src/tools/wait_shell.rs` |

注册：`src/tool/registry.rs`（`"bash"` / `"kill_shell"` / `"wait_shell"` → 各 Tool + `ide.terminal`）。

---

## 2. charter.md 原文摘录（bash 相关）

**D2（§2）：**

> **D2 首期 code 工具集只含核心文件工具。** `read`、`glob`、`grep`、`edit`、`write`。写工具 code 化是首期必须落地的范围。其余工具保持 Direct：bash 系列、subagent、todo/plan、knowledge、litecode_workspace、session_search、code_search、lsp、webfetch、websearch、ask_user、custom、mcp。

**范围「不做」（§4）：**

> 不做：MCP 的 code 化；custom tool 首期 code 化；持久 REPL；网络；TypeScript 语法；任意 `import`/`require`；per-agent 的 code 开关或权限预设；消息列表中 `run_code` 的专门视图；bash/subagent 的 code 化（未来目标，但不在本轮）。

**未来目标句：** 上句末尾——「bash/subagent 的 code 化（未来目标，但不在本轮）。」

---

## 3. code-tool-rework / run-code-remediation 是否提 bash code 化

**有。**

- `docs/ptc/code-tool-rework-plan.md`：  
  > 契约是后续能力的前置：多文件编排、未来的 bash（含 `kill_shell` / `wait_shell`）code 化，都要求先有“结构化结果 + 文件版本 + 确定并发”。  
  另：§5.2「不在本批范围」列出 `` `bash` / `kill_shell` / `wait_shell` 的 code 化（D-11；需要独立项目：jobs、PTY、流式、取消）。``
- `docs/ptc/run-code-remediation-plan.md`：  
  > 不在范围：Session schema、ctx/提示词组装、MCP/custom、bash 系列的 code 化、新增工具能力。

---

## 4. agent 视角：今日 Direct bash

**描述首段**（`src/tools/descriptions/bash.md` 全文即一段）：

> Run a shell command. cwd and environment do not persist across calls — chain with && in one command. Use bash for builds, tests, git, and scripts. Short commands return when they exit. For longer work, set timeout (seconds, default 30) so this call waits that long; if still running, the job continues in the background: wait_shell to wait, kill_shell to stop.

**结果形态一句话：** 明文文本——前台多为 `exit_code` + 内联输出（过大则 head/tail + `output_file`）；后台/等待为 `status: running|exited|waited` + `bash_id` / `output_file` / running 列表（见 `bash_status.rs`）。

