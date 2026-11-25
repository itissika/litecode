```node
node : knowledge-parse
status : pending
summary : 节点解析：声明块定键与 status；文件名主干必须等于键。
x : 4500
y : 558
domain : knowledge
```

**准入：** A1。`parse_knowledge_markdown` 从 node 声明围栏读出键/status/画布字段；正文引用统一走标准 Markdown 链接（`[label](path)` / `#L12-L30` / `#Store.save`），由 `extract_cites` 提取。键不满足规则、status 非法、或文件名主干 ≠ 键（`filename_mismatch`）均报 Error，坏节点不可被引用。解析错了，后面的引用与索引都会建在错误身份上。

依据：[parse_knowledge_markdown](src/knowledge/document.rs#parse_knowledge_markdown)、[Status](src/knowledge/document.rs#Status)、[nodes_from_parsed](src/knowledge/corpus.rs#nodes_from_parsed)、[spec_tests.rs](src/knowledge/spec_tests.rs)。
上层：[knowledge](knowledge/framework/knowledge.md)。
相关：[knowledge-ref-authority](knowledge/contracts/knowledge-ref-authority.md)。
