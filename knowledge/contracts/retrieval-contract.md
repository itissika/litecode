```node
node : retrieval-contract
status : pending
summary : 检索契约：文件/会话为真源，索引可重建；命中按证据排序、水合验证，复制品不冒充原件。
x : 1224
y : 674
w : 398
h : 720
```

**检索召回已有事实的证据，不让索引或回声副本变成事实源。**

### 数据
- code corpus 来自工作区文件；session corpus 来自会话业务行。索引、chunk、向量及排名都是派生数据。
- code hit 以 path／行范围定位；session hit 以 session_id／seq 定位，排名键与用于展示的 score 分开。

### 流转
真源 → 语料准入／派生索引维护 → 查询过滤／词法与可用语义候选 → 按各自契约融合排序 → 真源水合 → 预算内证据视图。

### 不变量
- 不混同两种语料的融合／执行门；code 人类搜索的文本与语义列可分开，session 搜索有自己的融合排序。
- session agent 搜索硬排除活跃 session 当前 Surface 上的行；已被 compact 遮蔽的原始行仍可召回，不是把整个当前 session 排除。
- 回声通过同 session 的 call_id 关联判定：session_search 或读会话产生的 result 整条剔除，call 意图保留；不靠内容相似度猜“原件”。
- session 排序先看 RankKey 的匹配证据、强度与角色；近似同分时偏好对话，随后才是调用者会话家族、会话更新时间与后面的行。不能概括成“最近优先”。
- session 索引允许滞后，检索热路径不在当前查询前阻塞重建；返回前用真源水合并复核行身份与过滤。滞后可能少召回，不能使已删除或回声行成为有效证据；语义 preview 仍可能来自旧索引，水合不保证其文本新鲜度。
- 输出受 token／候选窗口预算约束；结果是召回视图，不承诺穷尽全部历史，也不称预算内召回为无损全量。

依据：[@ file="src/tools/session_search.rs" symbol="impl SessionSearchTool › fn search_in_workspace" label="活跃窗口与只读查询"]、[@ file="src/engines/session_search/mod.rs" symbol="fn sort_hits_for_agent" label="排序优先级"]、[@ file="src/engines/session_search/mod.rs" symbol="fn hydrate_hits" label="真源水合"]、[@ file="src/engines/session_search/echo.rs" symbol="fn result_keys_with" label="结构化回声剔除"]、[@ file="src/engines/code_search/facade.rs" label="人类 code 检索视图"]。
验证：[@ file="src/engines/session_search/echo.rs" label="回声隔离测试"]、[@ file="src/engines/session_search/ranking.rs" label="排名契约与测试"]。

上层：[@ id="featured-tools" label="特色工具"]、[@ id="engine-design" label="engine 设计"]。
