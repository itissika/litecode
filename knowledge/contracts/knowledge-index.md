```node
node : knowledge-index
status : pending
summary : 常驻索引：每工作区一份；快照可订阅；多入口读同一 Content。
x : 5364
y : 1170
domain : knowledge
```

**准入：** A3。`KnowledgeIndex` 按工作区注册常驻引擎；`snapshot` / `fresh_content` / `turn_note` / `query_symbols` 都读同一套解析结果，不允许入口各算各的。索引是派生物，可重建；真源仍是磁盘上的 knowledge/ 与被引用文件。

依据：[@ file="src/knowledge/index.rs" symbol="fn start"]、[@ file="src/knowledge/index.rs" symbol="impl KnowledgeIndex › fn snapshot"]、[@ file="src/knowledge/index.rs" symbol="fn fresh_content"]、[@ file="src/knowledge/index.rs" symbol="fn turn_note"]。
上层：[@ key="knowledge"]。
相关：[@ key="knowledge-ref-authority"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 常驻索引 start/snapshot/fresh_content/turn_note —— [@ file="src/knowledge/index.rs" symbol="fn start"]、[@ file="src/knowledge/index.rs" symbol="impl KnowledgeIndex › fn snapshot"]、[@ file="src/knowledge/index.rs" symbol="fn fresh_content"]、[@ file="src/knowledge/index.rs" symbol="fn turn_note"]。
