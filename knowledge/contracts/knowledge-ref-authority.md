```node
node : knowledge-ref-authority
status : pending
summary : 引用权威：磁盘物理状态为准；后端状态为唯一真相；前端不自判。
x : 5598
y : 414
domain : knowledge
```

**准入：** A1/A3。KR-INV-1..3：引用成立与否只看工作区磁盘真实内容；同一份磁盘状态下任意入口（提醒、工具、API、消息展开）必须同结论；前端只消费后端快照。这是知识域的总不变量，其它解析/索引节点都服从它。

依据：契约由 [spec_tests.rs](src/knowledge/spec_tests.rs) 与 [cases.json](tests/fixtures/knowledge_refs/cases.json) 锁定；索引实现见 [compute_once](src/knowledge/index.rs#compute_once)。
上层：[knowledge](knowledge/framework/knowledge.md)。
相关：[knowledge-client-view](knowledge/contracts/knowledge-client-view.md)。
