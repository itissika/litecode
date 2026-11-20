```node
node : knowledge-tool
status : pending
summary : knowledge 工具：读走常驻索引；写落盘后再刷新；不自造第二套解析。
x : 5094
y : 1170
domain : knowledge
```

**准入：** A4。agent 面的 `knowledge` 工具家族（`knowledge.list` / `knowledge.check` / `knowledge.refs` / `knowledge.create`，经 `run_code` `tools.knowledge.*` 调用）必须消费同一索引与同一状态码；其中 `list` 与 `check` 支持全部、单/多文件夹与单/多节点并集过滤，输出扁平标量行适配结构化表格渲染；写操作（`create`）改磁盘后通知刷新，不得在工具内另写一套引用判定。它是知识契约对 agent 的出口，也是 featured-tools 里「关系可验证」的落点。

依据：[@ file="src/tools/knowledge.rs" symbol="struct KnowledgeListTool"]、[@ file="src/tools/knowledge.rs" symbol="struct KnowledgeCheckTool"]、[@ file="src/tools/knowledge.rs" symbol="struct KnowledgeRefsTool"]、[@ file="src/tools/knowledge.rs" symbol="struct KnowledgeCreateTool"]、[@ file="src/knowledge/index.rs" symbol="impl KnowledgeIndex › fn notify_paths"]。
上层：[@ key="knowledge"]。
相关：[@ key="featured-tools"]、[@ key="knowledge-index"]。
