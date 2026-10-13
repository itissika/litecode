# WP-110-provider · WP-113 执行明细

- 包：WP-113（owner=provider）；父 WP-111；查阅/落地日期：**2026-10-13 HKT**
- 规则：已改 catalog（+ 相关测）；**未 commit**；未动 `windows-*.yml`；无 key → OpenCode 探测 **未测**

## 最终 provider id（给 WP-112）

| id | 显示名 | 类型 | endpoint | endpoint_type |
|---|---|---|---|---|
| `moonshot-coding` | Kimi Code | plan（订阅） | `https://api.kimi.com/coding/v1` | chat_completions |
| `moonshot` | Moonshot | payg（按量） | `https://api.moonshot.cn/v1` | chat_completions |
| `minimax-token` | MiniMax Token Plan | plan（M Plan / 订阅 Key） | `https://api.minimax.cn/v1` | chat_completions |
| `minimax` | MiniMax | payg（按量 Key） | `https://api.minimax.cn/v1` | chat_completions |

既有改动（非新建）：
- `tencent-token`：补模型 `kimi-k3`、`mimo-v2.6-flash`（**未**加 `minimax-m3`）
- `opencode` / `opencode-go`：base 由 `zen` / `zen/go` 迁到 `inference` / `inference/go`；保留 `x-opencode-session`

说明：MiniMax plan 与 payg **官方 OpenAI 兼容 base 相同**，但订阅 Key 与按量 Key **不可混用**，故拆成两个 provider（与 ark 双套「分 key」一致）。config 侧凭据按 catalog `provider_id` 动态登记，无需硬编码新表。

## 1. tencent-token

| 项 | 结论 | 标签 | 证据 |
|---|---|---|---|
| 官方模型表含 kimi-k3、mimo-v2.6-flash | 已挂入 catalog | 已核实 | [Token Plan 个人版套餐](https://cloud.tencent.com/document/product/1823/130060)（2026-10-13 HKT） |
| 不加 minimax-m3 | catalog 无 `tencent-token/minimax-m3`；测断言 `is_none` | 已核实 | 产品裁定 WP-110-backend；`tests.rs` seed_gateway… |
| 能力字段 | kimi-k3 对齐 ark-coding/opencode-go（max_output 32768，tiers low/high/max）；mimo-v2.6-flash 对齐 opencode-go（off=none） | 推断（能力镜像）+ 已核实（腾讯 model id） | catalog + 腾讯文档 Model ID 列 |

## 2. Moonshot / Kimi

| 项 | 结论 | 标签 | 证据 |
|---|---|---|---|
| 双端点 | Code 订阅 `api.kimi.com/coding/v1` vs 开放平台 `api.moonshot.cn/v1` | 已核实 | [Kimi Code 概览](https://www.kimi.com/code/docs/)；[开放平台 API](https://platform.kimi.com/docs/api/overview)（2026-10-13 HKT） |
| 协议 | 均走 OpenAI 兼容 Chat Completions（本仓 `chat_completions`）；未建 Anthropic | 已核实 | 同上 |
| plan 模型 | `k3`、`kimi-for-coding` | 已核实 | [Kimi Code 模型](https://www.kimi.com/code/docs/kimi-code/models) |
| payg 模型 | `kimi-k3` | 已核实 | [列出模型 / K3](https://platform.kimi.com/docs/api/list-models) |
| modalities | chat codec 不实现 image → 目录标 `["text"]` | 已核实 | resolve 校验；`opencode-go` 同模式 |

## 3. MiniMax

| 项 | 结论 | 标签 | 证据 |
|---|---|---|---|
| 双套 Key | M Plan 订阅 Key vs 按量 API Key；base 同为 `https://api.minimax.cn/v1` | 已核实 | [OpenAI SDK](https://platform.minimaxi.com/docs/api-reference/text-openai-api)；[Token Plan / M Plan](https://platform.minimaxi.com/docs/token-plan/intro)（2026-10-13 HKT） |
| 拆 provider | `minimax-token` + `minimax` | 已核实（决策） | 与 ark 双 key 模式一致；端点相同但 key 隔离 |
| 模型 | `MiniMax-M3`、`MiniMax-M2.7`（官方大小写） | 已核实 | 同上 OpenAI SDK 模型表 |
| reasoning | 暂不声明 tiers（chat 无 thinking.type 开关；M2.7 官方思考常开） | 推断 | OpenAI 兼容页 thinking 说明 |

## 4. OpenCode / OpenCode-Go

| 项 | 结论 | 标签 | 证据 |
|---|---|---|---|
| 官方 base | Zen：`https://opencode.ai/inference/v1`；Go：`…/inference/go/v1` | 已核实 | [Zen](https://opencode.ai/docs/zen/)；[Go](https://opencode.ai/docs/go/)（2026-10-13 HKT） |
| 仓库变更 | catalog endpoint 改为 inference / inference/go | 已核实 | `default_catalog.toml` |
| session 头 | 仍 `x-opencode-session = "{{session_id}}"` | 已核实 | catalog headers；Go 文档要求 |
| live 探测 | 环境无 OPENCODE key；未区分 zen vs inference 是否仍 200 | **未测** | `.tmp/wp113/opencode-probe.txt` |
| 历史 fixture | `src/config/global_db/legacy.rs` 仍含旧 zen URL（迁移测历史数据） | 已核实 | 故意不改 |

## 5. 验证

| 检查 | 结果 |
|---|---|
| `cargo test --lib provider_catalog::` | 33 passed |
| `cargo test --lib config::settings_writer::` / `bridge::` / `resolved::` | 全绿 |
| `cargo fmt --check` | 干净 |
| `config::path::tests::strip_makes_verbatim_prefix_compatible` | 失败（Windows verbatim 路径，与本包无关，Linux 既有） |
| commit | **无** |

## 6. 改动文件

- `src/provider_catalog/default_catalog.toml`
- `src/provider_catalog/tests.rs`
- `.litecode/ops/artifacts/WP-110-provider.md`（本文件）
- `.tmp/wp113/opencode-probe.txt`（未测记录）

## 7. 未做（范围外）

MiMo 改协议、硅基流动、Anthropic codec、Gemini 直连、前端分组、commit。
