# WP-105-research-c · WP-109 聚合网关 + Q4 新供应商

- 包：WP-109（owner=provider）；查阅日期：**2026-10-13 HKT**
- 无 key → 实测 **未测**；标签：`已核实` / `推断`

## G2 总表（范围内既有 + Q4 候选）

| 名称 | 类型 | 大陆能否直连 | 能否开发票 | 标签 | 证据 |
|---|---|---|---|---|---|
| opencode (Zen) | 聚合 PAYG | 不稳定/常需代理（服务定位国际；主机海外） | 推断难（海外卡支付；无中国增值税发票通道） | 推断（直连）；已核实（产品/价目） | [Zen](https://opencode.ai/docs/zen/) |
| opencode-go | 聚合订阅 | 官方写「primarily international」；大陆直连不稳定 | 同 Zen，推断不可开中国发票 | 已核实（国际定位）；推断（发票） | [Go](https://opencode.ai/docs/go/) |
| commandcode | 聚合 | 公司地址 SF；大陆直连不稳定 | 推断无中国发票；企业另议 | 推断 | [Provider](https://commandcode.ai/docs/provider) |
| OpenRouter | Q4 | 大陆通常需代理 | 海外主体，常规无中国发票 | 推断 | [openrouter.ai](https://openrouter.ai/) |
| Moonshot / Kimi 直连 | Q4 | 是（api.moonshot.cn） | 是（充值可开票；北京月之暗面，6%） | 已核实 | [Kimi API](https://platform.kimi.com/docs/api/overview)；[账号财务](https://platform.kimi.com/docs/guide/account-and-payments) |
| MiniMax 直连 | Q4 | 是 | 是（国内云厂商惯例；须控制台确认） | 推断（发票流程）；已核实（国内可访） | 官网 minimaxi.com API |
| 硅基流动 SiliconFlow | Q4 | 是 | 是（国内平台支持发票，须控制台确认档位） | 推断（开票细则）；已核实（国内聚合） | [siliconflow.com](https://www.siliconflow.com/) |
| Google Gemini | Q4 | 否（需境外） | 否（无中国增值税发票） | 推断 | Google AI Studio / Vertex |
| Anthropic | Q4 | 否 | 否 | 推断 | anthropic.com |
| xAI Grok | Q4 | 否 | 否 | 推断 | x.ai |

## 1. 仓库盘点

| provider | 行 | endpoint（catalog） | endpoint_type | 头 | 模型要点 |
|---|---|---|---|---|---|
| opencode | 78–83 | https://opencode.ai/zen/v1 | chat_completions（GPT 族模型覆盖为 responses） | x-opencode-session={{session_id}} | gpt-6.1-sol@288；gpt-6-luna@302；gpt-5.6-sol@314；grok-4.7@326；muse-spark-1.3@338；glm/deepseek/kimi @348–385 |
| opencode-go | 86–91 | https://opencode.ai/zen/go/v1 | 同上模式 | 同上 | deepseek@392；gpt-6-luna@404；grok@416；muse contributor@427；glm/kimi/mimo/hy4 @437–497 |
| commandcode | 94–100 | https://api.commandcode.ai/provider/v1 | chat_completions；部分模型 responses | provider 级 tiers low/medium→high/high→max | gpt-5.6-sol@502；gpt-6.1-sol@512；gpt-6-luna@524；xai/grok-4.7@536；zai-org/GLM-*@547+；moonshotai/Kimi-K3@567；deepseek/…@577；google/gemini-3.8-flash@589；xiaomi/mimo@600 |

## 2. Q1 既有聚合商差异

### 2.1 opencode（Zen）

| 项 | 官方 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| **Base URL** | 文档端点为 `https://opencode.ai/inference/v1/{responses\|chat/completions\|messages\|models/...}` | `https://opencode.ai/zen/v1` @80 | **路径前缀可能过时（zen vs inference）**——高优先级核实；若 zen 仍反代则无功能差，但应与官方对齐 | 已核实（两边文档/目录不一致） |
| 协议 | 按模型分流：GPT/Grok/Muse→responses；多数开源→chat/completions；Claude→messages；Gemini→Google 路径 | provider 默认 chat；GPT 族与 grok/muse 模型级 responses | **缺 Anthropic messages codec**（目录未挂 Claude）；Gemini 官方用特殊路径，commandcode 用 responses 挂 gemini——Zen 未挂 Gemini | 已核实 |
| 头 | Go 文档强调 `x-opencode-session`；Zen 控制台场景亦用 session | headers @83 | 保留正确 | 已核实 |
| 价格 | PAYG USD/1M，官方表（如 GLM-5.3 $1.4/$4.4；Kimi K3 $3/$15；GPT-5.6 Sol ≤272k $4/$20）；声称接近成本+卡费 | 无加价字段；**属聚合加成后的标价**（标「官方表价」） | 非上游裸价；有刷卡费 4.4%+$0.30 | 已核实 |
| 模型缺口 | 官方表远大于目录 9 条（含 Claude、Qwen、MiniMax、免费线路等） | 9 models | 可按产品需要扩容；Claude 需新 codec | 已核实 |
| 实测 | – | – | **未测** | – |

### 2.2 opencode-go

| 项 | 官方 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| Base | `https://opencode.ai/inference/go/v1/...` | `https://opencode.ai/zen/go/v1` @88 | 同 Zen，**zen vs inference** 前缀差异 | 已核实 |
| 套餐 | Go $10 / Go Plus $40 月；模型月限额不同；可回落 PAYG balance | 无价格 | – | 已核实 |
| session 头 | **必须**稳定 session 以优化路由/缓存；列明校验客户端 | @91 | 对齐 | 已核实 |
| 模型 | 含 Hy3/Hy4、MiMo、Claude Haiku、Qwen messages 路径等 | 10 条；无 Claude/Qwen messages；无 hy3 | 可补 hy3、mimo 已有；Claude 仍阻于 codec | 已核实 |
| 实测 | – | – | **未测** | – |

### 2.3 commandcode

| 项 | 官方 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| Base | https://api.commandcode.ai/provider/v1 | @96 | 对齐 | 已核实 | [Provider](https://commandcode.ai/docs/provider) |
| 协议 | chat/completions + responses + messages + systemone；模型带 supported_endpoints | 默认 chat；多模型覆盖 responses（GPT/Grok/Gemini/MiMo） | Anthropic `/messages`、systemone **未接** | 已核实 |
| 计价 | Provider 计划约 $15/月起 + 加工费；宣称 **no markup**；GOAT $10/月含额度等 | provider tiers @100 | 目录模型 id 带 org 前缀（zai-org/、moonshotai/）与官方示例 deepseek/… 风格一致 | 已核实 |
| ZDR | 头 x-cmd-zdr: 1 | 未建模 | 可选增强 | 已核实 |
| 流 usage | 文档称末包 usage **无需** opt-in；仍兼容 stream_options | chat codec 仍发 include_usage | 冗余无害 | 已核实 |
| 实测 | – | – | **未测** | – |

## 3. Q4 新供应商候选（≥5）

协议列：能否走现有 `responses` / `chat_completions` codec（不新增 Anthropic/Google 原生）。

| # | 候选 | 价格页 | 币种 | 协议兼容 | 大陆直连 | 发票 | 建议 | 标签 |
|---|---|---|---|---|---|---|---|---|
| 1 | **Moonshot / Kimi 直连** | [platform.kimi.com](https://platform.kimi.com/docs/api/overview) | CNY | chat_completions（OpenAI 兼容 api.moonshot.cn/v1） | 是 | 是 | **新增**优先（国内可直连可开票；Kimi K3 已在多家聚合） | 已核实 |
| 2 | **硅基流动 SiliconFlow** | siliconflow 定价页 / 控制台 | CNY/USD 标价混用 | chat_completions | 是 | 推断是 | **新增**（国内聚合，覆盖开源+部分闭源） | 已核实（可直连）；推断（发票） |
| 3 | **MiniMax 直连** | minimaxi 开放平台价目 | CNY | chat_completions（兼 Anthropic 兼容需另论） | 是 | 推断是 | **新增**（M2.7/M3 已在腾讯/方舟；直连可降依赖） | 推断 |
| 4 | **OpenRouter** | [openrouter.ai/models](https://openrouter.ai/models) | USD | chat_completions（统一）；部分 responses 视模型 | 否 | 否 | 可选（模型广）；G2 差，适合海外用户 | 推断（直连/票）；已核实（产品形态） |
| 5 | **Google Gemini 直连** | ai.google.dev 价目 | USD | 官方 generateContent ≠ 现有两 codec；Via OpenAI 兼容层有限 | 否 | 否 | **暂不做**直连（需新 codec 或依赖 commandcode 已挂的 responses 包装） | 推断 |
| 附 | Anthropic 直连 | anthropic.com/pricing | USD | messages API — **现有 codec 不够** | 否 | 否 | **不做**直至 Runtime 有 Anthropic codec | 已核实（协议） |
| 附 | xAI | x.ai/api | USD | 多走 OpenAI 兼容 / responses | 否 | 否 | 低优先（已能经 Zen/CommandCode 用 Grok） | 推断 |

### Q4 详细备注

- **Moonshot**：财务文档写明可按消耗或充值开票，企业认证抬头限制——G2 友好。
- **SiliconFlow**：国内访问友好；Kimi K3 等已上架（博客价例 $3/$15 per 1M）——作聚合备选，注意与直连 Moonshot 重复。
- **OpenRouter**：加价/路由差异大；大陆网络与发票均弱——仅当目标用户在海外时考虑。
- **Gemini/Anthropic**：官方协议超出 Provider 当前两 codec；CommandCode/Zen 已提供部分间接路径。

## 4. Q2 / Q3（聚合侧）

| provider | 可升级 | 新模型 | 建议 |
|---|---|---|---|
| opencode | **核实并迁移 endpoint 前缀 zen→inference**；评估 Claude 需求（Runtime） | 官方大量免费/付费未进目录 | 先修 base URL；再挑性价比模型 |
| opencode-go | 同上 inference/go；补 hy3 | 官方列表更全 | 同左 |
| commandcode | 可选 ZDR 头；模型 id 与 /models 同步 | gemini-3.8-flash 已在目录 | 定期 curl /models 对账（无 key 则文档对账） |

## 5. 建议汇总

| 动作 | 项 |
|---|---|
| 升级 | opencode / opencode-go base URL 与官方 `inference` 前缀对齐（**先探测 zen 是否仍 200**） |
| 新增 | Moonshot 直连、SiliconFlow、MiniMax 直连（G2 优） |
| 不做 | Anthropic/Gemini 原生直连（缺 codec） |
| 不做 | 以 OpenRouter 作为国内默认 |
| 未测 | 全部 live；endpoint 探测建议放 `.tmp/wp106/` 无 key HEAD/GET models |

## 6. 待 Andy/GM 裁定

- 是否接受海外聚合（Zen/Go/CommandCode）无中国发票作为「可继续卖」条件。
- Q4 首发直连优先级：Moonshot vs SiliconFlow vs MiniMax。
- 是否立项 Anthropic messages codec（Runtime）。

## 7. 无 key 探测记录（.tmp 外只读 GET）

查阅日期 2026-10-13 HKT。对 `/models` 无鉴权 GET：

| URL | 结果 |
|---|---|
| https://opencode.ai/zen/v1/models | HTTP 403 |
| https://opencode.ai/inference/v1/models | HTTP 403 |
| https://opencode.ai/zen/go/v1/models | HTTP 403 |
| https://opencode.ai/inference/go/v1/models | HTTP 403 |

结论：无 key 无法区分 zen/inference 哪个仍为生产入口；**未测**（需 key）。官方文档以 `inference` 为准，目录仍写 `zen`——差异保留为升级项。
