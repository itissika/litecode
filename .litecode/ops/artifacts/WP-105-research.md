# WP-105-research · 供应商与模型更新调研（WP-106 方案 + 发现骨架）

- 角色：backend_mid；父 WP-105（gm，approver=andy）；本包 WP-106（split，approver=gm）。
- 规则：只读调研；不改 `src/`、`tests/`；脚本（若有）只放 `.tmp/wp106/`；不 commit；不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13 19:5x HKT。

## 1. 目标
对每个已支持供应商查官方文档：Q1 协议/模型/端点/请求响应格式更新及与仓库实现的差异；Q2 现有模型可升级能力（推理、缓存、工具调用、上下文长度）；Q3 现有供应商的新性价比模型；Q4 值得新增的其他性价比供应商。最终产出差异清单与 新增/升级/不做 建议。

## 2. 仓库盘点（中层已核，2026-10-13）
- 没有 `src/providers`。供应商目录：`src/provider_catalog/default_catalog.toml`（11 个 provider、62 个 model 条目）；加载/解析 `src/provider_catalog/{mod,store,resolve}.rs`；key 与选型 `src/config/{resolved,settings_writer,bridge}.rs`。
- 线协议只有两种：`src/llm/codec/mod.rs:31-33` 按 `endpoint_type` 分 `responses`（`codec/responses.rs` + `responses_harden.rs`）与 `chat_completions`（`codec/chat.rs`、`chat_synth.rs`、`chat_usage.rs`）；SSE `codec/sse.rs`；推理回放 `codec/replay.rs`、`llm/replay_compat.rs`；重连 `llm/reconnect.rs`。
- 目录用到的能力字段：`reasoning.tiers/summary/replay`、`context_window(_max)`、`max_output`、`modalities`、`usage_patch`、`extra_body`、`quirks`（`reasoning_replay`、`thinking_type_switch`）、`headers`、auth 全为 bearer。

| provider | 行 | endpoint | endpoint_type | quirks | 模型数 |
|---|---|---|---|---|---|
| openai | src/provider_catalog/default_catalog.toml:42 | https://api.openai.com/v1 | responses | – | 1（gpt-5.6-sol） |
| deepseek | src/provider_catalog/default_catalog.toml:49 | https://api.deepseek.com | responses | reasoning_replay | 1（deepseek-flash） |
| mimo | src/provider_catalog/default_catalog.toml:59 | https://api.xiaomimimo.com/v1 | responses | reasoning_replay | 2 |
| ark-coding | src/provider_catalog/default_catalog.toml:68 | ark.cn-beijing.volces.com/api/coding/v3 | responses | thinking_type_switch | 7 |
| opencode | src/provider_catalog/default_catalog.toml:78 | opencode.ai/zen/v1 | chat_completions | 头 x-opencode-session | 9 |
| opencode-go | src/provider_catalog/default_catalog.toml:86 | opencode.ai/zen/go/v1 | chat_completions | 同上 | 10 |
| commandcode | src/provider_catalog/default_catalog.toml:94 | api.commandcode.ai/provider/v1 | chat_completions | 自有 tiers | 10 |
| zhipu | src/provider_catalog/default_catalog.toml:106 | open.bigmodel.cn/api/paas/v4 | chat_completions | – | 2 |
| ark-agent | src/provider_catalog/default_catalog.toml:116 | ark.cn-beijing.volces.com/api/plan/v3 | responses | thinking_type_switch | 9 |
| tencent-token | src/provider_catalog/default_catalog.toml:127 | api.lkeap.cloud.tencent.com/plan/v3 | chat_completions | – | 7 |
| aliyun-token | src/provider_catalog/default_catalog.toml:136 | token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1 | responses | – | 4 |
执行者须把每个模型条目补到 file:line 级。

## 3. 执行者建议（GM 裁定项 G1）
- 现状：`src/llm` 归 Runtime，Runtime 正做 WP-098（WP-095），排队会拖。目录在 `src/provider_catalog` + `src/config` 的模型配置部分，对应已有产品域 `model-config`（board 中 owner_agent 为空）。
- **建议**：GM 新设 **Provider** 成员（agent id 建议 `provider`），担 `model-config` 域：
  - 范围：`src/provider_catalog/**`、`src/llm/codec/**` 中与供应商差异相关的适配（quirks、extra_body、usage_patch）、`src/config` 的模型/key 配置部分（resolved/settings_writer/bridge 中 provider 段）。本调研内**只读**。
  - 与 Runtime 边界：Runtime 保留 `src/llm/{provider,request,reconnect}.rs` 的流式/重试/turn 集成与 codec 公共框架（`codec/mod.rs`、`sse.rs`、`stream_contract.rs`）；Provider 只动供应商条目与差异适配；公共框架改动须 Runtime 审。
- 工具限制：`wp new --owner provider` 校验失败（agent 不存在），无法用占位 owner。故三个子包暂建为 owner=runtime 并**排在 WP-098 之后**；若 GM 批 G1，GM 新增 agent 后由 GM 取消这三个包、按同样范围在 provider 名下重建（或 GM 用其权限改 owner）。

## 4. 子包（按供应商组切分）
| 包 | 组 | 额外问题 | 产物 |
|---|---|---|---|
| WP-107 | A 一方直连：openai、deepseek、mimo、zhipu | 两种 codec 与 OpenAI Responses / Chat Completions 官方规范逐项对照（流事件、tool call、usage、reasoning 字段、prompt cache） | WP-105-research-a.md |
| WP-108 | B 国内云套餐：ark-coding、ark-agent、tencent-token、aliyun-token | 套餐计价、限额、可用模型清单变更 | WP-105-research-b.md |
| WP-109 | C 聚合/网关：opencode、opencode-go、commandcode；+ Q4 新供应商 | Q4：≥5 个候选（如 OpenRouter、Moonshot 直连、MiniMax 直连、Google Gemini、Anthropic、xAI、硅基流动等，执行者定），价格页链接、协议兼容性（能否走现有两种 codec） | WP-105-research-c.md |
顺序：三包可并行（只读，互不冲突）；若仍由 Runtime 执行，统一排在 WP-098 deliver 之后。

## 5. 证据要求
- 每条结论：官方文档链接 + 访问日期（HKT）+ 标签 `已证实`（官方文档原文/实测请求）或 `推断`。
- 仓库对照：file:line。
- 若实测请求（需 key）：只在 `.tmp/wp106/`，不落 key，不提交；无 key 则标"未实测"。
- 价格：官方价格页链接 + 币种 + 日期；聚合商标明是否加价。

## 6. 发现（子包回填，中层合并）
查阅日期统一 **2026-10-13 HKT**。子包产物：`WP-105-research-a.md`（WP-107）、`WP-105-research-b.md`（WP-108）、`WP-105-research-c.md`（WP-109）。无 API key → 全部实测标 **未测**。中层抽查 catalog/codec file:line 对齐后 seal。

### 6.1 各供应商 Q1 差异（摘要）
| provider | 关键差异 | 标签 | 详见 |
|---|---|---|---|
| openai | 仅挂 gpt-5.6-sol；官方已有 Astra/Luna/6.1 等；encrypted_content include 冗余无害 | 已核实 | a §2.1 |
| deepseek | 对齐 Responses + deepseek-flash；无 encrypted；summary 不生成 | 已核实 | a §2.2 |
| mimo | 官方深思主推 chat+reasoning_content；仓库走 responses → **协议偏好冲突** | 推断（Responses 覆盖未确认） | a §2.3 |
| zhipu | paas/v4 chat 对齐；Coding Plan 独立端点未建 | 已核实 | a §2.4 |
| ark-coding / ark-agent | 端点对齐；thinking_type_switch **仍需要**；chat 第三方 quirks=[] 正确 | 已核实 | b §2.1–2.2 |
| tencent-token | 端点对齐；缺 kimi-k3 / minimax-m3 / mimo-v2.6-flash；缺 hy3 | 已核实 | b §2.3 |
| aliyun-token | 北京 compatible-mode + summary 回放对齐；Coding Plan 不做回退 | 已核实 | b §2.4 |
| opencode / opencode-go | 官方文档 base 为 `inference`；目录仍为 `zen`；无 key 两路径均 403 | 已核实（文档差）；未测（探测） | c §2.1–2.2 / §7 |
| commandcode | base 对齐；缺 Anthropic messages / systemone；可选 ZDR 头 | 已核实 | c §2.3 |
| codec Responses | keepalive 丢弃、function_call、encrypted/replay、thinking_type_switch 分支对齐；DeepSeek reasoning_text 映射待 Runtime 确认 | 混 | a §3.1 |
| codec Chat | include_usage、tool_calls、reasoning_content 三键、usage 别名对齐 | 已核实 | a §3.2 |

### 6.2 Q2 可升级能力
- openai：扩模型矩阵；reasoning.mode/context 需 Runtime 字段
- deepseek：确认 reasoning_text.* 事件消费；可选 v4-pro
- mimo：评估改 chat_completions 或双协议
- zhipu：可选独立 Coding Plan provider
- 国内云：tencent 补上架模型；ark 维持 thinking_type_switch
- 聚合：先对齐 OpenCode base 前缀；Claude 需 Anthropic codec（Runtime）

### 6.3 Q3 同家新模型
| 建议新增 | 来源 | 标签 |
|---|---|---|
| openai gpt-6-astra / gpt-6.1-sol / Luna·Terra | a §5 | 已核实（文档列出） |
| deepseek-v4-pro | a §5 | 已核实 |
| tencent：kimi-k3、minimax-m3、mimo-v2.6-flash（及可选 hy3） | b §3 | 已核实 |
| opencode(-go) 按官方表挑性价比补缺 | c §4 | 已核实 |

### 6.4 Q4 新供应商候选（≥5）
| 优先级建议 | 候选 | 协议 | 大陆直连 | 发票 | 建议 |
|---|---|---|---|---|---|
| 1 | Moonshot/Kimi 直连 | chat_completions | 是 | 是 | **新增** |
| 2 | 硅基流动 SiliconFlow | chat_completions | 是 | 推断是 | **新增** |
| 3 | MiniMax 直连 | chat_completions | 是 | 推断是 | **新增** |
| 4 | OpenRouter | chat（部分 responses） | 否 | 否 | 可选（海外用户） |
| 5 | Google Gemini 直连 | 非现有两 codec | 否 | 否 | **暂不做** |
| 附 | Anthropic / xAI | messages / 兼容层 | 否 | 否 | Anthropic **不做**直至有 codec；xAI 低优先（已能经聚合用） |

### 6.5 差异清单（汇总）
1. MiMo 仓库 responses vs 官方 chat 深思文档冲突（高）
2. OpenCode zen vs inference base 前缀（高；无 key 未测通）
3. tencent-token 模型清单落后官方 2026-09-30 文档（中）
4. openai 单模型目录落后官方多 snapshot（中）
5. 聚合商缺 Anthropic messages codec，Claude 无法直挂（中，归 Runtime）
6. thinking_type_switch 必须保留（已核实）

### 6.6 建议：新增 / 升级 / 不做
| 动作 | 项 |
|---|---|
| 升级 | OpenCode/Go base 与官方 `inference` 对齐（先探测 zen 是否仍可用） |
| 升级 | tencent-token 模型列表；openai 模型矩阵 |
| 升级 | 核实 MiMo 协议；必要时改 chat_completions |
| 新增 | Moonshot、SiliconFlow、MiniMax 直连（G2 友好） |
| 新增 | deepseek-v4-pro（可选） |
| 不做 | DeepSeek encrypted include；删 thinking_type_switch；复活阿里云 Coding Plan |
| 不做 | Anthropic/Gemini 原生直连（缺 codec）；以 OpenRouter 作国内默认 |
| 未测 | 全部 live 调用 |

### 6.7 待 Andy/GM 裁定（高优先级）
1. **MiMo** 是否强制改 `chat_completions`（或双 endpoint）。
2. **OpenCode** base 是否迁到 `inference`（zen 是否仍生产入口）。
3. **Q4 首发**优先级：Moonshot vs SiliconFlow vs MiniMax。
4. 海外聚合（Zen/Go/CommandCode）无中国发票是否仍可继续作为默认卖点。
5. 是否立项 Anthropic messages codec（Runtime）。

### 6.8 G2 总览（大陆直连 / 发票）
- 友好：deepseek、mimo、zhipu、ark-*、tencent-token、aliyun-token、Moonshot、SiliconFlow、MiniMax
- 不友好/不稳定：openai、opencode、opencode-go、commandcode、OpenRouter、Gemini、Anthropic、xAI
- 详见 a/b/c 各文 G2 总表

## 7. 合并计划
已完成：子包 deliver → 中层审证据 → 合并 §6 → WP-106 deliver。

## 8. 不在本包
任何目录/代码改动；接入新供应商；key 管理；UI。

## 9. GM 裁定状态
- G1：已同意，Provider 成员已建（id e310cad5-aff8-488e-837b-1c950e26305c），三子包已由 provider 执行并 done。
- G2：不限定范围；每家已注明大陆直连与发票（见 §6.8）。

## 10. 验收
- [x] WP-107/108/109 done；结论带链接+日期+标签（抽查通过）
- [x] 差异清单 + 新增/升级/不做 建议齐全（§6.5–6.6）
- [x] 未改 src/tests；未 commit；validate_all 交付时再跑

## 11. Andy 裁定（2026-10-13 HKT）
1. MiMo：不改，继续 responses。
2. OpenCode base：按 GM 收敛（迁 inference，先探 zen）。
3. 腾讯 token：加上 kimi-k3、mimo-v2.6-flash（minimax-m3 本次不加）。
4. 新 provider：只加 Moonshot、MiniMax；两家可能有 plan，走双套端点（plan + 按量）。
5. 海外聚合无发票：按 GM 收敛（留选项、非国内默认卖点）。
6. Anthropic messages：暂不立项。
7. 前端：provider 多了要分组——plan 一组、api 按量一组；其余不变。
