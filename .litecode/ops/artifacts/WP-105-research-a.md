# WP-105-research-a · WP-107 一方直连 + codec 对照

- 包：WP-107（owner=provider）；父 WP-106；查阅日期：**2026-10-13 HKT**
- 规则：只读；无 key → 全部实测标 **未测**；证据标签：`已核实` / `推断`
- 目录：`src/provider_catalog/default_catalog.toml`（下称 catalog）

## G2 总表（本包范围内）

| provider | 大陆能否直连 | 能否开发票 | 标签 | 证据 |
|---|---|---|---|---|
| openai | 否（需跨境/代理，api.openai.com 在大陆不可稳定直连） | 否（对中国大陆个人/小微无常规增值税发票通道；企业合同另议） | 推断 | [OpenAI API](https://platform.openai.com/)；大陆网络通行常识 |
| deepseek | 是 | 是（开放平台充值可申请发票） | 已核实（端点/模型）；推断（开票流程） | [DeepSeek Docs](https://api-docs.deepseek.com/quick_start/pricing) |
| mimo | 是 | 推断可（小米开放平台企业认证后开票，未逐条核对开票页） | 已核实（端点）；推断（发票） | [MiMo Docs](https://mimo.mi.com/docs/quick-start/first-api-call) |
| zhipu | 是 | 是（开放平台发票申请页，Coding Plan FAQ 明示） | 已核实 | [docs.bigmodel.cn](https://docs.bigmodel.cn/cn/api/introduction)；[Coding Plan FAQ](https://www.bigmodel.cn/glm-coding) |

## 1. 仓库盘点（file:line）

| provider | catalog 行 | endpoint | endpoint_type | quirks | 模型 (id @ 行) |
|---|---|---|---|---|---|
| openai | 42–46 | https://api.openai.com/v1 | responses | – | gpt-5.6-sol @149–159 |
| deepseek | 49–56 | https://api.deepseek.com | responses | reasoning_replay | deepseek-flash @164–172 |
| mimo | 59–65 | https://api.xiaomimimo.com/v1 | responses | reasoning_replay | mimo-v2.6-pro @177–187；mimo-v2.6-flash @190–198 |
| zhipu | 106–110 | https://open.bigmodel.cn/api/paas/v4 | chat_completions | – | glm-5.3 @615–622；glm-5.3-flash @625–632 |

Codec 分派：`src/llm/codec/mod.rs:31-33`。

## 2. Q1 差异（官方 vs 仓库）

### 2.1 openai

| 项 | 官方现状 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 协议 | 推荐 Responses；Chat Completions 仍支持但推理/工具更弱 | responses @45；模型 gpt-5.6-sol | 对齐推荐路径 | 已核实 | [Reasoning guide](https://developers.openai.com/api/docs/guides/reasoning) |
| 模型 | 文档示例 gpt-6-astra / gpt-5.6-* / gpt-6.1-sol 等；effort 含 none/low/medium/high/xhigh/max | 仅 gpt-5.6-sol；tiers off=none, low, medium, high→max；usage_patch=map_max_effort_to_xhigh @156–157 | 目录只挂 1 个 Sol；缺 Astra/Luna/Terra/6.1；high→max 与官方「max 为更高档」并存靠 patch 映 xhigh | 已核实 |
| encrypted_content | store:false/ZDR 时 reasoning 项默带 encrypted_content；include=["reasoning.encrypted_content"] 仍兼容 | extra_body include @159 | 仍显式 include：兼容旧行为，非必需（官方已默认） | 已核实 |
| reasoning.summary | 需 opt-in summary=auto/concise/detailed | summary=auto @157 | 对齐 | 已核实 |
| 上下文 | 模型页给窗口；guide 建议预留 ≥25k reasoning | context_window=256000, max=1050000, max_output=128000 @152–154 | 需对照具体 snapshot 页复核绝对值 | 推断 |
| 实测 | – | – | **未测** | – |

### 2.2 deepseek

| 项 | 官方现状 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点/模型 | base https://api.deepseek.com；model `deepseek-flash`（legacy v4-flash 名仍接受，落到 V4.1 Flash） | endpoint @51；id deepseek-flash @164 | **对齐** | 已核实 | [Responses](https://api-docs.deepseek.com/guides/responses_api) [Pricing](https://api-docs.deepseek.com/quick_start/pricing) |
| reasoning | effort 支持；summary 接受但不生成摘要；store 恒 false；**include 不支持**；reasoning 输入支持明文 content，**不支持 encrypted_content** | quirks reasoning_replay @56；无 summary；tiers off=none,low,medium→high,high→max @172 | 用明文回放而非 ciphertext——与官方「无 encrypted」一致；catalog 无 summary——与「不生成 summary」一致 | 已核实 |
| 流事件 | 含 `response.reasoning_text.delta/done`（明文 CoT）及标准 output_item / function_call 事件 | responses codec 解析标准事件 + keepalive 丢弃 `responses.rs:174-175` | 需确认 codec 是否消费 reasoning_text.*（若只认 summary 路径会丢明文思考） | 推断 |
| 价格 USD/1M | Flash：cache-hit 0.003/0.006；cache-miss 0.15/0.3；out 0.6/1.2（off/peak） | 目录无价格字段 | 建议 UI/文档引用官方价；可考虑挂 deepseek-v4-pro | 已核实 |
| 实测 | – | – | **未测** | – |

### 2.3 mimo

| 项 | 官方现状 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点 | OpenAI 兼容 `https://api.xiaomimimo.com/v1`；文档主推 **Chat Completions** + `thinking.type` + `reasoning_content` 回传 | endpoint_type=**responses** @62；quirks reasoning_replay @65 | **协议偏好冲突**：官方深思/多轮工具文档以 chat+reasoning_content 为主；仓库走 Responses。若官方 Responses 不完整，可能不稳定 | 推断（文档侧已核实 chat；Responses 覆盖未在官方深思页确认） | [Deep Thinking](https://mimo.mi.com/docs/en-US/quick-start/usage-guide/text-generation/deep-thinking) |
| 模型 | mimo-v2.6-pro / flash 等 | 同 id @177/@190 | 对齐命名 | 已核实 |
| 思考控制 | thinking.type enabled/disabled；部分模型默认 enabled | tiers none/low/medium/high @187 | catalog 用 reasoning.effort 方言（Responses）；与 chat 的 thinking 对象不同——依赖官方 Responses 是否接受 effort | 推断 |
| modalities | 文档支持多模态（版本相关） | text/image/video/audio @183 | 偏乐观；需官方模型卡核对 video/audio | 推断 |
| 实测 | – | – | **未测** | – |

### 2.4 zhipu

| 项 | 官方现状 | 仓库 | 差异 | 标签 |
|---|---|---|---|---|
| 端点 | https://open.bigmodel.cn/api/paas/v4/chat/completions | base @108 + chat_completions @109 | 对齐 | 已核实 | [HTTP 引入](https://docs.bigmodel.cn/cn/guide/develop/http/introduction) |
| 模型 | 文档示例 glm-5.3；另有 Coding Plan 独立端点 `/api/coding/paas/v4` | glm-5.3 / glm-5.3-flash @615/@625；**未**接 Coding Plan 端点 | 标准按量端点 OK；Coding Plan 套餐未建独立 provider | 已核实 |
| reasoning | GLM-5.3 拒 disabled；effort low/high/max（catalog 注释 @612） | tiers low/medium→high/high→max，无 off @622 | 对齐注释约束 | 已核实（仓库注释）；官方 effort 枚举页需再核对 | 推断（完整枚举） |
| 实测 | – | – | **未测** | – |

## 3. Codec 逐项对照

### 3.1 Responses codec ↔ OpenAI Responses / 方言

对照实现：`src/llm/codec/responses.rs`、`responses_harden.rs`；分派 `mod.rs:31-33`。

| 规范项 | 官方 (OpenAI / DeepSeek 注) | 仓库实现 | 结论 | 标签 |
|---|---|---|---|---|
| 流事件类型 | response.created / in_progress / output_item.* / content_part.* / output_text.* / function_call_arguments.* / completed|incomplete|failed；DeepSeek 另有 **reasoning_text.delta/done** | `parse_stream_event` @165+；丢弃 keepalive @174-175；harden 后再反序列化 | 支持权威枚举；keepalive 正确忽略；DeepSeek 明文 reasoning_text 是否映射进 Reasoning item：**待 Runtime 确认** | 已核实（keepalive）；推断（reasoning_text 映射） |
| tool / function_call | tools[].type=function；流式 arguments delta；input 回传 function_call + function_call_output | 编码 tools @109-119；测试断言 input function_call @1204 | 对齐 function 工具 | 已核实 |
| usage / cached_tokens | usage.input_tokens_details.cached_tokens；output_tokens_details.reasoning_tokens | harden + usage_patch（map_max_effort_to_xhigh / fill_empty_token_details） | patch 处理方言；cached 依赖上游字段 | 已核实（代码）；实测 **未测** |
| reasoning item | OpenAI：summary + 可选 encrypted_content；DeepSeek：明文 content，无 encrypted | serialize / replay：encrypted 路径 @214,244；ciphertext vs summary replay（catalog reasoning.replay） | OpenAI 路径对齐；DeepSeek 依赖 reasoning_replay quirk 而非 include | 已核实 |
| prompt cache | OpenAI prompt_cache_key / retention；DeepSeek **不支持**这些参数（自动缓存） | 未见 catalog/codec 写入 prompt_cache_key | OpenAI 可选增强未接；DeepSeek 无需 | 已核实 |
| thinking_type_switch | Doubao：thinking.type + reasoning.effort | quirk 分支 @131-140：off 时 thinking.type=effort 字面量；on 时 type=enabled + reasoning | **仍需要**（见 WP-108） | 已核实 |
| include encrypted | 官方 store:false 默认带密文；include 遗留 | openai 模型 extra_body include @159 | 冗余但无害 | 已核实 |

### 3.2 Chat Completions codec ↔ OpenAI Chat Completions / 方言

对照：`chat.rs`、`chat_synth.rs`、`chat_usage.rs`。

| 规范项 | 官方 / 方言 | 仓库实现 | 结论 | 标签 |
|---|---|---|---|---|
| stream_options.include_usage | OpenAI 流式末包 usage 需 opt-in | `stream_options.include_usage=true` @chat.rs:157-158（model.stream_usage） | 对齐 | 已核实 |
| delta.tool_calls | index/id/function.name/arguments 增量 | chat_synth 吞 tool_calls @281+；flush 写回 messages @481-489 | 对齐 | 已核实 |
| reasoning_content 方言 | 智谱/DeepSeek-chat/MiMo 等用 reasoning_content；亦有 reasoning / reasoning.content | `chat_reasoning_text` 顺序读三键 @chat_synth.rs:20-35 | 覆盖主流方言 | 已核实 |
| usage 映射 | prompt_tokens / completion_tokens / cached / reasoning_tokens 别名多 | `chat_usage_to_responses` @chat_usage.rs:12-31 含 prompt_cache_hit_tokens | 对齐多别名 | 已核实 |
| reasoning_effort | Chat 侧 effort 字段 | body["reasoning_effort"] @chat.rs:160-161 | 对齐 OpenAI/兼容网关；MiMo 官方 chat 更常用 thinking 对象——若走 chat 需确认 | 已核实（代码）；推断（MiMo） |
| 中缝 content/reasoning | 部分厂商思考截断后 spill 到 content | seam 检测 @chat.rs:425+、chat_synth forensics | 防御性处理存在 | 已核实 |

## 4. Q2 可升级能力

| provider | 能力 | 建议 | 标签 |
|---|---|---|---|
| openai | 多 snapshot（Astra/Luna/6.1 Sol）、reasoning.mode=pro、reasoning.context=all_turns、configuration_update | **升级目录模型列表**；pro/context 需 Runtime 字段才可表达 | 已核实（官方有）；推断（产品优先级） |
| deepseek | 已 Responses；可加 v4-pro；明文 reasoning 流 | **可选新增** deepseek-v4-pro；确认 reasoning_text 事件 | 已核实 |
| mimo | 官方深思/工具回传成熟在 chat | **评估**改 chat_completions 或双 endpoint_type；补 thinking 开关语义 | 推断 |
| zhipu | Coding Plan 独立 base | **可选新增** zhipu-coding provider；非必须 | 已核实 |

## 5. Q3 同家新模型

| provider | 候选 | 动作 | 标签 |
|---|---|---|---|
| openai | gpt-6-astra, gpt-6.1-sol, gpt-5.6-luna/terra | 建议新增（性价比 Luna；旗舰 Astra） | 已核实（文档列出） |
| deepseek | deepseek-v4-pro；flash 已在目录 | 建议新增 pro；flash 保持 | 已核实 |
| mimo | ultraspeed 等变体 | 低优先 | 推断 |
| zhipu | glm-5.3-flash 已在；关注 Coding Plan 模型表 | 维持；视套餐再拆 | 已核实 |

## 6. 建议汇总

| 动作 | 项 | 理由 |
|---|---|---|
| 升级 | openai 模型矩阵 + 可选 reasoning.mode/context（Runtime） | 官方主推能力已超目录单模型 |
| 升级 | 核实 MiMo Responses 是否等价 chat 深思；否则改协议 | 避免 400/质量回退 |
| 新增 | deepseek-v4-pro | 官方在售 |
| 不做 | 为 DeepSeek 加 encrypted include | 官方明确不支持 |
| 不做（本包） | 改 codec 公共框架 | 归 Runtime |

## 7. 待裁定

- MiMo 是否强制改 chat_completions。
- OpenAI 是否引入 gpt-6-astra / Luna 默认档。
