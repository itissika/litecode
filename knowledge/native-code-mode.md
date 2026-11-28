```node
node : native-code-mode
status : enabled
summary : code 模式是唯一原生接口：同模型、同题、同档思考强度，用更低的成本换更高的成功率；证明只靠 tracing 和评测，不靠感觉。
x : 18
y : 410
w : 272
h : 300
domain : harness-core
```

**一句话：** 模型只写代码，工具是代码里的函数。没有工具卡，没有第二套调用形态。

**钉死的四样**（不钉死，数字就不能比）

1. 一个 `run_code` 面。
2. 一个模型。模型不一样，比的就不是同一件事。
3. 一套题。
4. 同一个思考档位，两档都要看：默认档和最大档。钉的是这两档的意思——模型推荐的那一档、模型能到的最大档，不是厂商的具体参数值。LiteCode 的 medium 一般就落在推荐档，真漏了对不上，数据自己会逼出来，到时候再说。codex、pi 已经钉住且有测试守着；opencode 没有档位开关，只能靠请求参数，最大档还没验过。档位没对齐的批次标成「档位未对齐」，只作趋势，不作结论。

**松口的：供应商。** 按账单随便切，横评里不要求对等。代价是成本不按各家真实计价——按同一张价目表、拿 token 重算；非官方 provider 的缓存命中不可信，按全未缓存算，宁可低估自己。

**自己跟自己比。** 横评只用来定位，日常更多是 LiteCode 跟自己比。所以 LiteCode 的可配面注定比别家宽：工具集可配、供应商和模型可切、提示词可改、思考强度可调、上下文可压。这些都是它自己的能力，评测只是拿它们做控制变量，不是另开旁路。这不是作弊——别家的成绩冻着不动，我们每次只动一个旋钮，看哪一格变好，这就是控制变量，用来找突破口。

**口径的边界**（各管一层，不打架）

- 旋钮是 LiteCode 自己的，不是评测专用的旁路。对话里只暴露两个意图：想多久、上下文多大；供应商写法只在模型目录里对应 [platform-knobs](knowledge/contracts/config/platform-knobs.md)，能切的也只是目录里有的 [provider-catalog](knowledge/framework/provider-catalog.md)。工具集、提示词、模型在配置里，不在对话里。
- 一次会话内不换。供应商、密钥、模型定一次，压缩原样复用 [turn-llm-binding](knowledge/contracts/agent/turn-llm-binding.md)。要换就换批次。
- 上下文是模式，不是阈值。80% 准备、100% 硬线不动 [context-budget](knowledge/contracts/context/context-budget.md)。
- 少而好，不是不许造工具。默认面只许一个 `run_code`；用户自造的 custom tool 不进默认面 [featured-tools](knowledge/framework/featured-tools.md)。

**要什么**（决定性的只有一句：更少成本，更高成功率）

- 成功率比别家高。高多少还没定，先按「不低于别家」判定——差 0 个百分点也算达标。数字以后再收。
- 总 token 比别家低 40%。成本从它派生，这样供应商差异污染不到结论。
- 每题成本比别家低 40%，按同一张价目表重算。受缓存波动影响，只当参考。
- 步数和时间是参考项：更少更快更好，不行也不致命。

**怎么算数**（不然上面几条永远判不了）

- 基线：钉一个 build、一套题、每题 3 trial。对手冻结在一批 run 上，模型、题集、档位一致即可；只有缺新题的数据才补跑对手。
- 判定：成功率看 pass@1 的多数；主指标是逐题 token 中位数；成本只作参考。
- 取舍：通过率掉一题，省多少 token 都不算赢。
- 落点：挖完 tracing 的结论写在 tracing 数据旁边，跟着 run 走，连同「下次预测哪一格会变」一起写；不进本仓提交。

**怎么做到**：挖 tracing。自己的和同题别家的都挖——别家的成绩是冻结的，正好当参照，看它为什么更省、更快、更少绕路。

挖自己只挖两处。

- **tool**：接口设计得好不好，出入参有没有冗余，有没有多余的工具；信号给没给、一致不一致、好不好懂、措辞准不准。有时候砍一个字段、改几个词就立竿见影，这里没有工程魔法——这是面向用户的设计，每个词都得琢磨。
- **ctx**：系统提示词和 reminder。一句话改完就见效。reminder 尤其要看：什么情况下给了什么信息。

**现在做到哪**：没有硬统计，只有一大堆 tracing。这是常态——每改一次就跑一次评测、挖一次过程。别家的成绩和 tracing 是冻结的，LiteCode 的是活的。只有确认每次改动真的变好了一点（tracing 里看得见模型的思考过程，摩擦和困惑在减少，而不是多出新的困惑），再大跑一波，钉死基线。

**工作流**：优化要可追溯，评测用最新，出问题回退定位。动了 tool 或 ctx 的提交必须带 `Eval:` 指针，指不到证据不算数——`.githooks/commit-msg` 会拦。

**评测管线**：统一入口 `eval/run.py` 与共享聚合层 `eval/core/` 驱动两条管线：product（`eval/usability/` 场景自检与基线门禁）与 panel（Harbor 跑 11 题横评、`egress` 记竞品 usage、LiteCode 直连记 CLI usage、`collect` 汇总出 6 节报告与 tracing）。总 token 和每题成本都要看，单题能占七成。

**依据（每条钉死的东西落在哪）**

- 一个 `run_code` 面：`builtin_code_surface_tools` 是唯一装配点，测试断言 provider 拿到的 tools 只有 `run_code` [builtin_code_surface_tools](src/tool/registry.rs#builtin_code_surface_tools)。
- 思考档位：`ThinkingTier` 是唯一枚举，厂商写法只在目录里映射 [ThinkingTier](src/platform_knobs.rs#ThinkingTier)。
- 压缩预算线：`COMPACTION_BUDGET_PERCENT` 是唯一常量，三处都读它——会话压缩触发线、压缩后历史预算、摘要请求自身的输入上限 [compaction_budget](src/session/estimate.rs#compaction_budget)。
- 模型绑定：一次会话一套 provider + 模型，压缩原样复用 [TurnLlmBinding](src/runtime/llm_resolve.rs#TurnLlmBinding)。
- 统一入口、题集/价目表与报告渲染各一处：[run.py](eval/run.py)、[panel-v4.1-flash.yaml](eval/harbor/bench5/campaigns/panel-v4.1-flash.yaml)、[report.py](eval/core/report.py)。

父节点：[product-brief](knowledge/product-brief.md)。

相关：[agent-design](knowledge/design/agent-design.md)、[engine-design](knowledge/design/engine-design.md)、[featured-tools](knowledge/framework/featured-tools.md)。