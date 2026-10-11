```node
node : knowledge-tool
status : pending
domain : knowledge
summary : knowledge 工具：读走常驻索引；写落盘后再刷新；不自造第二套解析。
```

**准入：** A4。agent 面的 `knowledge` 工具（board/list/refs/check/create/rename）必须消费同一索引与同一状态码，写操作改磁盘后通知刷新，不得在工具内另写一套引用判定。它是知识契约对 agent 的出口，也是 featured-tools 里「关系可验证」的落点。

依据：[@ file="src/tools/knowledge.rs" symbol="impl Tool for KnowledgeTool › fn execute"]、[@ file="src/tools/knowledge.rs" symbol="impl KnowledgeTool › fn check"]、[@ file="src/tools/knowledge.rs" symbol="impl KnowledgeTool › fn refs"]、[@ file="src/knowledge/index.rs" symbol="impl KnowledgeIndex › fn notify_paths"]。
上层：[@ key="knowledge"]。
相关：[@ key="featured-tools"]、[@ key="knowledge-index"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 工具读索引、写后 notify —— [@ file="src/tools/knowledge.rs" symbol="impl Tool for KnowledgeTool › fn execute"]、[@ file="src/tools/knowledge.rs" symbol="impl KnowledgeTool › fn check"]、[@ file="src/knowledge/index.rs" symbol="impl KnowledgeIndex › fn notify_paths"]。
