```node
node : knowledge-parse
status : pending
summary : 节点解析：声明块定键与 status；文件名主干必须等于键。
x : 4500
y : 558
domain : knowledge
```

**准入：** A1。`parse_knowledge_markdown` 从 node 声明围栏读出键/status/画布字段；键不满足规则或 status 非法要报声明错误；文件名主干 ≠ 键报 filename_mismatch。解析错了，后面的引用与索引都会建在错误身份上。

依据：[@ file="src/knowledge/document.rs" symbol="fn parse_knowledge_markdown"]、[@ file="src/knowledge/document.rs" symbol="enum Status"]、[@ file="src/knowledge/corpus.rs" symbol="fn nodes_from_parsed"]、[@ file="src/knowledge/spec_tests.rs"]。
上层：[@ key="knowledge"]。
相关：[@ key="knowledge-ref-authority"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 声明解析与 Status —— [@ file="src/knowledge/document.rs" symbol="fn parse_knowledge_markdown"]、[@ file="src/knowledge/document.rs" symbol="enum Status"]。
- 布局契约测试 —— [@ file="src/knowledge/spec_tests.rs"]。
