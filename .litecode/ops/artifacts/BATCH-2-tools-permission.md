# Batch 2 · tools-permission 目录清单

基线：`075308b`。负责人：Tools。本轮不核实实现（Batch 3）。

## 1. 拟新增节点

| key | 上层/父节点 | 准入 | 一句话承诺 |
|---|---|---|---|
| mcp-connection-pool | 父：tool-adapters | A2/A4 | MCP 启停与 call 只经连接池，无旁路 |
| custom-tool-hub | 父：tool-adapters | A2/A5 | Custom 后台 job_id + mailbox 完成投递 |
| webfetch-ssrf | 父：tool-implementation-contract | A6 | webfetch 公网 http(s) 出站门 |
| websearch | 父：tool-implementation-contract | A1 | websearch 只读检索适配，走公共出口 |

## 2. 拟保留 / 拟改存量

**保留（打磨 domain/基线/父节点 + 去重指针，不改主张意思）：**

- enabled：`tool-pipeline`、`tool-call-contract`、`tool-control-boundary`、`tool-implementation-contract`（补 `domain`/`基线`；后两者补 `上层: tools`；implementation 增 `契约展开` → webfetch/websearch）
- pending 子契约 10 个（`contracts/tools/tool-*`）：补 `domain: tools-permission`、`基线`；缺父的补 `父节点`；去掉与他节点重复的主证据，改 `相关：` 指向主节点

**主节点归属（消 dup 用）：**

| 不变量 | 主节点 |
|---|---|
| 取消/超时/批次 join | tool-call-lifecycle |
| ToolExecutionContext 装配 | tool-execution-context |
| 授权 + floor | tool-authorization |
| schema/语义校验 | tool-input-validation |
| name vs call_id | tool-identity |
| 可用/可见/列表快照 | tool-request-entry |
| call_id 配对交付 | tool-result-delivery |
| 分批 + 写锁 | tool-scheduling |
| 内置/Custom/MCP 适配差 | tool-adapters |
| 信号/截断/媒体出口 | tool-output-shaping |
| MCP 连接池 | mcp-connection-pool |
| Custom mailbox | custom-tool-hub |
| webfetch SSRF | webfetch-ssrf |

## 3. 疑似主节点重复消解

对照基线 `dup_primary_suspect`（多围绕 `run_tool` / MCP·Custom `execute`）：

- 已从次要节点撤掉重复符号，改为 `相关`/`父节点` 指向主节点。
- 本域 kb_check：`dup_primary_suspect` **0**；仅剩 enabled→pending 的 `inactive_target`×2（webfetch-ssrf / websearch），属挂树预期。
- 与 harness-core 的 `agent-loop` 若仍共享 `fn run` 类证据，归对方域对齐；本域已尽量改用 `interrupted_outputs` / `function_call_must_not_execute`。

## 4. Issue 草稿（Batch 3 再落盘）

| 草稿 id | 级别 | 摘要 | 依据 |
|---|---|---|---|
| （沿用 ISSUE-011） | drift/gate | `ssrf_rejects_missing_host` 前提与 WHATWG 主机解析不符 | webfetch-ssrf |
| （沿用 ISSUE-012） | bug | `validate_public_http_url` 漏 IPv4 映射 IPv6 / fe80 / 198.18；DNS 重绑定窗口 suspect | webfetch-ssrf · A6 |
| TP-B2-01 | suspect | Custom 后台完成事件与 session mailbox 字段是否全路径对齐（需 Batch 3 联调） | custom-tool-hub |
| TP-B2-02 | suspect | MCP `get_or_create` 与 agent 工具列表快照时序：配置存在 ≠ Warm | mcp-connection-pool ↔ tool-request-entry |

无本批阻断（不触及 B1–B5 新确认）。

## 5. 本域节点数（含 domain 标注后）

- tools-permission 带 `domain`：18（enabled 4 + 原 pending 10 + 新增 4）
- kb_check（全库）：errors **0**；本域 findings：inactive_target×2
