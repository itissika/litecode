# WP-203 摸底 dig — webfetch / websearch 进 Code + 软失败 + TUN/过度防御

> **作者：** backend_mid（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 02:58 Asia/Hong_Kong  
> **分支 / HEAD：** `feat/tool-contract-ptc` @ `b38ebc4`  
> **依据：** `brief.md`（Andy 定稿）  
> **硬约束：** **只读**；**未改代码**；**未 cargo**；**未 commit/push**；不合 main

---

## 0. Brief 目标（对照）

| # | 目标 | 现状摘要 |
|---|---|---|
| 1 | `webfetch` / `websearch` → **Code-only**（无 Direct） | 两工具均 **默认 `ToolSurface::Direct`**（未 override `surface()`）；在 Direct LLM 列表（绑定后）；**不在** `builtin_code_surface_tools()` / SDK fixture |
| 2 | **软失败**：HTTP/业务失败 → 工具数据或可 catch 的 `ToolError`，尽量不打崩 `run_code` | 现状几乎一律 `ToolCallResult::error` → `program_result` → **reject `ToolError`**；未 catch 则 **整次 run_code 失败**（PRELUDE 明文） |
| 3 | TUN / 过度防御：开 TUN 后 fetch 不可用 | **已定位**：`is_ipv4_benchmarking`（`198.18.0.0/15`）+ `is_ipv4_shared`（`100.64.0.0/10`）。本机 `getent` 对 `example.com`/`google.com`/`no-host` **一律 → 198.18.0.1**，主机名 fetch 会被 SSRF 门挡死 |

非目标：合 main；改 libs；LSP。

---

## 1. 现注册面（Direct vs Code）

### 1.1 身份与分类

| 项 | webfetch | websearch |
|---|---|---|
| `Tool::name` | `"webfetch"` | `"websearch"` |
| 别名 | **无**（无 `web_fetch` / `fetch` 工具名） | **无**（Exa MCP 内部方法名 `web_search_exa`，非 agent 工具名） |
| `Tool::surface` | **默认 Direct** | **默认 Direct** |
| 目录类 | `network_core_tools()`（**core**，非 optional_builtin） | 同左 |
| 引擎 id | `"webfetch"`（`WebfetchEngine`） | `"websearch"`（`WebsearchEngine` + Exa endpoint） |
| Settings 权限面 | `PermissionSurface::Fixed`（无 All/Safe 预设差异） | 同左 |
| 默认 primary 绑定 | **不**自动绑（`seed_primary` 只绑 configurable + none + `code_search`） | 同左 |
| explore 种子 | 绑 `ToolPreset::All` | 同左 |
| 可用卡 | `is_available` 恒真（core）；`agent_tool_enabled` 看绑 | 同左 |

注册：`registry::instantiate_tool` → `WebFetchTool` / `WebSearchTool` + `warm_hook`。  
引擎：`optional::EngineManager`；webfetch warmup 只建 client；websearch 还需 endpoint（Exa key / `EXA_API_KEY`）。

### 1.2 Schema（模型可见入参）

**webfetch**

```json
{
  "type": "object",
  "properties": {
    "url": { "type": "string", "description": "The URL to fetch content from" },
    "format": {
      "type": "string",
      "enum": ["text", "markdown", "html"],
      "description": "Desired output format (default: markdown)"
    }
  },
  "required": ["url"]
}
```

**websearch**

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "The search query" }
  },
  "required": ["query"]
}
```

### 1.3 出参（现状 = Direct 文本）

| 工具 | 成功 | 失败 |
|---|---|---|
| webfetch | `ToolCallResult::ok(truncated_body)` — **无 `data` / 无 `data_schema`**；上限 50_000 bytes | `ToolCallResult::error(msg)`（含 http/DNS/SSRF/warm/redirect） |
| websearch | `ToolCallResult::ok(provider_text)` — 同样无 structured data | `ToolCallResult::error(msg)`（warm/config/Exa HTTP/parse） |

教法：`descriptions/webfetch.md`、`descriptions/websearch.md`（各一行发现/拉取指引）。

### 1.4 Direct vs Code 暴露路径

```
available_tools ──(network_core 始终入卡)──► bind 后 instantiate
project_tools ── surface==Code → code_tools → run_code 白名单 + SDK
              └─ else → Direct（模型顶层 function-call）
```

| 面 | 现状 | WP-203 目标 |
|---|---|---|
| **Direct** | 绑定后两名可顶层调用 | **清零** |
| **Code** | 未进；无 `data_schema`；不在 `builtin_code_surface_tools()` | 两名 `surface=Code` + 完整 data 契约，进 bundle |
| SDK fixture | `run_code_sdk.{txt,sha256}`（sha=`60686e6c…`，WP-202 tip）**无** webfetch/websearch | Tools 刷白名单两名 + PRELUDE 软失败句 |
| 引擎卡 | Engines Warm（webfetch）/ Advanced Exa（websearch） | **保留**引擎 id；只迁调用面 |

---

## 2. 失败语义与 run_code 调用链

### 2.1 调用链

```
tools.webfetch(args) / tools.websearch(args)   // 目标：Code 内
  → invoke_binding (tools/code.rs)
      → host.run_nested → Tool::execute / call_inner
      → ledger.record(result)
      → outcome::program_result(&result)
           ├─ 成功 (level≠Error, error=None) → Ok(result.data ?? null)
           └─ 失败 → Err(HostError::tool(code, message, details, retryable))
                → PTC raise → JS `ToolError` reject（可 catch）
                → 未 catch → run_code 头「uncaught ToolError」整程序失败
```

协议层（多余参数 / 过深嵌套 / 工具不在 bundle）→ `HostError::type_error` → JS `TypeError`（与 domain 不同）。

### 2.2 现状失败分类（webfetch）

| 场景 | 现状出口 | 现 code |
|---|---|---|
| 缺/空 `url`、非法 URL 串 | `ToolCallResult::error` | 默认 `internal`（未设 `error_code`） |
| scheme 非 http(s) | error `"scheme '…' not allowed"` | 同上 |
| DNS 失败 | error `"could not resolve host"` | 同上 |
| 解析到非公网 IP（含 198.18 / 100.64 / 私网 / 回环…） | error `"URL resolves to a non-public address (…)"` | 同上 |
| HTTP 非 2xx | error `"http error: {status}"` | 同上 |
| 读 body / 过多 redirect / 超时 (client 30s, tool 35s) | error | 同上 |
| 引擎未 warm / warm 失败 | error + Settings 提示 | 同上 |
| join/panic | error | 同上 |

**websearch：** 空 query → error；未配置/未 warm → error + Exa 提示；Exa HTTP/`error_for_status`/不可解析 → error。无 SSRF URL 门（只打 Exa MCP）。

### 2.3 与「软失败」的差距

PRELUDE 已区分：

- **domain `ToolError`**：可 catch；**未 catch → 打崩程序**
- **edit 软失败**：`EditResult{ok:false,message}` **resolve**（不 reject）
- **协议 TypeError / 预算 / cancel**：不可当业务软失败

现状网络两工具走 **硬 error → reject**，且多数未填稳定 `error_code`（进 Code 后 `ToolError.code` 会糊成 `internal`）。  
进 Code 后若不改出口形状，模型必须对每次 fetch/search 包 `try/catch` 或 `Promise.allSettled`，否则一次 404/DNS 就毁掉整段 `run_code`——这正是 brief「软失败」要消的痛。

**建议边界（待 mid Q1）：**

| 类 | 建议出口 | 例 |
|---|---|---|
| **业务/HTTP 软** | `ok_data`：`{ok:false, …, message}` **不 reject** | 4xx/5xx、超时、DNS、解析失败、Exa 业务错 |
| **可 catch 的契约错** | `ToolError`（稳定 code） | 非法入参（空 url/query）、未 warm / 未配置（可选：也进 ok:false） |
| **升程序失败** | 仅协议 / 沙箱 / 宿主基础设施 | 多余参数、嵌套过深、surface mismatch、host panic、run_code 预算 |

SSRF「非公网拒绝」更接近 **安全门**：建议仍 `ToolError`（`permission_denied` 或新 `blocked`）或 `ok:false`+明确 code——见 Q2（是否与软失败同形）。

---

## 3. 网络 / 沙箱防御与 TUN（Andy）

### 3.1 Gate 位置（唯一出站门）

文件：`src/tools/webfetch.rs`

1. `resolve_and_validate_public_http_url`  
   - 只许 `http`/`https`  
   - DNS 或字面 IP → 逐地址 `is_non_public_ip`  
   - 返回「本趟已校验公网 IP」供钉死  
2. `client_for_validated_hop`：`redirect=Policy::none` + `resolve_to_addrs(host, pinned)`（关 ISSUE-021 重绑定窗）  
3. `fetch` 循环 ≤10 跳，每跳重校验 Location  

契约：`knowledge/contracts/tools/webfetch-ssrf.md`；回归 `webfetch::tests::ssrf_*`。

### 3.2 非公网分类（`is_non_public_ip` / IPv4）

| 类 | 条件 | 典型来源 |
|---|---|---|
| loopback / unspecified | `127/8`, `::1`, `0.0.0.0` | 本机 |
| private RFC1918 | `10/8`, `172.16/12`, `192.168/16` | 内网 / 部分 VPN |
| link-local | `169.254/16`, `fe80::/10` | 云 metadata 等 |
| unique-local IPv6 | `fd00::/8` | |
| **CGNAT / shared** | **`100.64.0.0/10`**（`is_ipv4_shared`） | **Tailscale 等** |
| **benchmarking** | **`198.18.0.0/15`**（`is_ipv4_benchmarking`） | **Clash/Surge/Mihomo fake-ip、部分 TUN/sinkhole** |
| IPv4-mapped | `::ffff:a.b.c.d` 按嵌入 v4 判 | ISSUE-012 |

### 3.3 Andy「开 TUN 后不可用」——结论

**根因（高置信）：** 校验看的是 **DNS 解析结果**，不是本机 TUN 接口。  
许多 TUN（尤其 **fake-ip**）把公网域名解析到 **`198.18.0.0/15`**；本门在 ISSUE-012/Batch3 **故意拒绝**该段 → 主机名 fetch 全灭。

**本机实证（Asia/Hong_Kong 摸底时）：**

```
getent hosts example.com  → 198.18.0.1
getent hosts google.com   → 198.18.0.1
getent hosts no-host      → 198.18.0.1
```

与 ISSUE-011 repro（「本环境 DNS 常把任意名解析到 198.18.0.1」）一致。  
字面公网 IP（如 `http://8.8.8.8/`）测例仍允许——故「过度防御」表现为 **主机名路径死、字面 IP 活**。

次要风险：

- **`100.64/10`**：Tailscale 等 CGNAT 出口/内网名解析会被拒（可能误伤「合理私网」场景）。  
- **系统代理**：契约已注「系统代理路径仍可能由代理解析」；`client_for_validated_hop` 未显式 `no_proxy`；钉 IP 与 HTTP 代理并存时行为依赖 reqwest 默认（dig 未跑网络实测）。  
- **websearch**：无 URL SSRF 门；走 Exa HTTPS；TUN 若破坏到 `mcp.exa.ai` 的连通性则表现为请求失败（非 198.18 分类器）。

### 3.4 修复方向（供方案，本 dig 不改码）

| 方向 | 含义 | 风险 |
|---|---|---|
| A. 对 **fake-ip 段放行但连接仍钉解析 IP** | 198.18 不再当 SSRF 拒；流量仍进本机 TUN/代理栈 | 需确认不会把「真·基准测试网」当公网；与 ISSUE-012 意图对冲 |
| B. 检测「解析 IP ∈ fake-ip/CGNAT **且** 主机非字面 IP」时改道：不钉死 / 走系统代理 / 或二次策略 | 更贴 TUN | 实现复杂；重绑定窗要重审 |
| C. 配置开关「允许 RFC6598 / 198.18」或「信任系统代理」 | 产品可控 | Settings 面；默认仍安全 |
| D. 仅放宽 198.18，**坚持拒** 真私网/回环/169.254 | 最小修 Andy 路径 | 推荐作默认修法候选 |

**dig 建议默认：D + 测「主机名→198.18 仍能 fetch（经 TUN）」；真 RFC1918/loopback/metadata 保持拒绝。** mid 见 Q2。

---

## 4. 底座能力与 Code-only 最小改动面

### 4.1 实现路径

| 路径 | 角色 |
|---|---|
| `src/tools/webfetch.rs` | Tool + SSRF + HTML→md/text |
| `src/tools/websearch.rs` | Tool 壳 |
| `src/optional/{webfetch,websearch}_engine.rs` + `exa_mcp.rs` | Warm client / Exa MCP |
| `src/tool/registry.rs` | instantiate；`builtin_code_surface_tools`（**现无此二名**） |
| `src/tool/outcome.rs` | `ok_data` / `program_result` / `ToolError` |
| `src/tool/code_manifest.rs` | PRELUDE + SDK 投影（要 `data_schema`） |
| `src/tool/fixtures/run_code_sdk.{txt,sha256}` | Tools 刷 |
| `knowledge/contracts/tools/webfetch-ssrf.md`、`websearch.md` | 契约对齐 |
| 单测 | `webfetch::tests::ssrf_*`；`websearch::tests::*`（冷引擎/空 query） |

依赖：`reqwest` blocking、`url`、`regex`（webfetch 排版）；websearch → Exa hosted MCP。

### 4.2 最小 Code 迁移动作（相对 WP-202）

比 LSP 简单：**不必拆工具、不必改引擎卡**。

1. 两工具 `fn surface() -> Code`  
2. 补 `data_schema` + 成功/软失败走 `ok_data`（结构化）；硬错走稳定 `error_code`  
3. 纳入 `builtin_code_surface_tools()`（或等价 admission 集——否则 SDK snapshot / 设置目录 Code 面漏名）  
4. PRELUDE 补软失败/网络教法；刷 fixture sha  
5. Direct 清零：靠 surface 门禁（executor `surface_mismatch`）；测「Direct 再调 → missing/mismatch」  
6. TUN：按 Q2 改 `is_non_public_ip` 或旁路条件 + 单测  

**不必：** 改 `network_core_tools` 身份（可仍 core）；不必 optional 卡；不必动 web/。

### 4.3 风险

- Code 准入硬要求 `data_schema`；现两工具只有 content 字符串 → **必做结构化**。  
- 默认 primary **未绑** network：迁 Code 后仍靠用户/explore 绑定；产品预期不变即可。  
- 放宽 198.18 与 ISSUE-012/契约文案需同步，避免 drift。  
- websearch 出参现为 Exa 纯文本；进 Code 可先 `{ok, text|results?, message}` 再迭代。

---

## 5. 硬清 / 迁移清单

### 5.1 代码

| 项 | 动作 |
|---|---|
| `WebFetchTool` / `WebSearchTool` `surface` | → `ToolSurface::Code` |
| `data_schema` + `ok_data` 出参 | 新增；成功与软失败同形 |
| HTTP/DNS/超时/解析 等 | 按 Q1：优先 **不 reject** |
| SSRF / 未 warm / 非法入参 | 按 Q1/Q2 定 code 或 ok:false |
| `builtin_code_surface_tools` | **加入**两名（需能无 engines 构造 stub/default，或 snapshot 用同构壳——实现时对齐 read 的无状态构造；现 instantiate 依赖 `EngineManager`） |
| Direct 回归 | 断言投影后 Direct 无名；Code whitelist 有名 |
| `error_code` | 填稳定 snake（`invalid_input` / `timeout` / `unavailable` / …） |
| SSRF 分类器 | 按 Q2 放宽 fake-ip；补测 |
| `contract_matrix` / availability 测 | 更新 surface 期望（Fixed 可留） |

### 5.2 教法 / SDK / 契约

| 项 | 动作 |
|---|---|
| `descriptions/webfetch.md`、`websearch.md` | 改 Code 用法（`tools.webfetch`）；写软失败分支 |
| PRELUDE | 类 edit：网络失败 `ok:false` 或不 catch 即崩的边界写清 |
| `run_code_sdk.{txt,sha256}` | Tools 刷 |
| `webfetch-ssrf.md` | 若放宽 198.18/策略，更新核实段 |
| 别名 | **无旧名可硬清**；保持 `webfetch`/`websearch` 即可（Q3） |

### 5.3 不碰

- `web/`；libs；LSP；合 main  
- Exa 商务/换提供商（除非软失败需要更稳的 parse）

---

## 6. mid 拍板 Q（≤5）

**Q1 — 软失败形状（HTTP/DNS/超时/Exa 业务）**  
A) **edit 风格**：一律 resolve `{ok:false, status?, message, …}`，不 reject（推荐：批处理友好，少打崩）  
B) **catchable `ToolError`**：稳定 `code`（`timeout`/`io`/`unavailable`…），文档强制 `try/catch` / `allSettled`  
C) 混合：4xx/空结果 → ok:false；5xx/超时/DNS → ToolError  
**建议：A**（与 brief「尽量不打崩」最贴；非法入参/未配置仍可用 ToolError）。

**Q2 — TUN / 私网默认**  
A) **放行 198.18/15（fake-ip），坚持拒 RFC1918/loopback/169.254/fe80**；100.64 另定  
B) 198.18 + 100.64 一并放行（利 Tailscale；扩大内网面）  
C) 加 Settings「允许非公网 / 信任系统代理」，默认仍拒  
D) 不改分类器，只文档「关 fake-ip / 用字面 IP」（不满足 Andy）  
**建议：A**；100.64 默认仍拒，除非产品明确要内网 fetch（可进 C）。

**Q3 — 工具名**  
A) 保持 `webfetch` / `websearch`（推荐）  
B) 改 snake 如 `web_fetch` / `web_search`（无收益，绑/教法噪音）  
**建议：A。**

**Q4 — 出参 data 形状**  
A) webfetch：`{ok, url, status?, format, body?, message}`；websearch：`{ok, query, text|results[], message}`  
B) 最小 `{ok, message, text?}` 两工具共用  
C) 成功不写 `ok`（仅失败带 ok:false）——与 edit 不一致  
**建议：A**（status 利于分支；body/text 分字段清）。

**Q5 — 未 warm / 未配置 / SSRF 拒绝 与 run_code**  
A) 均 `ok:false`（程序永不因网络工具 reject，除非协议错）  
B) 未 warm/未配置/SSRF → `ToolError`（`unavailable` / `permission_denied`）；仅 HTTP/DNS/超时软  
C) SSRF → 不可 catch 的程序失败（过激，不推荐）  
**建议：B**（配置/安全门应醒目；业务拉页失败应软）。

---

## 7. 结论（给方案用）

- **迁 Code**：两工具已是完整 Direct 实现 + 引擎；缺口是 `surface`、`data_schema`/结构化出参、SDK/PRELUDE、软失败出口——**比 WP-202 轻**。  
- **软失败**：今日全是 reject 型 error，进 Code 后不改就会用 404 打崩 `run_code`；应对齐 edit 的 `ok:false` 或明确 ToolError 边界（Q1/Q5）。  
- **TUN**：过度防御闸在 **`198.18.0.0/15`（及可选 `100.64/10`）SSRF 分类**；本环境 DNS 已证实 fake-ip。修分类或策略即可，不是「没网」也不是 reqwest 本身坏了。  
- **硬清**：Direct 两名消失；引擎卡与 `network_core` 身份可留；无别名包袱。

