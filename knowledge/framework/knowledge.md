```node
node : knowledge
status : enabled
summary : 知识框架：关系可验证；磁盘为引用权威；后端索引为唯一真相。
x : 1728
y : 504
w : 480
h : 398
domain : knowledge
```

**准入：** A3。知识库的价值在稳定、可验证的关系（节点↔节点、正文↔文件/符号），而不是散文；引用是否成立只看磁盘物理状态，结论只认后端索引，前端只消费快照。坏了则 turn 提醒、knowledge 工具与 API 会对同一仓库给出不同答案。

依据：契约测试 [spec_tests.rs](src/knowledge/spec_tests.rs)、用例表 [cases.json](tests/fixtures/knowledge_refs/cases.json)；实现根 [mod.rs](src/knowledge/mod.rs)、[knowledge-client-view](knowledge/contracts/client/knowledge-client-view.md)。
上层：[product-brief](knowledge/product-brief.md)。
相关：[featured-tools](knowledge/framework/featured-tools.md)。
契约展开：[knowledge-ref-authority](knowledge/contracts/knowledge/knowledge-ref-authority.md)、[knowledge-parse](knowledge/contracts/knowledge/knowledge-parse.md)、[knowledge-index](knowledge/contracts/knowledge/knowledge-index.md)、[knowledge-ref-resolve](knowledge/contracts/knowledge/knowledge-ref-resolve.md)、[knowledge-tool](knowledge/contracts/knowledge/knowledge-tool.md)。
