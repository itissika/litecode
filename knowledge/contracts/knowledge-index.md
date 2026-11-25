```node
node : knowledge-index
status : pending
summary : 常驻索引：每工作区一份；快照可订阅；多入口读同一 Content。
x : 5364
y : 1170
domain : knowledge
```

**准入：** A3。`KnowledgeIndex` 按工作区注册常驻引擎；`snapshot` / `fresh_content` / `turn_note` / `query_symbols` 都读同一套解析结果，不允许入口各算各的。索引是派生物，可重建；真源仍是磁盘上的 knowledge/ 与被引用文件。

依据：[start](src/knowledge/index.rs#start)、[KnowledgeIndex.snapshot](src/knowledge/index.rs#KnowledgeIndex.snapshot)、[fresh_content](src/knowledge/index.rs#fresh_content)、[turn_note](src/knowledge/index.rs#turn_note)。
上层：[knowledge](knowledge/framework/knowledge.md)。
相关：[knowledge-ref-authority](knowledge/contracts/knowledge-ref-authority.md)。
