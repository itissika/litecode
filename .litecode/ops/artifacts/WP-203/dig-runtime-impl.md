# WP-203 Runtime 实现笔记 — webfetch / websearch → Code + 软失败 + TUN 198.18

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `b38ebc4`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`（Q1A/Q2A/Q4A/Q5B）、`dig-survey.md`；mid 放行 cargo  
> **硬约束遵守：** cargo 见下；**未 commit/push**；**未合 main**；**未改** `web/` / libs / LSP / `run_code_sdk.*`；Tools 教法/SDK **未刷**（artifacts 草稿已有）

---

## 1. 关闭/替换层（一句话）

`webfetch` / `websearch` 均为 **`ToolSurface::Code`**；HTTP/DNS/超时/Exa 业务走 **`ok_data` `{ok:false}` resolve**；非法入参 / 非 http(s) / 真 SSRF / 未 warm·未配 Exa 走 **`ToolError` + 稳定 `error_code`**。SSRF 门 **放行 `198.18.0.0/15`**（TUN fake-ip），**坚持拒** RFC1918/loopback/link-local/fe80/ULA/`100.64/10`。引擎卡 / Warm id 保留。

---

## 2. 软失败 vs 硬错边界

| 类 | 出口 | webfetch 例 | websearch 例 | `error_code` |
|---|---|---|---|---|
| **业务/HTTP 软** | `{ok:false,…,message}` resolve（非 Error level） | 4xx/5xx、DNS、timeout、request/body/redirect 读失败、过多 redirect | Exa HTTP/业务/parse（`exa_mcp::search` Err） | — |
| **契约/配置/安全硬** | `coded_error` → catchable `ToolError` | 空/非法 url、非 http(s)、SSRF 真非公网、未 warm / warm 失败 | 空/缺 query、未 warm、未配置 Exa、warm 失败 | `invalid_input` / `permission_denied` / `unavailable` |
| **升程序失败** | 协议/沙箱/宿主（未改） | 多余参数、嵌套过深、surface mismatch、`run_code` 预算 | 同左 | 协议层 |

### 出参形状（Q4A）

```ts
webfetch → { ok, url?, status?, format?, body?, message }  // success: 全有；soft: 无 body
websearch → { ok, query?, text?, message }                 // 未加 results[]（底座 Exa 仍压平 text）
```

`content` 空（与 edit 一致）；结构化以 `data` 为准；`data_schema` 已补。

---

## 3. TUN / 198.18 证据

| 测 | 结果 |
|---|---|
| `ssrf_allows_benchmarking_fake_ip_range` | 字面 `198.18.0.1` / `198.19.255.255` / `::ffff:198.18.0.1` **过门** |
| `resolve_allows_literal_fake_ip_through_gate` | resolve 返回钉死用 IP |
| `ssrf_allows_hostname_resolved_to_fake_ip` | 本机 DNS：`example.com` → `198.18.0.1`；`resolve_and_validate` **Ok** |
| `client_pins_hostname_to_fake_ip` | `resolve_to_addrs` 接受 198.18 pin |
| `ssrf_rejects_loopback_and_private_ips` | RFC1918/loopback/169.254/fe80/ULA/`100.64` **仍拒** |
| `ssrf_shared_cgnat_still_non_public` | `100.64/10` 分类仍非公网 |
| `private_ip_is_hard_permission_denied` | SSRF → `ToolError` `permission_denied`（**非** ok:false） |

分类器：`is_non_public_ipv4` **不再**调用 `is_ipv4_benchmarking`；后者仅 `#[cfg(test)]` 辅助。`is_ipv4_shared` 掩码改为 `(octets[1] & 0xc0) == 0x40`（正确 `/10`）。

契约：`knowledge/contracts/tools/webfetch-ssrf.md` 已同步（放行 198.18；拒 100.64）。

---

## 4. Direct 清零 / Code 准入

| 证据 | 说明 |
|---|---|
| `surface() == Code` | `webfetch` / `websearch` 均 override |
| `builtin_code_surface_tools()` | 冷 stub 两名（`RwLock::new(None)`）进 admission 集 |
| `webfetch_websearch_project_to_code_not_direct` | `project_tools` → Direct **空**；whitelist 含两名；有 host 时顶层仅 `run_code`，bindings 含两名 |
| `catalog_payload_…` | Settings 卡 `webfetch`/`websearch` stamp 为 **`CallSurface::Code`**（原 Direct 断言已改） |
| rg | `ToolCallResult::error` 在两工具业务路径 **零命中**（硬错走 `coded_error`） |

无 code host 时投影仍会把 Code 工具 fallback 到 Direct 列表（既有行为）；**有 host** 时 Direct 无名。

---

## 5. 改动文件（Runtime）

| 文件 | 动作 |
|---|---|
| `src/tools/webfetch.rs` | surface=Code；`data_schema`；软/硬分类；198.18 放行；结构化出参；测扩充 |
| `src/tools/websearch.rs` | 同上（无 SSRF）；Exa Err → soft |
| `src/tool/registry.rs` | `builtin_code_surface_tools` + 两名；Direct 清零测 |
| `src/tool/availability.rs` | catalog surface 期望：两名 → Code |
| `knowledge/contracts/tools/webfetch-ssrf.md` | TUN/198.18 定稿对齐 |

**未改：** `web/`；`descriptions/*.md` live；PRELUDE；`run_code_sdk.*`；engines Warm 卡 id。

---

## 6. Cargo（tee）

| 日志 | 内容 |
|---|---|
| `artifacts/WP-203/cargo-webfetch.txt` | webfetch 模块（含 ssrf_* / hostname→198.18） |
| `artifacts/WP-203/cargo-websearch-registry.txt` | websearch + catalog + unbound_webfetch |
| `artifacts/WP-203/cargo-project-registry-avail.txt` | project/registry/availability 套件 + Direct 清零测 |
| `artifacts/WP-203/cargo-final-wp203.txt` | **终跑：34 passed / 0 failed**（filter 下 webfetch+websearch+相关） |

`CARGO_BUILD_JOBS=2`；available ≈10Gi；exit 0。

---

## 7. 残留（Tools / mid）

| 项 | 谁 |
|---|---|
| live `descriptions/webfetch.md` / `websearch.md` ← artifacts `draft-*.md` | **Tools** |
| PRELUDE 软失败句（类 edit：网络 `ok:false` 不 reject） | **Tools** |
| `run_code_sdk.{txt,sha256}` 刷入两名 | **Tools** |
| dig-tools-teaching 收口叠 live | **Tools** |
| 交叉审 / acceptance / commit·push 实验分支 | **mid** |

**Runtime 齐：可 mid「放收口」。未 commit / 未合 main / 未改 web/。**
