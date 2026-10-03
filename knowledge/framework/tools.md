```node
node : tools
status : enabled
summary : tools：统一合约 / 门闸 / 出口；权限 Allow·Ask·Deny 单向收紧。
x : 522
y : 350
w : 380
h : 481
```

- 内置 / Custom / MCP 同合约；并发安全且资源不冲突才并行，其余按批串行。
- 未定义／未绑定的工具不可见；配置 readiness 不等于运行 Warm，执行另有门闸；floor 不可放宽、grant 只软化 Ask。
- tools 的输出以 markdown 语法为硬性要求。
- 信号克制：Error / Warning / Hint 是稳定语法；Hint 只留给 LSP 成功反馈，不手写前缀、不造第二信号。
- tool的使用场景和哲学应当在描述中自解释，system prompt尽量保持人格和工作方式上的设定
- 描述与参数要精准、聚焦、无冗余；内部实现不得污染描述——agent 拿到结果，自然知晓实现。
- tool 调用应做到自动化、美观、信息密度大。
  - 自动化：减少agent决策成本，有些事情帮agent直接做掉，并提供说明，例如自适应展开和压缩的检索工具。
  - 美观：保持格式稳定，结构清晰，统计聚合信息放开头，中间内容合理排序和聚合，需要的提醒按情况出现在尾部，提供下一步指引
  - 信息密度：md 语法优先，前端易于渲染；人类也能通过渲染的美观度判断信息密度——美观就是信息

契约展开：[@ key="tool-pipeline"]。
上层：[@ key="agent-design"]。
