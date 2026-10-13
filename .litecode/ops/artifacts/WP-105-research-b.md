# WP-105-research-b · WP-108 国内云套餐

- 包：WP-108（owner=provider）；查阅日期：**2026-10-13 HKT**
- 无 key → 实测全部 **未测**；标签：`已核实` / `推断`

## G2 总表

| provider | 大陆能否直连 | 能否开发票 | 标签 | 证据 |
|---|---|---|---|---|
| ark-coding | 是 | 是（火山引擎费用中心发票管理，普票/专票） | 已核实 | [开票指南](https://www.volcengine.com/article/2810622)；端点 [Coding 指南](https://www.volcengine.com/article/2810162) |
| ark-agent | 是 | 是（同火山引擎发票体系） | 已核实 | [Agent Plan](https://www.volcengine.com/activity/agentplan) |
| tencent-token | 是 | 是（费用中心发票管理；企业可专票） | 已核实 | [Token Plan](https://cloud.tencent.com/document/product/1823/130060)；[发票](https://cloud.tencent.com/document/product/555/7434) |
| aliyun-token | 是（仅华北2 北京） | 是（百炼/通义云启主体，6% 软件服务费等） | 已核实 | [Token Plan 概述](https://help.aliyun.com/zh/model-studio/token-plan-overview)；[发票 FAQ](https://help.aliyun.com/zh/user-center/support/faq-about-invoice-requests) |

## 1. 仓库盘点

| provider | catalog 行 | endpoint | endpoint_type | quirks | 模型数 / 代表 id@行 |
|---|---|---|---|---|---|
| ark-coding | 68–75 | …/api/coding/v3 | responses（第三方模型可覆盖 chat） | thinking_type_switch | doubao-seed-2.1-turbo@203；pro@214；lite@226；glm-5.3@240；glm-5.3-flash@252；kimi-k3@263；deepseek-v4.1-flash@275 |
| ark-agent | 116–121 | …/api/plan/v3 | responses | thinking_type_switch | doubao-seed-2.1-pro@637；lite@648；evolving@659；2.0-mini@670；glm-5.3@682（quirks=[]）；…；minimax-m3@730 |
| tencent-token | 127–131 | api.lkeap.cloud.tencent.com/plan/v3 | chat_completions | – | glm-5.3@743；flash@753；deepseek/deepseek-flash@764；minimax-m2.7@774；hy4-preview@783；auto@792；tc-code-latest@800 |
| aliyun-token | 136–140 | token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1 | responses | – | qwen3.8-max@813；flash@824；deepseek-v4.1-flash@835；glm-5.3@846（均 replay=summary） |

`thinking_type_switch` 编码：`responses.rs:131-140`；单测 `ark_uses_the_thinking_switch_and_store_extra_body` @776+。

## 2. Q1 / 套餐计价与限额

### 2.1 ark-coding（火山方舟 Coding Plan）

| 项 | 官方 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点 | OpenAI 兼容 `https://ark.cn-beijing.volces.com/api/coding/v3`（与 Agent `/api/plan/v3` 分离；Key 不可混用） | endpoint @70 | 对齐 | 已核实 |
| 协议 | Doubao 走 Responses + thinking 开关；第三方 GLM/Kimi/DeepSeek 多为 Chat | provider responses + 模型级 endpoint_type=chat_completions @243+ | 对齐设计 | 已核实 |
| 价格 CNY | 第三方汇总：Lite 原价¥40 / Pro¥200；活动首两月可 ¥9.9/¥49.9 至约 2026-11-08（以结算页为准） | 无价格字段 | 活动价易变；以控制台为准 | 推断（活动页多源）；结构 已核实于二手汇总 |
| thinking_type_switch | Doubao 需 thinking.type + effort；off 字面量 disabled | quirks @75；tiers off=disabled @210 | **仍需要**；单测覆盖 enabled/disabled | 已核实 |
| store | Doubao 常 store:false | extra_body store=false @211 等 | 对齐 | 已核实 |
| 模型清单 | 活动页含 Doubao-Seed-2.1、GLM、Kimi、DeepSeek、Auto 等（动态） | 7 个模型条目 | 需对照控制台「开通管理」是否缺 MiniMax/Auto | 推断 |
| 实测 | – | – | **未测** | – |

### 2.2 ark-agent（Agent Plan）

| 项 | 官方 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点 | `/api/plan/v3` | @118 | 对齐 | 已核实 |
| 计价 | AFP 积分；Small/Medium 活动价曾 ¥9.9/¥49.9；档位含多模态/Harness | 无价格 | 以活动页/控制台为准 | 已核实（产品存在） |
| 模型 | 官宣 GLM-5.3、Doubao-Seed-Evolving、Kimi-K3(部分)、DeepSeek-v4-flash 等 | 含 evolving@659、minimax-m3@730、第三方 chat 清 quirks @686+ | 大体覆盖；注意 chat 模型 quirks=[] 清掉 thinking_type_switch——正确 | 已核实 |
| thinking_type_switch | 仅 Doubao Responses 需要 | provider quirk + chat 模型 quirks=[] | **仍需要**（Doubao）；chat 清除正确 | 已核实 |
| 实测 | – | – | **未测** | – |

### 2.3 tencent-token（TokenHub Token Plan）

| 项 | 官方（2026-09-30 文档） | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点 | OpenAI：`https://api.lkeap.cloud.tencent.com/plan/v3`；Anthropic：`…/plan/anthropic` | @129 chat_completions | 对齐 OpenAI 路径；未接 Anthropic | 已核实 | [文档](https://cloud.tencent.com/document/product/1823/130060) |
| 通用价 | Lite¥39/780积分；Standard¥99/1980；Pro¥299/5980；Max¥599/11980（订阅月） | 无 | – | 已核实 |
| Hy 价 | Lite¥28 … Max¥468；模型 Hy3/Hy4 preview | 目录有 hy4-preview@783，无 hy3，无独立 Hy provider | 缺 Hy3；Hy 与通用共用 Key | 已核实 |
| 模型 | Auto(tc-code-latest)、deepseek/deepseek-flash、GLM-5.3/Flash、Kimi K3、MiniMax-M2.7/M3、MiMo-V2.6-Flash、Hy4；GLM-5/5.1 将于 2026-10-09 下线 | 有 glm/deepseek-flash/minimax-m2.7/hy4/auto/tc-code-latest；**无** kimi-k3、minimax-m3、mimo-v2.6-flash | **建议新增**官方已上架模型；勿再依赖即将下线的 GLM-5/5.1 | 已核实 |
| 限制 | 禁止非工具场景裸 API 滥用；并发随档位；不退款 | – | 产品合规提示 | 已核实 |
| 实测 | – | – | **未测** | – |

### 2.4 aliyun-token（百炼 Token Plan）

| 项 | 官方 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点 | 仅北京；团队版示例 `https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`；Key `sk-sp-` | @138 responses | 对齐 | 已核实 | [概述](https://help.aliyun.com/zh/model-studio/token-plan-overview) |
| 个人价 CNY | Lite 原价60限时39；Essential 120→79；Standard 180→139；Pro 600→499；用量包 100元/2万 Credits | 无 | – | 已核实 |
| 团队价 | 标准座席198→150；高级698→550；尊享1398；共享包5000 | 无 | – | 已核实 |
| 协议 | compatible-mode；目录注释 reasoning.effort + replay=summary（百炼） | endpoint_type responses；reasoning.replay=summary @821 | 对齐 Bailian summary 回放 | 已核实 |
| 模型 | 文本/图像/视频/语音等多模态（完整列表另页） | qwen3.8-max/flash、deepseek-v4.1-flash、glm-5.3 | 覆盖主力文本；多模态生成未建独立条目（注释称图像理解） | 已核实 |
| Coding Plan | 独立产品，停新购/迁移；推荐 Token Plan | 未建 Coding Plan provider | **不做**回退 Coding Plan | 已核实 |
| 实测 | – | – | **未测** | – |

## 3. Q2 / Q3

| provider | 可升级 | 同家新模型 | 建议 |
|---|---|---|---|
| ark-coding | 核对 Auto/MiniMax 是否应进目录 | Seed 系列已较新 | 维持 thinking_type_switch；补控制台缺失模型 |
| ark-agent | Harness/多模态若产品要接需 Runtime | evolving / M3 已在 | 维持 |
| tencent-token | 补 kimi-k3、minimax-m3、mimo-v2.6-flash；可选 hy3 | 同上 | **升级目录** |
| aliyun-token | 按官方模型列表扩视觉/视频（若产品要） | qwen3.8 已新 | 按需 |

## 4. thinking_type_switch 结论

**仍需要**（已核实）：Doubao Responses 方言在 `responses.rs:131-140` 明确分支；catalog ark-coding/ark-agent 保留 quirk；chat 第三方模型用 `quirks = []` 清除（ark-agent @686+）。关闭该 quirk 会导致 Doubao off/effort 编码错误。

## 5. 建议汇总

| 动作 | 项 |
|---|---|
| 升级 | tencent-token 模型列表对齐 2026-09-30 文档 |
| 升级 | 三家价格/限额在设置或文档侧链到官方页（目录不存价） |
| 不做 | 删除 thinking_type_switch |
| 不做 | 复活阿里云 Coding Plan |
| 未测 | 全部 live 调用 |

## 6. 待裁定

- Agent Plan vs Coding Plan 是否在 UI 默认推荐 Agent。
- 腾讯「禁止裸 API」条款 vs LiteCode 作为编码工具的合规表述。
