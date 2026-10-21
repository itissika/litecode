# WP-203 方案 — webfetch / websearch 进 Code（软失败 + TUN）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** HEAD `b38ebc4`（WP-202 tip）  
**依据：** brief；Andy 2026-10-22；`dig-survey.md`  
**内存：** available ≥ 2GiB；中层串行放行 cargo

---

## 1. 目标契约（唯一活跃面）

两工具 **Code-only**（无 Direct）；名保持 `webfetch` / `websearch`；入参 object bag 不变：

```ts
tools.webfetch({ url, format? /* text|markdown|html，默认 markdown */ })
  -> { ok: true,  url, status, format, body, message }
  |  { ok: false, url?, status?, format?, message }   // 软失败：不 reject

tools.websearch({ query })
  -> { ok: true,  query, text, message }              // text=提供商压平正文；可另加 results[] 若底座已有
  |  { ok: false, query?, message }
```

- 成功与软失败同走 `ok_data`（resolve，**不**变成 `ToolError` reject）。
- `content` 可继续短确认/摘要；结构化以 `data` 为准（进 Code 必有 `data_schema`）。
- 引擎 id / Warm 卡保留；只迁调用面。

### 软失败 vs 硬错（Q1/Q5）

| 类 | 出口 | 例 |
|---|---|---|
| **业务/HTTP 软** | `{ok:false,…,message}` resolve | 4xx/5xx、超时、DNS、body/redirect 读失败、Exa 业务/HTTP/parse |
| **契约/配置/安全** | 可 catch `ToolError`（稳定 `error_code`） | 空/非法 url·query、非 http(s)、**SSRF 真非公网拒绝**、未 warm / 未配 Exa |
| **升程序失败** | 仅协议/沙箱/宿主 | 多余参数、嵌套过深、surface mismatch、host panic、run_code 预算 |

### TUN / SSRF（Q2）

- **放行** `198.18.0.0/15`（fake-ip / 常见 TUN）：主机名解析到该段不再当 SSRF 拒；连接仍钉本趟解析 IP（现有 redirect 重校验保留）。
- **坚持拒** RFC1918 / loopback / unspecified / link-local / fe80 / unique-local；**`100.64/10` 默认仍拒**（本包不加 Settings 开关）。
- 同步 `knowledge/contracts/tools/webfetch-ssrf.md` + `ssrf_*` 测；字面公网 IP 行为不变。

---

## 2. 硬清

| 砍 | 含 |
|---|---|
| Direct 活跃面 | 两工具 `surface → Code`；投影后 Direct 无名 |
| 硬 error 打崩 run_code | HTTP/DNS/超时等改 `ok:false`；禁默认 `ToolCallResult::error` 走业务路径 |
| 无结构 content-only | 补 `data_schema` + 稳定字段；填 `error_code`（硬错） |
| 旧 SSRF 误伤 TUN | 分类器放宽 198.18；契约/测对齐 |

无旧别名包袱。不改 libs / web / LSP。

---

## 3. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | surface=Code；`ok_data` 软失败；稳定 error_code；放宽 198.18；进 `builtin_code_surface_tools`（或等价 admission）；Direct 清零测；ssrf/软失败测；dig-runtime-impl；契约 md |
| **Tools** | descriptions + PRELUDE 软失败句 + SDK fixture 两名；扫 Direct/硬崩文案；dig-tools-teaching |
| **mid** | 串槽；交叉审；acceptance/diff-summary；commit/push；deliver |

顺序：Runtime → Tools → mid。cargo 独占，门槛 ≥2GiB。

---

## 4. 验收

1. 两工具 Code-only；Direct 清零；软失败可证（测：HTTP/DNS 等不打崩未 catch 的 run_code）。  
2. TUN：主机名→198.18 可过门（测或 dig 实证）；真私网/回环仍拒。  
3. 相关 cargo 绿；`artifacts/WP-203`；push 实验分支；**未合 main**；无 FE。

---

## 5. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 软失败形状：edit 风格 `{ok:false}` resolve（非默认 ToolError） | **是（A）** |
| Q2 | 放行 198.18/15；拒 RFC1918/loopback/metadata；100.64 仍拒 | **是（A）** |
| Q3 | 名保持 `webfetch` / `websearch` | **是** |
| Q4 | 出参 `{ok,url,status?,format?,body?,message}` / search 对等 | **是（A）** |
| Q5 | 未 warm/未配置/SSRF → ToolError；仅 HTTP/DNS/超时等软 | **是（B）** |

