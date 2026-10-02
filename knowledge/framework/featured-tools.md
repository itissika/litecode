```node
node : featured-tools
status : enabled
summary : 特色工具：设计理念的落点——每条理念最终都长成一个工具的样子。
x : 1224
y : 352
w : 380
h : 517
```

特色工具集是理念的落点而不是出发点；litecode 设计 harness 的核心落点在此说明。

- tool：session search
  - 检索不给已有的：硬排除活跃 session 当前 Surface 行，不把窗口里已有的再吐回来；复制品不是原件，回声 result 整条剔除、call 意图保留；先按匹配证据与角色排序，同等排名再偏好调用者会话家族和最近更新。
  - 全仓 session 可以检索、能够过滤；会话在工作区持续积累，基于对话意图召回演进过程和思路，预算内召回不是全量历史。

- tool：knowledge
  - session 收敛后的下一步，session对话可能是讨论，推进，测试，验证落地的过程，人的思想收敛为知识，对人来说，到底要做什么，不做什么，会成为人类和agent共同的指引和方针；
  - 知识需要稳定，知识之间的引用关系必须保持稳定，且基于事实。知识库的核心在于关系，知识节点之间的关系，知识正文中与文件的关系，这些关系是机械式的，可验证的稳定关系，通过关系，能够知道，知识飘了没，知识该不该更新。
  - 对话中可以引用，自动展开内容给agent，方便引用知识节点，对人来说很重要，方便，才能够经常用；而对agent来说，知识本身很重要，没有知识的定义，agent实现起来就会谨慎，偏离了知识要求的轨道，agent则能够自省；

- tool：subagent（launch / send / wait / stop / list）
  - 由于底层完全共享同一套基础设施，所以primary agent和subagent session之间的关系就是人类和primary的关系，primary能够对subagent做大部分事情。
  - 刻意排除的操作，成本敏感：模型，上下文，思考强度；工作范围：agent类型；复杂度：revert回退会话；

- tool：litecode_workspace
  - 工作区门面，全部只读：status / sessions / guide / seed / refresh，永不写文件。
  - 一个工作区可能有多个session并行，session之间应该能够互相感知，而协作要求是给到人类的，人类应该做好工作安排

- tool：lsp
  - 人机共用的 LSP 引擎在 agent 面的入口；刻意收窄为四个动作：goToDefinition / findReferences / hover / diagnostics。
  - lsp启用的时候，agent无需刻意调用，甚至刻意从工具集中关掉，但只要lsp启动，write edit会自动诊断提供信息，类似于人类代码编辑时候的红色报错下划线，提醒原则是：不打扰，不过度，不强制；

- tool：code_search
  - 与 session search 共用同一检索引擎的另一语料（code）；索引该更新时自己开工，输出是 token 预算内的视图，帮助agent宽找代码范围。

- tool：plan
  - 工作区里的 markdown 计划（.litecode/plan/）：create / finish，从不删——人机共同读改，不是 agent 私有状态。

- tool：custom tool
  - custom tool 是重中之重：只有你才知道你需要什么，你需要的东西，你的agent需要的东西，请自己造，造出最合适的工具。

契约展开：[@ id="subagent-lifecycle" label="子会话生命周期"]、[@ id="retrieval-contract" label="检索真源与召回"]。
上层：[@ id="agent-design" label="agent 设计"]、[@ id="why" label="为什么"]。
