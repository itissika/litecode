# WP-110-backend · WP-111 方案（backend_mid → Provider）

- 父：WP-110（gm，Andy 已批）；本包：WP-111（split，approver=gm）；子：WP-113（provider，approver=backend_mid）。
- 时间：2026-10-13 20:2x HKT。落地 WP-105/106 裁定；**改完绿测，不 commit**；总包封完由 GM 推。
- 前端 WP-112 并行（plan 一组 / api 按量一组）；后端只管 catalog/config，不改 web UI。

## 1. 范围（做）
1. **tencent-token**：在 `default_catalog.toml` 补挂 `kimi-k3`、`mimo-v2.6-flash`（**不加** `minimax-m3`）。对齐腾讯 Token Plan 文档模型名/上下文/协议；可参考同仓 ark-coding / opencode-go 已有同名条目的能力字段，再按腾讯端点核对。
2. **新增 Moonshot、MiniMax**：
   - 查官方是否有 **plan（套餐）+ 按量** 两套端点；有则拆成两个 provider（命名与 ark-coding / ark-agent 同类：一家 plan、一家 payg），无则只建按量一家。
   - 协议走现有 `chat_completions`（或官方明确的 responses）；auth bearer；不新增 codec。
   - 模型：首批挂官方主力（Kimi K3 / MiniMax M2.7 或 M3 等，以官方当前列表为准）；`context_window`/`max_output`/`reasoning`/`quirks` 有据再填。
   - `src/config` 中 provider key / 选型若需登记新 id，只改 provider 相关段（resolved/settings_writer/bridge），保持与现有 ark 双套登记方式一致。
3. **OpenCode / OpenCode-Go base**：
   - 先探 `zen` 是否仍可用（有 key 再探；无 key 按官方文档迁到 `inference` / `inference/go`，并在产物记 **未测**）。
   - 保留 `x-opencode-session` 头。

## 2. 不做
- MiMo 改协议（仍 responses）
- 硅基流动
- Anthropic codec / Gemini 直连
- 前端分组（WP-112）
- commit / 推送 / 动 `windows-*.yml`

## 3. 切分
| 包 | owner | 工作 |
|---|---|---|
| WP-111 | backend_mid | 方案、批 WP-113、抽测、汇总 WP-110-backend.md、deliver |
| WP-113 | provider | 查官方 → 改 catalog/config → 绿测 → 写 WP-110-provider.md |

## 4. 执行步骤（批后）
1. GM 批 WP-111 → `start` WP-111 → Provider fill/submit WP-113 → 中层批 → Provider `start`。
2. Provider：官方文档核对（链接+HKT 日期）→ 改 `src/provider_catalog/default_catalog.toml`（+ 必要 config）→ `cargo fmt` / 相关测绿（catalog resolve、provider 配置测；中层复跑抽样）。
3. OpenCode：探测记录写入产物；无 key 则文档迁 + 标未测。
4. 中层抽测 + 交叉：与 WP-112 对齐新 provider id（plan vs api 命名），私信前端中层一次。
5. WP-113 deliver → 中层 close → 汇总 → WP-111 deliver → GM。

## 5. 验收 / done_when
- tencent-token 已有 kimi-k3、mimo-v2.6-flash；无 minimax-m3 新增
- Moonshot、MiniMax 已进目录；有 plan 则双套端点，命名可区分 plan/按量
- OpenCode/Go endpoint 已对齐官方或文档迁并记未测
- 相关测绿；`cargo fmt --check` 干净；未 commit
- 产物：WP-110-backend.md（汇总）+ WP-110-provider.md（执行明细+链接）

## 6. 风险
- 官方 plan 端点不存在或文档含糊 → 只建按量，产物写明
- 无 OpenCode key → 无法实测 zen vs inference，按文档迁
- 模型 id 与腾讯侧大小写/路径不一致 → 以官方模型表为准并在产物附链接
